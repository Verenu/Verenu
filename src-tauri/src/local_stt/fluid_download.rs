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
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

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
            let _ = app.emit("local-stt-model-download-progress", serde_json::json!({
                "model_id": manifest.id, "progress": (completed + received) as f64 / total as f64 * 100.0,
                "downloaded_bytes": completed + received, "total_bytes": total
            }));
        }
        output.sync_all().await?;
        anyhow::ensure!(
            received == file.size && format!("{:x}", hash.finalize()) == file.sha256,
            "model artifact integrity check failed"
        );
        completed += received;
    }
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "model download cancelled");
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
    tokio::fs::rename(staging, manifest.final_path(root)).await?;
    Ok(())
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
