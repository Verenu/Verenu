use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

#[derive(Default)]
struct ProgressThrottle(Option<std::time::Instant>);
impl ProgressThrottle {
    fn due(&mut self, now: std::time::Instant, complete: bool) -> bool {
        if complete
            || self.0.is_none_or(|last| {
                now.duration_since(last) >= std::time::Duration::from_millis(150)
            })
        {
            self.0 = Some(now);
            true
        } else {
            false
        }
    }
}

#[derive(Deserialize)]
struct Bundle {
    id: String,
    repo: String,
    revision: String,
    files: Vec<Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    /// Local file name, relative to the installed model folder.
    path: String,
    sha256: String,
    size: u64,
    /// Path inside the Hugging Face repo, when it differs from `path`.
    #[serde(default)]
    source: Option<String>,
    /// Repo and revision for this file, when it differs from the bundle.
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    revision: Option<String>,
}

/// Bundles for FluidAudio (CoreML) models and for speech models hosted on Hugging Face.
fn load_bundles() -> anyhow::Result<Vec<Bundle>> {
    let mut bundles: Vec<Bundle> = serde_json::from_str(include_str!("fluid_assets.json"))?;
    bundles.extend(serde_json::from_str::<Vec<Bundle>>(include_str!(
        "speech_assets.json"
    ))?);
    Ok(bundles)
}

/// True when a model is installed from pinned Hugging Face files instead of an archive.
pub fn has_bundle(id: &str) -> bool {
    load_bundles().is_ok_and(|bundles| bundles.iter().any(|bundle| bundle.id == id))
}

pub async fn download(
    app: &AppHandle,
    manifest: &super::model::LocalSttModelManifest,
    root: &Path,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let _consent = crate::legal_consent::network_permit().await?;
    let bundles = load_bundles()?;
    let bundle = bundles
        .into_iter()
        .find(|bundle| bundle.id == manifest.id)
        .ok_or_else(|| anyhow::anyhow!("unknown CoreML bundle"))?;
    let staging = manifest.extracting_path(root);
    let total = prepare_staging(&bundle, root, &staging, |root, total| {
        crate::api::model_download::ensure_disk_space(root, 0, Some(total))
    })
    .await?;
    let mut completed = 0;
    let mut progress = ProgressThrottle::default();
    let client = reqwest::Client::new();
    for file in &bundle.files {
        crate::legal_consent::require_local_download()?;
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "model download cancelled");
        anyhow::ensure!(
            Path::new(&file.path)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid model artifact path"
        );
        let source = file.source.as_deref().unwrap_or(&file.path);
        anyhow::ensure!(
            Path::new(source)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid model source path"
        );
        let repo = file.repo.as_deref().unwrap_or(&bundle.repo);
        let revision = file.revision.as_deref().unwrap_or(&bundle.revision);
        let path = staging.join(&file.path);
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let mut output = tokio::fs::File::create(path).await?;
        let mut response = cancellable_io(
            &cancel,
            client
                .get(format!("https://huggingface.co/{repo}/resolve/{revision}/{source}"))
                .send(),
        )
        .await?
        .error_for_status()?;
        let mut hash = Sha256::new();
        let mut received = 0;
        while let Some(chunk) = cancellable_io(&cancel, response.chunk()).await? {
            crate::legal_consent::require_local_download()?;
            anyhow::ensure!(!cancel.load(Ordering::Relaxed), "model download cancelled");
            received += chunk.len() as u64;
            anyhow::ensure!(
                received <= file.size,
                "model artifact exceeds expected size"
            );
            hash.update(&chunk);
            output.write_all(&chunk).await?;
            if progress.due(std::time::Instant::now(), false) {
                crate::api::model_download::emit_download_progress(
                    app,
                    "local-stt-model-download-progress",
                    manifest.id,
                    completed + received,
                    Some(total),
                );
            }
        }
        output.sync_all().await?;
        anyhow::ensure!(
            received == file.size && format!("{:x}", hash.finalize()) == file.sha256,
            "model artifact integrity check failed"
        );
        completed += received;
    }
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "model download cancelled");
    if progress.due(std::time::Instant::now(), true) {
        crate::api::model_download::emit_download_progress(
            app,
            "local-stt-model-download-progress",
            manifest.id,
            completed,
            Some(total),
        );
    }
    finish_validated_install(
        &staging,
        &manifest.final_path(root),
        &bundle.revision,
        &cancel,
        async {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            if manifest.engine_type == super::model::LocalSttEngineType::FluidAudio {
                let validation_path = staging.clone();
                let model_id = manifest.id.to_string();
                let validation_cancel = cancel.clone();
                tokio::task::spawn_blocking(move || {
                    super::fluid::FluidEngine::load_with_cancellation(
                        &validation_path,
                        &model_id,
                        &validation_cancel,
                    )
                })
                .await??;
            }
            Ok(())
        },
    )
    .await?;
    Ok(())
}

