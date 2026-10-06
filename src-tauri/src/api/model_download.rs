use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, serde::Serialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress: f32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelEvent {
    pub model_id: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct VerificationProgress {
    pub model_id: String,
    pub progress: f32,
}

pub fn emit_download_progress(
    app: &AppHandle,
    event: &str,
    model_id: &str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let progress = total_bytes
        .filter(|total| *total > 0)
        .map(|total| (downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let _ = app.emit(
        event,
        DownloadProgress {
            model_id: model_id.to_string(),
            downloaded_bytes,
            total_bytes,
            progress,
        },
    );
}

pub fn ensure_not_cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("download cancelled")
    }
    Ok(())
}

pub fn ensure_disk_space(root: &Path, downloaded: u64, total: Option<u64>) -> anyhow::Result<()> {
    if let Some(required) = total
        .map(|total| total.saturating_sub(downloaded))
        .filter(|n| *n > 0)
    {
        if let Ok(available) = crate::system::memory::free_bytes_for_path(root) {
            if available < required {
                anyhow::bail!(
                    "Not enough disk space to download model. Required: {} MB, Available: {} MB",
                    required / (1024 * 1024),
                    available / (1024 * 1024)
                );
            }
        }
    }
    Ok(())
}

/// Hashing runs on the caller's blocking worker. Completion is emitted only
/// after EOF; cancellation and read failures must leave verification unfinished.
pub fn sha256_with_progress(
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let total_bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut hashed_bytes = 0;
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 128];
    loop {
        ensure_not_cancelled(cancel)?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        hashed_bytes += read as u64;
        if total_bytes > 0 && last_emit.elapsed() >= Duration::from_millis(150) {
            progress((hashed_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0));
            last_emit = Instant::now();
        }
    }
    progress(1.0);
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn verify_sha256(
    app: &AppHandle,
    event: &str,
    model_id: &str,
    path: &Path,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    sha256_with_progress(path, cancel, |progress| {
        let _ = app.emit(
            event,
            VerificationProgress {
                model_id: model_id.to_string(),
                progress,
            },
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_vectors_completion_and_cancellation() {
        let dir = std::env::temp_dir().join(format!("verenu-shared-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture");
        let cancel = AtomicBool::new(false);
        for (bytes, hash) in [
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc".as_slice(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ] {
            std::fs::write(&path, bytes).unwrap();
            let mut events = Vec::new();
            assert_eq!(
                sha256_with_progress(&path, &cancel, |p| events.push(p)).unwrap(),
                hash
            );
            assert_eq!(events.last(), Some(&1.0));
            assert!(events.iter().all(|p| (0.0..=1.0).contains(p)));
        }
        std::fs::write(&path, vec![42; 256 * 1024]).unwrap();
        let mut events = Vec::new();
        let error = sha256_with_progress(&path, &cancel, |p| {
            events.push(p);
            cancel.store(true, Ordering::Relaxed);
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "download cancelled");
        assert_eq!(events, vec![0.5]);
        events.clear();
        assert!(sha256_with_progress(&path, &cancel, |p| events.push(p)).is_err());
        assert!(events.is_empty());
        cancel.store(false, Ordering::Relaxed);
        assert!(sha256_with_progress(&dir.join("missing"), &cancel, |p| events.push(p)).is_err());
        assert!(events.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn payload_shapes_remain_identical() {
        assert_eq!(
            serde_json::to_value(DownloadProgress {
                model_id: "fixture".into(),
                downloaded_bytes: 5,
                total_bytes: Some(10),
                progress: 0.5,
            })
            .unwrap(),
            serde_json::json!({"model_id":"fixture", "downloaded_bytes":5, "total_bytes":10, "progress":0.5})
        );
        assert_eq!(
            serde_json::to_value(ModelEvent {
                model_id: "fixture".into(),
                error: None
            })
            .unwrap(),
            serde_json::json!({"model_id":"fixture", "error":null})
        );
        assert_eq!(
            serde_json::to_value(VerificationProgress {
                model_id: "fixture".into(),
                progress: 1.0
            })
            .unwrap(),
            serde_json::json!({"model_id":"fixture", "progress":1.0})
        );
    }
}
