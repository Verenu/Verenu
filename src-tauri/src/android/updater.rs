//! Verified APK staging and Android's user-approved installer handoff.
use sha2::{Digest, Sha256};
use tauri::{plugin::PluginHandle, AppHandle, Emitter, Manager, Runtime};
use tokio::io::AsyncWriteExt;

struct AndroidUpdater<R: Runtime>(PluginHandle<R>);

#[derive(serde::Deserialize)]
struct Prepared {
    directory: String,
}

pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("verenu-updater")
        .setup(|app, api| {
            app.manage(AndroidUpdater(api.register_android_plugin(
                "com.verenu.app",
                "VerenuUpdaterPlugin",
            )?));
            Ok(())
        })
        .build()
}

fn progress(app: &AppHandle, phase: &str, downloaded: u64, total: Option<u64>) {
    let _ = app.emit(
        "verenu:update-progress",
        serde_json::json!({
            "phase": phase, "downloaded": downloaded, "total": total,
        }),
    );
}

pub async fn install(
    app: &AppHandle,
    url: &str,
    channel: crate::api::updater::UpdateChannel,
) -> Result<crate::commands::InstallOutcome, String> {
    let plugin = app.state::<AndroidUpdater<tauri::Wry>>();
    // Ask for source permission before downloading. The user returns to the
    // update button after allowing Verenu; no pending intent survives restart.
    let prepared: Prepared = plugin
        .0
        .run_mobile_plugin_async("prepare", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())?;
    progress(app, "resolving", 0, None);
    let (_, expected) = crate::api::updater::resolve_verified_download(url, channel)
        .await
        .map_err(|e| crate::api::user_facing_error(&e))?;
    let path = std::path::Path::new(&prepared.directory).join("update.apk");
    let result = download(app, url, &path, &expected).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }
    let handoff: Result<(), _> = plugin
        .0
        .run_mobile_plugin_async("install", serde_json::json!({}))
        .await;
    if let Err(error) = handoff {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error.to_string());
    }
    // The system installer is still awaiting approval. Never claim success.
    Ok(crate::commands::InstallOutcome::InstallerOpened)
}

async fn download(
    app: &AppHandle,
    url: &str,
    path: &std::path::Path,
    expected: &str,
) -> Result<(), String> {
    const LIMIT: u64 = 1024 * 1024 * 1024;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(60))
        .timeout(std::time::Duration::from_secs(30 * 60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .header("User-Agent", "verenu")
        .send()
        .await
        .map_err(|e| crate::api::updater::request_error_message(&e))?
        .error_for_status()
        .map_err(|e| crate::api::updater::request_error_message(&e))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > LIMIT) {
        return Err("The update exceeds the 1 GiB download limit.".into());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut downloaded = 0;
    let mut last_event = std::time::Instant::now();
    progress(app, "downloading", 0, total);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| crate::api::updater::request_error_message(&e))?
    {
        downloaded += chunk.len() as u64;
        if downloaded > LIMIT {
            return Err("The update exceeds the 1 GiB download limit.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        if last_event.elapsed() >= std::time::Duration::from_millis(200) {
            progress(app, "downloading", downloaded, total);
            last_event = std::time::Instant::now();
        }
    }
    file.sync_all().await.map_err(|e| e.to_string())?;
    progress(app, "verifying", downloaded, total);
    if downloaded == 0 || format!("{:x}", hash.finalize()) != expected {
        return Err("The APK checksum did not match. Installation was blocked. Try again.".into());
    }
    Ok(())
}