async fn cancellable_io<T>(
    cancel: &AtomicBool,
    operation: impl std::future::Future<Output = reqwest::Result<T>>,
) -> anyhow::Result<T> {
    crate::api::model_download::ensure_not_cancelled(cancel)?;
    tokio::select! {
        result = operation => Ok(result?),
        _ = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if cancel.load(Ordering::Acquire) || crate::legal_consent::require_local_download().is_err() {
                    break;
                }
            }
        } => anyhow::bail!("model download cancelled"),
    }
}

async fn finish_validated_install(
    staging: &Path,
    destination: &Path,
    revision: &str,
    cancel: &AtomicBool,
    validation: impl std::future::Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    validation.await?;
    crate::api::model_download::ensure_not_cancelled(cancel)?;
    tokio::fs::write(staging.join(".verenu-integrity"), revision).await?;
    crate::api::model_download::ensure_not_cancelled(cancel)?;
    promote_verified(staging, destination)
}

async fn prepare_staging(
    bundle: &Bundle,
    root: &Path,
    staging: &Path,
    ensure_space: impl FnOnce(&Path, u64) -> anyhow::Result<()>,
) -> anyhow::Result<u64> {
    let total = bundle.files.iter().try_fold(0u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| anyhow::anyhow!("model bundle size overflow"))
    })?;
    tokio::fs::create_dir_all(root).await?;
    ensure_space(root, total)?;
    if staging.exists() {
        tokio::fs::remove_dir_all(staging).await?;
    }
    tokio::fs::create_dir_all(staging).await?;
    Ok(total)
}

fn promote_verified(staging: &Path, destination: &Path) -> anyhow::Result<()> {
    promote_with_cleanup(staging, destination, |backup| {
        std::fs::remove_dir_all(backup)
    })
}

