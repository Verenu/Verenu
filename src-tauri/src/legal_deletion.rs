use std::path::Path;

const MARKER: &str = "legal-deletion-pending";
const WIPE_COMPLETE: &str = "legal-local-wipe-complete";
const REMOTE_CONFIRMED: &str = "legal-remote-delete-confirmed";
const ANDROID_WIPE_COMPLETE: &str = "legal-android-wipe-complete";
const ANDROID_ORIGIN: &str = "legal-deletion-origin-pid";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RemoteDelete {
    Confirmed,
    Retry,
    Rejected,
}

pub(crate) async fn delete_remote(endpoint: &str, id: &str, secret: &str) -> RemoteDelete {
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build() else {
        return RemoteDelete::Retry;
    };
    match client.post(endpoint).bearer_auth(secret).json(&serde_json::json!({ "userId": id })).send().await {
        Ok(response) if response.status().is_success() => RemoteDelete::Confirmed,
        Ok(response) if response.status() == reqwest::StatusCode::FORBIDDEN => RemoteDelete::Rejected,
        _ => RemoteDelete::Retry,
    }
}

pub(crate) async fn confirm_remote_delete(data_dir: &Path, endpoint: &str, id: &str, secret: &str) -> Result<RemoteDelete, String> {
    let result = delete_remote(endpoint, id, secret).await;
    log::info!(
        "legal: remote deletion {}",
        match result {
            RemoteDelete::Confirmed => "confirmed by the server",
            RemoteDelete::Rejected => "rejected by the server; not retried with this secret",
            RemoteDelete::Retry => "not confirmed; will retry",
        }
    );
    if result == RemoteDelete::Confirmed { mark_remote_delete_confirmed(data_dir)?; }
    Ok(result)
}

pub(crate) fn marker_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(MARKER)
}

pub(crate) fn validate_data_directory(app: &tauri::AppHandle, data_dir: &Path) -> Result<(), String> {
    if !data_dir.is_absolute() || std::fs::symlink_metadata(data_dir).map_err(|error| error.to_string())?.file_type().is_symlink() {
        return Err("Refusing to delete from an unsafe data directory".into());
    }
    if crate::is_dev_session() {
        let session = std::env::var_os("VERENU_DEV_SESSION_DIR").ok_or("Missing dev-session directory")?;
        if data_dir != std::path::PathBuf::from(session).join("data") { return Err("Invalid dev-session data directory".into()); }
        return Ok(());
    }
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        if app.path().app_data_dir().map_err(|error| error.to_string())? != data_dir {
            return Err("Invalid Android data directory".into());
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        if data_dir.file_name().and_then(|name| name.to_str()) != Some("Verenu") {
            return Err("Invalid Verenu data directory".into());
        }
    }
    Ok(())
}

