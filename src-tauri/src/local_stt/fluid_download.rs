//! Explicit, revision-pinned multi-file CoreML installs. Inference never
//! invokes this downloader. Every payload is authenticated before promotion.
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
    path: String,
    sha256: String,
    size: u64,
}

pub async fn download(
    app: &AppHandle,
    manifest: &super::model::LocalSttModelManifest,
    root: &Path,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let bundles: Vec<Bundle> = serde_json::from_str(include_str!("fluid_assets.json"))?;
    let bundle = bundles
        .into_iter()
        .find(|bundle| bundle.id == manifest.id)
        .ok_or_else(|| anyhow::anyhow!("unknown CoreML bundle"))?;
    let staging = manifest.extracting_path(root);
    if staging.exists() {
        tokio::fs::remove_dir_all(&staging).await?;
    }
    tokio::fs::create_dir_all(&staging).await?;
    let total: u64 = bundle.files.iter().map(|file| file.size).sum();
    let mut completed = 0;
    let mut progress = ProgressThrottle::default();
    let client = reqwest::Client::new();
    for file in &bundle.files {
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "model download cancelled");
        anyhow::ensure!(
            Path::new(&file.path)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid model artifact path"
        );
        let path = staging.join(&file.path);
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let mut output = tokio::fs::File::create(path).await?;
        let mut response = client
            .get(format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                bundle.repo, bundle.revision, file.path
            ))
            .send()
            .await?
            .error_for_status()?;
        let mut hash = Sha256::new();
        let mut received = 0;
        while let Some(chunk) = response.chunk().await? {
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
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let validation_path = staging.clone();
        let model_id = manifest.id.to_string();
        tokio::task::spawn_blocking(move || {
            super::fluid::FluidEngine::load(&validation_path, &model_id)
        })
        .await??;
    }
    tokio::fs::write(staging.join(".verenu-integrity"), bundle.revision).await?;
    promote_verified(&staging, &manifest.final_path(root))?;
    Ok(())
}

// Keep the old directory until promotion succeeds. A failed replacement
// restores it; only verified model artifacts replace an existing install.
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
    if previous {
        if cleanup(&backup).is_err() {
            log::warn!(
                "local-stt: verified CoreML install promoted; prior artifact cleanup failed"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
    let Ok(bundles) = serde_json::from_str::<Vec<Bundle>>(include_str!("fluid_assets.json")) else {
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
    let bundles: Vec<Bundle> = serde_json::from_str(include_str!("fluid_assets.json"))?;
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
    let bundles: Vec<Bundle> = serde_json::from_str(include_str!("fluid_assets.json"))?;
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