fn promote_with_cleanup(
    staging: &Path,
    destination: &Path,
    cleanup: impl FnOnce(&Path) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    let backup = destination.with_extension(format!("replaced-{}", uuid::Uuid::new_v4()));
    let previous = destination.exists();
    if previous {
        std::fs::rename(destination, &backup)?;
    }
    if let Err(error) = std::fs::rename(staging, destination) {
        if previous {
            std::fs::rename(&backup, destination)?;
        }
        return Err(error.into());
    }
    if previous && cleanup(&backup).is_err() {
        log::warn!(
            "local-stt: verified CoreML install promoted; prior artifact cleanup failed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn cancellation_interrupts_pending_network_io() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let pending = async {
            cancel.store(true, std::sync::atomic::Ordering::Release);
            std::future::pending::<reqwest::Result<()>>().await
        };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::cancellable_io(&cancel, pending),
        )
        .await
        .expect("cancelled network I/O must not remain active")
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn cancellation_during_validation_preserves_existing_install() {
        let root =
            std::env::temp_dir().join(format!("verenu-fluid-cancel-{}", uuid::Uuid::new_v4()));
        let staging = root.join("model.extracting");
        let destination = root.join("model");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(staging.join("new"), b"downloaded").unwrap();
        std::fs::write(destination.join("old"), b"installed").unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let error = super::finish_validated_install(
            &staging,
            &destination,
            "synthetic",
            &cancel,
            async {
                cancel.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(std::fs::read(destination.join("old")).unwrap(), b"installed");
        assert!(!destination.join("new").exists());
        assert!(!staging.join(".verenu-integrity").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn insufficient_bundle_space_leaves_staging_untouched() {
        let root = std::env::temp_dir().join(format!("verenu-fluid-space-{}", uuid::Uuid::new_v4()));
        let staging = root.join("model.extracting");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("existing"), b"retained").unwrap();
        let bundle = super::Bundle {
            id: "synthetic".into(),
            repo: String::new(),
            revision: String::new(),
            files: vec![
                super::Artifact { path: "synthetic".into(), sha256: String::new(), size: 40, source: None, repo: None, revision: None },
                super::Artifact { path: "second".into(), sha256: String::new(), size: 60, source: None, repo: None, revision: None },
            ],
        };
        let error = super::prepare_staging(&bundle, &root, &staging, |checked_root, total| {
            assert_eq!(checked_root, root);
            assert_eq!(total, 100);
            anyhow::bail!("synthetic insufficient disk space")
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("insufficient disk space"));
        assert_eq!(std::fs::read(staging.join("existing")).unwrap(), b"retained");
        assert!(!staging.join("synthetic").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_model_promotion_is_not_failed_by_backup_cleanup() {
        let root =
            std::env::temp_dir().join(format!("verenu-fluid-cleanup-{}", uuid::Uuid::new_v4()));
        let destination = root.join("model");
        let staging = root.join("verified");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(destination.join("old"), b"old").unwrap();
        std::fs::write(staging.join("new"), b"verified").unwrap();
        super::promote_with_cleanup(&staging, &destination, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap();
        assert_eq!(std::fs::read(destination.join("new")).unwrap(), b"verified");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn damaged_install_is_replaced_and_failed_promotion_restores_previous_files() {
        let root =
            std::env::temp_dir().join(format!("verenu-fluid-promote-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let destination = root.join("model");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("old"), b"existing artifact").unwrap();
        let staging = root.join("verified");
        assert!(super::promote_verified(&staging, &destination).is_err());
        assert_eq!(
            std::fs::read(destination.join("old")).unwrap(),
            b"existing artifact"
        );
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("new"), b"verified artifact").unwrap();
        super::promote_verified(&staging, &destination).unwrap();
        assert_eq!(
            std::fs::read(destination.join("new")).unwrap(),
            b"verified artifact"
        );
        assert!(!destination.join("old").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fluid_download_progress_stays_fractional_and_chunk_updates_are_bounded() {
        let payload =
            crate::api::model_download::DownloadProgress::new("fluid-parakeet-110m", 1, Some(100));
        let wire = serde_json::to_value(payload).unwrap();
        assert!((wire["progress"].as_f64().unwrap() * 100.0 - 1.0).abs() < 0.0001);
        let mut throttle = super::ProgressThrottle::default();
        let start = std::time::Instant::now();
        let emitted = (0..10_000)
            .filter(|chunk| {
                throttle.due(
                    start + std::time::Duration::from_micros(*chunk * 100),
                    false,
                )
            })
            .count();
        assert_eq!(emitted, 7);
        assert!(throttle.due(start + std::time::Duration::from_secs(1), true));
        assert_eq!(
            crate::api::model_download::DownloadProgress::new("fluid", 100, Some(100)).progress,
            1.0
        );
        assert_eq!(
            crate::api::model_download::DownloadProgress::new("fluid", 200, Some(100)).progress,
            1.0
        );
    }
}

pub fn installed(manifest: &super::model::LocalSttModelManifest, root: &Path) -> bool {
    let Ok(bundles) = load_bundles() else {
        return false;
    };
    let Some(bundle) = bundles.iter().find(|bundle| bundle.id == manifest.id) else {
        return false;
    };
    let path = manifest.final_path(root);
    std::fs::read_to_string(path.join(".verenu-integrity"))
        .ok()
        .as_deref()
        == Some(bundle.revision.as_str())
        && bundle.files.iter().all(|file| {
            std::fs::metadata(path.join(&file.path)).is_ok_and(|meta| meta.len() == file.size)
        })
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(super) fn verify_cached_integrity(
    id: &str,
    path: &Path,
    cache: &mut super::integrity_cache::IntegrityCache,
) -> anyhow::Result<()> {
    let bundles = load_bundles()?;
    let bundle = bundles
        .iter()
        .find(|bundle| bundle.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown CoreML bundle"))?;
    let mut paths = bundle
        .files
        .iter()
        .map(|file| path.join(&file.path))
        .collect::<Vec<_>>();
    paths.push(path.join(".verenu-integrity"));
    cache.verify(&paths, || verify_integrity(id, path))
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn verify_integrity(id: &str, path: &Path) -> anyhow::Result<()> {
    let bundles = load_bundles()?;
    let bundle = bundles
        .iter()
        .find(|bundle| bundle.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown CoreML bundle"))?;
    for file in &bundle.files {
        let mut input = std::fs::File::open(path.join(&file.path))?;
        let mut hash = Sha256::new();
        let length = std::io::copy(&mut input, &mut hash)?;
        anyhow::ensure!(
            length == file.size && format!("{:x}", hash.finalize()) == file.sha256,
            "CoreML model is corrupt; remove and download it again"
        );
    }
    Ok(())
}

#[cfg(test)]
mod speech_manifest_tests {
    use super::load_bundles;
    use crate::local_stt::model::{built_in_model_manifests, manifest_by_id, LocalSttEngineType};

    /// Speech models with no verified Hugging Face source. They stay listed but cannot be downloaded.
    const UNAVAILABLE: &[&str] = &["moonshine-tiny", "moonshine-small", "moonshine-medium", "cohere"];

    fn is_lower_hex(value: &str, len: usize) -> bool {
        value.len() == len && value.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    #[test]
    fn speech_bundles_are_pinned_files_under_their_manifest() {
        for bundle in load_bundles().expect("bundles parse") {
            if bundle.id.starts_with("fluid") {
                continue;
            }
            let manifest = manifest_by_id(&bundle.id)
                .unwrap_or_else(|| panic!("no manifest for bundle {}", bundle.id));
            assert!(manifest.url.is_none(), "{} must not download from a URL", bundle.id);
            assert!(is_lower_hex(&bundle.revision, 40), "{} needs a commit SHA", bundle.id);
            let mut seen = std::collections::HashSet::new();
            for file in &bundle.files {
                assert!(seen.insert(file.path.clone()), "duplicate file {}", file.path);
                assert!(
                    !file.path.contains('/') && file.path != "." && file.path != "..",
                    "{} must be a plain file name",
                    file.path
                );
                assert!(is_lower_hex(&file.sha256, 64), "{} needs a SHA-256", file.path);
                assert!(file.size > 0, "{} has no size", file.path);
                if let Some(revision) = &file.revision {
                    assert!(is_lower_hex(revision, 40), "{} needs a commit SHA", file.path);
                }
            }
        }
    }

    #[test]
    fn every_speech_model_without_a_url_is_bundled_or_listed_as_unavailable() {
        for manifest in built_in_model_manifests() {
            if manifest.url.is_some() || matches!(
                manifest.engine_type,
                LocalSttEngineType::FluidAudio | LocalSttEngineType::CtcBooster | LocalSttEngineType::AppleSpeech
            ) {
                continue;
            }
            assert!(
                super::has_bundle(manifest.id) || UNAVAILABLE.contains(&manifest.id),
                "{} has no download source",
                manifest.id
            );
        }
        for id in UNAVAILABLE {
            assert!(!super::has_bundle(id), "{id} should not have a bundle yet");
        }
    }
}