pub(crate) fn write_marker(data_dir: &Path, id: &str, needs_remote: bool) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "Invalid installation identity")?;
    if parsed.to_string() != id {
        return Err("Invalid installation identity".into());
    }
    std::fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    if read_marker(data_dir)?.is_none() {
        for name in [WIPE_COMPLETE, REMOTE_CONFIRMED, ANDROID_WIPE_COMPLETE] {
            match std::fs::remove_file(data_dir.join(name)) {
                Ok(()) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    let path = marker_path(data_dir);
    let staging = data_dir.join("legal-deletion-pending.tmp");
    use std::io::Write;
    let mut file = std::fs::File::create(&staging).map_err(|error| error.to_string())?;
    let content = format!("{id}\n{}", if needs_remote { "remote" } else { "local" });
    file.write_all(content.as_bytes()).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(&staging, &path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(data_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn read_marker(data_dir: &Path) -> Result<Option<String>, String> {
    Ok(read_pending(data_dir)?.map(|(id, _)| id))
}

pub(crate) fn needs_remote_delete(data_dir: &Path) -> Result<bool, String> {
    read_pending(data_dir)?.map(|(_, remote)| remote).ok_or_else(|| "Missing pending deletion marker".into())
}

fn read_pending(data_dir: &Path) -> Result<Option<(String, bool)>, String> {
    let content = match std::fs::read_to_string(marker_path(data_dir)) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let (id_text, remote) = match content.split_once('\n') {
        Some((id, "remote")) => (id, true),
        Some((id, "local")) => (id, false),
        None => (content.as_str(), true),
        _ => return Err("Invalid pending deletion marker".into()),
    };
    let id = uuid::Uuid::parse_str(id_text).map_err(|_| "Invalid pending deletion marker")?;
    if id.to_string() != id_text { return Err("Invalid pending deletion marker".into()); }
    Ok(Some((id_text.to_owned(), remote)))
}

pub(crate) fn wipe_directory(data_dir: &Path, preserve_dev_secret: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let _ = preserve_dev_secret;
        if read_marker(data_dir)?.is_none() || !data_dir.join(ANDROID_WIPE_COMPLETE).is_file() {
            return Err("Waiting for Android deletion recovery".into());
        }
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
    if read_marker(data_dir)?.is_none() {
        return Err("Cannot wipe data without a pending deletion marker".into());
    }
    let canonical = std::fs::canonicalize(data_dir).map_err(|error| error.to_string())?;
    if canonical.parent().is_none() || canonical.parent().and_then(Path::parent).is_none() {
        return Err("Refusing to wipe an unsafe data directory".into());
    }
    for entry in std::fs::read_dir(&canonical).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name == MARKER || name == WIPE_COMPLETE || name == REMOTE_CONFIRMED || (preserve_dev_secret && name == "dev-legal-secret") { continue; }
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        } else {
            std::fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
    }
}

pub(crate) fn mark_local_wipe_complete(data_dir: &Path) -> Result<(), String> {
    if read_marker(data_dir)?.is_none() { return Err("Missing pending deletion marker".into()); }
    use std::io::Write;
    let path = data_dir.join(WIPE_COMPLETE);
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(b"1").map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(data_dir).and_then(|directory| directory.sync_all()).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn local_wipe_complete(data_dir: &Path) -> Result<bool, String> {
    data_dir.join(WIPE_COMPLETE).try_exists().map_err(|error| error.to_string())
}

pub(crate) fn remote_delete_confirmed(data_dir: &Path) -> Result<bool, String> {
    data_dir.join(REMOTE_CONFIRMED).try_exists().map_err(|error| error.to_string())
}

pub(crate) fn mark_remote_delete_confirmed(data_dir: &Path) -> Result<(), String> {
    if read_marker(data_dir)?.is_none() || !local_wipe_complete(data_dir)? {
        return Err("Deletion is not ready for confirmation".into());
    }
    let file = std::fs::File::create(data_dir.join(REMOTE_CONFIRMED)).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(data_dir).and_then(|directory| directory.sync_all()).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn clear_markers(data_dir: &Path) -> Result<(), String> {
    // Remove the confirmation last so a crash cannot lose proof of remote deletion.
    for name in [MARKER, WIPE_COMPLETE, ANDROID_WIPE_COMPLETE, ANDROID_ORIGIN, REMOTE_CONFIRMED] {
        match std::fs::remove_file(data_dir.join(name)) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    #[cfg(unix)]
    std::fs::File::open(data_dir).and_then(|directory| directory.sync_all()).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn isolate_database(db: &crate::DbHandle) -> Result<(), String> {
    let replacement = rusqlite::Connection::open_in_memory().map_err(|error| error.to_string())?;
    replacement.execute_batch("PRAGMA query_only=ON;").map_err(|error| error.to_string())?;
    let mut connection = db.lock().map_err(|_| "Database lock was poisoned".to_string())?;
    let old = std::mem::replace(&mut *connection, replacement);
    drop(old);
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub(crate) fn wipe_external_caches(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if crate::is_dev_session() { return Ok(()); }
    if read_marker(&crate::app_data_dir())?.is_none() { return Err("Missing pending deletion marker".into()); }
    let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let app_cache = app.path().app_cache_dir().map_err(|error| error.to_string())?;
    let identifier = app.config().identifier.as_str();
    if !app_data.is_absolute() || !app_cache.is_absolute()
        || app_data.file_name().and_then(|name| name.to_str()) != Some(identifier)
        || app_cache.file_name().and_then(|name| name.to_str()) != Some(identifier)
    {
        return Err("Invalid application cache directory".into());
    }
    if std::fs::symlink_metadata(&app_data).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("Refusing a symlinked application directory".into());
    }
    for path in [app_data.join("icon-cache"), app_data.join("favicon-cache"), app_cache] {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err("Refusing a symlinked cache directory".into()),
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path).map_err(|error| error.to_string())?,
            Ok(_) => return Err("Invalid cache directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub(crate) async fn delete_nonlegal_credentials(app: &tauri::AppHandle, snapshot: &crate::data::store::SettingsSnapshot) -> Result<(), String> {
    if crate::is_dev_session() {
        return crate::sync::secrets::delete_identity_key(&crate::app_data_dir());
    }
    #[cfg(target_os = "android")]
    {
        let _ = snapshot;
        crate::android::security_plugin::delete_provider_credentials(app).await?;
        crate::android::clear_credentials();
    }
    #[cfg(not(target_os = "android"))]
    {
        use crate::data::store;
        for provider in [store::GROQ, store::OPENAI, store::GOOGLE, store::ASSEMBLYAI, store::OPENROUTER, store::XAI, store::T3_INTEGRATION, store::T3_INTEGRATION_ALTERNATE] {
            crate::data::credentials::delete_saved(app, provider)?;
        }
        for provider in crate::api::custom::parse_stored(snapshot.get(store::CUSTOM_PROVIDERS)) {
            crate::data::credentials::delete_saved(app, &provider.id)?;
        }
    }
    crate::sync::secrets::delete_identity_key(&crate::app_data_dir())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_marker_survives_reopen_and_rejects_invalid_ids() {
        let dir = std::env::temp_dir().join(format!("verenu-deletion-test-{}", uuid::Uuid::new_v4()));
        let id = uuid::Uuid::new_v4().to_string();
        write_marker(&dir, &id, true).unwrap();
        assert_eq!(read_marker(&dir).unwrap(), Some(id));
        assert!(write_marker(&dir, "invalid", true).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_wipe_requires_marker_and_preserves_only_pending_identity() {
        let dir = std::env::temp_dir().join(format!("verenu-deletion-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("cache")).unwrap();
        std::fs::write(dir.join("cache/private"), b"synthetic").unwrap();
        std::fs::write(dir.join("dev-legal-secret"), b"synthetic-secret").unwrap();
        assert!(wipe_directory(&dir, true).is_err());
        let id = uuid::Uuid::new_v4().to_string();
        write_marker(&dir, &id, true).unwrap();
        wipe_directory(&dir, true).unwrap();
        assert_eq!(read_marker(&dir).unwrap(), Some(id));
        assert!(dir.join("dev-legal-secret").exists());
        assert!(!dir.join("cache").exists());
        assert!(!local_wipe_complete(&dir).unwrap());
        mark_local_wipe_complete(&dir).unwrap();
        assert!(local_wipe_complete(&dir).unwrap());
        assert!(!remote_delete_confirmed(&dir).unwrap());
        mark_remote_delete_confirmed(&dir).unwrap();
        assert!(remote_delete_confirmed(&dir).unwrap());
        clear_markers(&dir).unwrap();
        assert_eq!(read_marker(&dir).unwrap(), None);
        assert!(!local_wipe_complete(&dir).unwrap());
        assert!(!remote_delete_confirmed(&dir).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_completion_flags_do_not_skip_a_new_wipe() {
        let dir = std::env::temp_dir().join(format!("verenu-deletion-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(WIPE_COMPLETE), b"1").unwrap();
        std::fs::write(dir.join(REMOTE_CONFIRMED), b"1").unwrap();
        write_marker(&dir, &uuid::Uuid::new_v4().to_string(), false).unwrap();
        assert!(!needs_remote_delete(&dir).unwrap());
        assert!(!local_wipe_complete(&dir).unwrap());
        assert!(!remote_delete_confirmed(&dir).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn database_isolation_closes_disk_connection_and_blocks_writes() {
        let dir = std::env::temp_dir().join(format!("verenu-deletion-db-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.sqlite");
        let db = std::sync::Arc::new(std::sync::Mutex::new(rusqlite::Connection::open(&path).unwrap()));
        db.lock().unwrap().execute_batch("CREATE TABLE item (value TEXT); INSERT INTO item VALUES ('synthetic');").unwrap();
        let other_handle = db.clone();
        isolate_database(&db).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(other_handle.lock().unwrap().execute_batch("CREATE TABLE recreated (value TEXT)").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn remote_delete_distinguishes_success_offline_and_wrong_secret() {
        use std::io::{Read, Write};
        async fn response(status: &str) -> RemoteDelete {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/delete", listener.local_addr().unwrap());
            let status = status.to_owned();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let result = delete_remote(&url, "synthetic-id", "synthetic-secret").await;
            server.join().unwrap();
            result
        }
        assert_eq!(response("200 OK").await, RemoteDelete::Confirmed);
        assert_eq!(response("403 Forbidden").await, RemoteDelete::Rejected);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/delete", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(delete_remote(&url, "synthetic-id", "synthetic-secret").await, RemoteDelete::Retry);
    }

    #[tokio::test]
    async fn offline_deletion_keeps_marker_until_successful_retry() {
        const CHILD: &str = "VERENU_DELETION_TRANSPORT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "legal_deletion::tests::offline_deletion_keeps_marker_until_successful_retry"])
                .env(CHILD, "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("verenu-deletion-retry-{}", uuid::Uuid::new_v4()));
        let settings = crate::data::store::SettingsHandle::empty_for_test(dir.join("settings.json"));
        settings.set(crate::legal_consent::SKIPPED, serde_json::json!(true)).unwrap();
        crate::legal_consent::install_network_settings(settings);
        assert!(crate::legal_consent::require_consent().is_err());
        let id = uuid::Uuid::new_v4().to_string();
        write_marker(&dir, &id, true).unwrap();
        mark_local_wipe_complete(&dir).unwrap();
        let offline = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let offline_url = format!("http://{}/delete", offline.local_addr().unwrap());
        drop(offline);
        assert_eq!(confirm_remote_delete(&dir, &offline_url, &id, "synthetic-secret").await.unwrap(), RemoteDelete::Retry);
        assert_eq!(read_marker(&dir).unwrap(), Some(id.clone()));
        assert!(!remote_delete_confirmed(&dir).unwrap());
        let rejected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let rejected_url = format!("http://{}/delete", rejected.local_addr().unwrap());
        let rejected_server = std::thread::spawn(move || {
            let (mut stream, _) = rejected.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert_eq!(confirm_remote_delete(&dir, &rejected_url, &id, "wrong-secret").await.unwrap(), RemoteDelete::Rejected);
        rejected_server.join().unwrap();
        assert!(!remote_delete_confirmed(&dir).unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/delete", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert_eq!(confirm_remote_delete(&dir, &endpoint, &id, "synthetic-secret").await.unwrap(), RemoteDelete::Confirmed);
        server.join().unwrap();
        assert!(remote_delete_confirmed(&dir).unwrap());
        clear_markers(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
