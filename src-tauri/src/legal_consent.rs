use crate::data::store::SettingsSnapshot;
use rand::RngCore;
use serde::Serialize;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use tauri::AppHandle;

pub(crate) const TERMS_VERSION: &str = "2026-11-03";
pub(crate) const PRIVACY_VERSION: &str = "2026-11-03.1";
pub(crate) const ACCEPTED_TERMS: &str = "legal_accepted_terms_version";
pub(crate) const ACCEPTED_PRIVACY: &str = "legal_accepted_privacy_version";
pub(crate) const DELETE_PENDING: &str = "legal_delete_pending";
pub(crate) const INSTALL_ID: &str = "legal_install_id";
pub(crate) const SKIPPED: &str = "legal_skipped";
pub(crate) const ACCEPTED_AT: &str = "legal_accepted_at";
pub(crate) const SYNC_PENDING: &str = "legal_sync_pending";
pub(crate) const EVER_ACCEPTED: &str = "legal_ever_accepted";
static IDENTITY_LOCK: Mutex<()> = Mutex::new(());
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static DELETE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static NETWORK_GATE: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());
static REVOKING: AtomicBool = AtomicBool::new(false);
static FORCED_OFF: AtomicBool = AtomicBool::new(false);
static NETWORK_SETTINGS: OnceLock<crate::data::store::SettingsHandle> = OnceLock::new();

pub(crate) fn install_network_settings(settings: crate::data::store::SettingsHandle) {
    if let Ok(snapshot) = settings.snapshot() {
        set_local_downloads_allowed(crate::commands::settings::privacy_toggle_enabled(
            snapshot.get(crate::data::store::LOCAL_MODEL_DOWNLOADS_ENABLED).cloned(),
        ));
    }
    let _ = NETWORK_SETTINGS.set(settings);
}

#[cfg(test)]
pub(crate) fn enable_test_consent() {
    NETWORK_SETTINGS.get_or_init(|| {
        let settings = crate::data::store::SettingsHandle::empty_for_test(
            std::env::temp_dir().join(format!("verenu-test-consent-{}.json", uuid::Uuid::new_v4())),
        );
        settings.set(ACCEPTED_TERMS, serde_json::json!(TERMS_VERSION)).unwrap();
        settings.set(ACCEPTED_PRIVACY, serde_json::json!(PRIVACY_VERSION)).unwrap();
        settings
    });
}

pub(crate) fn require_consent() -> anyhow::Result<()> {
    let allowed = !REVOKING.load(Ordering::Acquire) && !FORCED_OFF.load(Ordering::Acquire) && NETWORK_SETTINGS
        .get()
        .is_some_and(|settings| {
            settings.data_directory().is_some_and(|dir| matches!(crate::legal_deletion::marker_path(dir).try_exists(), Ok(false)))
                && settings.snapshot().is_ok_and(|snapshot| permits_network(&snapshot))
        });
    consent_result(allowed)
}

pub(crate) async fn network_permit() -> anyhow::Result<tokio::sync::RwLockReadGuard<'static, ()>> {
    network_permit_now()
}

pub(crate) fn network_permit_now() -> anyhow::Result<tokio::sync::RwLockReadGuard<'static, ()>> {
    let guard = NETWORK_GATE.try_read().map_err(|_| anyhow::anyhow!("consent_required"))?;
    require_consent()?;
    Ok(guard)
}

pub(crate) async fn while_consented<F: std::future::Future>(future: F) -> anyhow::Result<F::Output> {
    while_checked(require_consent, future).await
}

/// Mirrors the Settings > Privacy toggle for local model downloads. Defaults on.
static LOCAL_DOWNLOADS_ALLOWED: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_local_downloads_allowed(allowed: bool) {
    LOCAL_DOWNLOADS_ALLOWED.store(allowed, Ordering::Release);
}

/// Consent plus the local-download toggle. Used by speech, cleanup, and runtime downloads only.
pub(crate) fn require_local_download() -> anyhow::Result<()> {
    require_consent()?;
    if LOCAL_DOWNLOADS_ALLOWED.load(Ordering::Acquire) {
        Ok(())
    } else {
        anyhow::bail!("Local model downloads are turned off. Turn them on in Settings > Privacy.")
    }
}

pub(crate) async fn while_local_download<F: std::future::Future>(future: F) -> anyhow::Result<F::Output> {
    while_checked(require_local_download, future).await
}

async fn while_checked<F: std::future::Future>(
    check: fn() -> anyhow::Result<()>,
    future: F,
) -> anyhow::Result<F::Output> {
    check()?;
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => check()?,
        }
    }
}

fn consent_result(allowed: bool) -> anyhow::Result<()> {
    if allowed {
        Ok(())
    } else {
        record_refusal();
        anyhow::bail!("consent_required")
    }
}

static LAST_REFUSAL_LOG: AtomicI64 = AtomicI64::new(0);
static REFUSALS_SINCE_LOG: AtomicU64 = AtomicU64::new(0);

/// Logs refused network work at most once a minute, with a count, so streaming loops stay quiet.
fn record_refusal() {
    REFUSALS_SINCE_LOG.fetch_add(1, Ordering::Relaxed);
    let now = chrono::Utc::now().timestamp();
    let last = LAST_REFUSAL_LOG.load(Ordering::Relaxed);
    if now - last >= 60
        && LAST_REFUSAL_LOG
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        let count = REFUSALS_SINCE_LOG.swap(0, Ordering::Relaxed);
        log::info!("legal: refused {count} network operation(s) because current consent is absent");
    }
}

fn possible_prior_acceptance(settings: &SettingsSnapshot) -> bool {
    let accepted_at = settings.get(ACCEPTED_AT).and_then(|value| value.as_i64()).is_some();
    accepted_at || match settings.get(EVER_ACCEPTED).and_then(|value| value.as_bool()) {
        Some(value) => value,
        None => settings.get(INSTALL_ID).and_then(|value| value.as_str()).is_some(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConsentState {
    status: &'static str,
    terms_version: &'static str,
    privacy_version: &'static str,
    accepted_at: Option<i64>,
    sync_pending: bool,
}

#[tauri::command]
pub(crate) async fn get_consent_state(app: AppHandle) -> Result<ConsentState, String> {
    let settings = crate::data::store::settings_snapshot(&app)?;
    let accepted_at = settings.get(ACCEPTED_AT).and_then(|value| value.as_i64());
    let deletion_marked = !matches!(crate::legal_deletion::marker_path(&crate::app_data_dir()).try_exists(), Ok(false));
    let status = if deletion_marked || settings
        .get(DELETE_PENDING)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "deletion_pending"
    } else if FORCED_OFF.load(Ordering::Acquire) {
        "skipped"
    } else if permits_network(&settings) {
        "accepted"
    } else if accepted_at.is_some() {
        "update_required"
    } else if settings
        .get(SKIPPED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "skipped"
    } else {
        "unanswered"
    };
    Ok(ConsentState {
        status,
        terms_version: TERMS_VERSION,
        privacy_version: PRIVACY_VERSION,
        accepted_at,
        sync_pending: settings
            .get(SYNC_PENDING)
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

#[tauri::command]
pub(crate) async fn skip_legal(app: AppHandle) -> Result<ConsentState, String> {
    let _sync_guard = SYNC_LOCK.lock().await;
    skip_legal_locked(app).await
}

async fn skip_legal_locked(app: AppHandle) -> Result<ConsentState, String> {
    log::info!("legal: user skipped consent; online features are off");
    struct ResetRevoking;
    impl Drop for ResetRevoking { fn drop(&mut self) { REVOKING.store(false, Ordering::Release); } }
    FORCED_OFF.store(true, Ordering::Release);
    REVOKING.store(true, Ordering::Release);
    let _reset = ResetRevoking;
    let settings = crate::data::store::settings_handle(&app)?;
    let ever_accepted = possible_prior_acceptance(&settings.snapshot()?);
    settings.save_values([
        (SKIPPED, serde_json::json!(true)),
        (ACCEPTED_TERMS, serde_json::Value::Null),
        (ACCEPTED_PRIVACY, serde_json::Value::Null),
        (ACCEPTED_AT, serde_json::Value::Null),
        (SYNC_PENDING, serde_json::json!(false)),
        (EVER_ACCEPTED, serde_json::json!(ever_accepted)),
    ])?;
    #[cfg(target_os = "android")]
    crate::android::security_plugin::revoke_analytics(&app).await?;
    {
        use tauri::Manager;
        if let Some(manager) = app.try_state::<crate::local_llm::LocalLlmManager>() {
            manager.cancel_download(None).map_err(|error| error.to_string())?;
            manager.cancel_runtime_download().map_err(|error| error.to_string())?;
            manager.wait_for_downloads_to_stop().await.map_err(|error| error.to_string())?;
        }
        if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() {
            manager.cancel_download(None).map_err(|error| error.to_string())?;
            manager.wait_for_downloads_to_stop().await.map_err(|error| error.to_string())?;
        }
    }
    let _network_guard = NETWORK_GATE.write().await;
    crate::media::nvidia_reuse::cancel_network_installation().await?;
    {
        use tauri::Manager;
        if let Some(sync) = app.try_state::<crate::sync::SyncManager>() {
            sync.cancel_pairing().await.map_err(|error| error.to_string())?;
            sync.suspend_discovery().await.map_err(|error| error.to_string())?;
            sync.wait_for_sessions_to_stop().await.map_err(|error| error.to_string())?;
        }
    }
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
            analytics.set_enabled(false);
        }
    }
    get_consent_state(app).await
}

#[tauri::command]
pub(crate) async fn delete_my_data(app: AppHandle) -> Result<ConsentState, String> {
    let _delete_guard = DELETE_LOCK.lock().await;
    let data_dir = crate::app_data_dir();
    crate::legal_deletion::validate_data_directory(&app, &data_dir)?;
    if crate::legal_deletion::read_marker(&data_dir)?.is_some() {
        return get_consent_state(app).await;
    }
    let id = get_install_id(app.clone()).await?;
    let secret = legal_secret(&app).await?;
    if secret.len() != 64 { return Err("The installation secret is unavailable".into()); }
    let _sync_guard = SYNC_LOCK.lock().await;
    skip_legal_locked(app.clone()).await?;
    let _check_guard = crate::media::nvidia_reuse::hold_checks_for_deletion().await;
    let settings = crate::data::store::settings_handle(&app)?;
    let needs_remote = possible_prior_acceptance(&settings.snapshot()?);
    #[cfg(target_os = "android")]
    {
        use std::io::Write;
        let mut origin = std::fs::File::create(data_dir.join("legal-deletion-origin-pid")).map_err(|error| error.to_string())?;
        write!(origin, "{}", std::process::id()).map_err(|error| error.to_string())?;
        origin.sync_all().map_err(|error| error.to_string())?;
    }
    crate::legal_deletion::write_marker(&data_dir, &id, needs_remote)?;
    {
        use tauri::Manager;
        if let Some(shared) = app.try_state::<crate::pipeline::SharedState>() {
            if let Some(active) = crate::pipeline::take_active_pipeline_for_interrupt(shared.inner()) {
                let _ = active.cancel_tx.send(true);
            }
            crate::pipeline::cancel_starting_reservation(shared.inner());
            if let Some((session, mic, _, _)) = crate::pipeline::take_recording_for_cancel(shared.inner()) {
                let _ = tokio::task::spawn_blocking(move || {
                    let result = session.stop();
                    if let Some(id) = mic { crate::system::volume::release_mic(id); }
                    result
                }).await;
            }
            crate::pipeline::clear_cancelled_capture(shared.inner());
        }
        if let Some(manager) = app.try_state::<crate::local_llm::LocalLlmManager>() { manager.unload(&app); }
        if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() { manager.unload(&app); }
    }
    #[cfg(target_os = "android")]
    crate::legal_deletion::delete_nonlegal_credentials(&app, &settings.snapshot()?).await?;
    crate::system::logger::stop_for_deletion()?;
    let state = get_consent_state(app.clone()).await?;
    #[cfg(target_os = "android")]
    crate::android::security_plugin::restart_for_deletion(&app).await?;
    #[cfg(not(target_os = "android"))]
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        app.restart();
    });
    Ok(state)
}

pub(crate) async fn retry_pending_deletion(app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let _delete_guard = DELETE_LOCK.lock().await;
    let data_dir = crate::app_data_dir();
    let Some(id) = crate::legal_deletion::read_marker(&data_dir)? else { return Ok(()); };
    crate::legal_deletion::validate_data_directory(&app, &data_dir)?;
    let settings = crate::data::store::settings_handle(&app)?;
    if !crate::legal_deletion::remote_delete_confirmed(&data_dir)? {
        if !crate::legal_deletion::local_wipe_complete(&data_dir)? {
            let snapshot = settings.snapshot()?;
            crate::legal_deletion::delete_nonlegal_credentials(&app, &snapshot).await?;
            settings.freeze_for_deletion(&id)?;
            crate::legal_deletion::isolate_database(app.state::<crate::DbHandle>().inner())?;
            crate::system::logger::stop_for_deletion()?;
            #[cfg(not(target_os = "android"))]
            crate::legal_deletion::wipe_external_caches(&app)?;
            crate::legal_deletion::wipe_directory(&data_dir, crate::is_dev_session())?;
            crate::legal_deletion::mark_local_wipe_complete(&data_dir)?;
        } else {
            settings.freeze_for_deletion(&id)?;
        }
        if crate::legal_deletion::needs_remote_delete(&data_dir)? {
            let secret = legal_secret(&app).await?;
            if secret.len() != 64 { return Err("The installation secret is unavailable".into()); }
            if crate::is_dev_session() { return Ok(()); }
            match crate::legal_deletion::confirm_remote_delete(&data_dir, "https://api.verenu.com/v1/legal/delete", &id, &secret).await? {
                crate::legal_deletion::RemoteDelete::Confirmed => {},
                crate::legal_deletion::RemoteDelete::Retry => return Ok(()),
                crate::legal_deletion::RemoteDelete::Rejected => return Err("Deletion authentication failed".into()),
            }
        } else {
            crate::legal_deletion::mark_remote_delete_confirmed(&data_dir)?;
        }
    }
    if crate::is_dev_session() {
        match std::fs::remove_file(data_dir.join("dev-legal-secret")) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    } else {
        #[cfg(target_os = "android")]
        crate::android::security_plugin::delete_legal_identity(&app).await?;
        #[cfg(not(target_os = "android"))]
        crate::data::credentials::delete_saved(&app, crate::data::store::LEGAL_INSTALL_SECRET)?;
    }
    #[cfg(target_os = "android")]
    crate::android::security_plugin::restart_for_deletion(&app).await?;
    crate::legal_deletion::clear_markers(&data_dir)?;
    #[cfg(not(target_os = "android"))]
    app.restart();
    #[cfg(target_os = "android")]
    Ok(())
}

#[tauri::command]
pub(crate) async fn accept_legal(app: AppHandle, terms_version: String, privacy_version: String) -> Result<ConsentState, String> {
    let _sync_guard = SYNC_LOCK.lock().await;
    if !shown_versions_are_current(&terms_version, &privacy_version) {
        return Err("consent_version_conflict".into());
    }
    if !matches!(crate::legal_deletion::marker_path(&crate::app_data_dir()).try_exists(), Ok(false)) {
        return Err("Data deletion is still pending".into());
    }
    let settings = crate::data::store::settings_handle(&app)?;
    if settings
        .get(DELETE_PENDING)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("Data deletion is still pending".into());
    }
    let id = get_install_id(app.clone()).await?;
    let secret = legal_secret(&app).await?;
    if secret.len() != 64 {
        return Err("The installation secret is unavailable".into());
    }
    let accepted_at = chrono::Utc::now().timestamp_millis();
    // An explicit acceptance may be sent before consent is enabled. Nothing
    // else can use this exception, and a server conflict must fail closed.
    let result = if crate::is_dev_session() {
        AcceptanceResult::Pending
    } else {
        post_acceptance("https://api.verenu.com/v1/legal/accept", &id, &secret, &terms_version, &privacy_version, accepted_at).await
    };
    if result == AcceptanceResult::Conflict {
        return match skip_legal_locked(app.clone()).await {
            Ok(_) => Err("consent_version_conflict".into()),
            Err(error) => Err(format!("consent_version_conflict: {error}")),
        };
    }
    settings.save_values([
        (ACCEPTED_TERMS, serde_json::json!(TERMS_VERSION)),
        (ACCEPTED_PRIVACY, serde_json::json!(PRIVACY_VERSION)),
        (ACCEPTED_AT, serde_json::json!(accepted_at)),
        (SKIPPED, serde_json::json!(false)),
        (SYNC_PENDING, serde_json::json!(result != AcceptanceResult::Synced)),
        (EVER_ACCEPTED, serde_json::json!(true)),
    ])?;
    log::info!(
        "legal: acceptance saved locally; server record {}",
        match result {
            AcceptanceResult::Synced => "confirmed",
            AcceptanceResult::Pending => "pending retry",
            AcceptanceResult::Conflict => "conflict",
        }
    );
    FORCED_OFF.store(false, Ordering::Release);
    {
        use tauri::Manager;
        if let Some(sync) = app.try_state::<crate::sync::SyncManager>() {
            if let Err(error) = sync.resume_discovery().await { log::warn!("sync: discovery resume failed: {error}"); }
        }
    }
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
            analytics.set_enabled(
                settings
                    .get(crate::data::store::ANALYTICS_ENABLED)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            );
        }
    }
    get_consent_state(app).await
}

fn shown_versions_are_current(terms: &str, privacy: &str) -> bool {
    terms == TERMS_VERSION && privacy == PRIVACY_VERSION
}

async fn sync_acceptance(
    settings: &crate::data::store::SettingsHandle,
    id: &str,
    secret: &str,
    accepted_at: i64,
) -> bool {
    if crate::is_dev_session() { return false; }
    sync_acceptance_to(settings, id, secret, accepted_at, "https://api.verenu.com/v1/legal/accept").await
}

async fn sync_acceptance_to(
    settings: &crate::data::store::SettingsHandle,
    id: &str,
    secret: &str,
    accepted_at: i64,
    endpoint: &str,
) -> bool {
    let Ok(_permit) = network_permit().await else { return false; };
    match post_acceptance(endpoint, id, secret, TERMS_VERSION, PRIVACY_VERSION, accepted_at).await {
        AcceptanceResult::Conflict => true,
        AcceptanceResult::Synced => {
            let _ = settings.save_value(SYNC_PENDING, serde_json::json!(false));
            false
        }
        AcceptanceResult::Pending => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AcceptanceResult { Synced, Pending, Conflict }

async fn post_acceptance(endpoint: &str, id: &str, secret: &str, terms: &str, privacy: &str, accepted_at: i64) -> AcceptanceResult {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return AcceptanceResult::Pending,
    };
    let result = client.post(endpoint)
        .bearer_auth(secret)
        .json(&serde_json::json!({ "userId": id, "termsVersion": terms, "privacyVersion": privacy, "acceptedAt": accepted_at }))
        .send().await;
    match result {
        Ok(response) if response.status() == reqwest::StatusCode::CONFLICT => AcceptanceResult::Conflict,
        Ok(response) if response.status().is_success() => AcceptanceResult::Synced,
        _ => AcceptanceResult::Pending,
    }
}

pub(crate) async fn retry_pending_acceptance(app: AppHandle) {
    let _sync_guard = SYNC_LOCK.lock().await;
    let Ok(settings) = crate::data::store::settings_handle(&app) else {
        return;
    };
    let Ok(snapshot) = settings.snapshot() else {
        return;
    };
    if !permits_network(&snapshot)
        || !snapshot
            .get(SYNC_PENDING)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return;
    }
    let Some(id) = snapshot.get(INSTALL_ID).and_then(|v| v.as_str()) else {
        return;
    };
    let Some(accepted_at) = snapshot.get(ACCEPTED_AT).and_then(|v| v.as_i64()) else {
        return;
    };
    let Ok(secret) = legal_secret(&app).await else {
        return;
    };
    if secret.len() == 64 && sync_acceptance(&settings, id, &secret, accepted_at).await {
        drop(_sync_guard);
        let _ = skip_legal(app).await;
    }
}

#[tauri::command]
pub(crate) async fn get_install_id(app: AppHandle) -> Result<String, String> {
    if let Some(id) = crate::legal_deletion::read_marker(&crate::app_data_dir())? {
        return Ok(id);
    }
    #[cfg(target_os = "android")]
    {
        let identity = crate::android::security_plugin::legal_identity(&app).await?;
        let settings = crate::data::store::settings_handle(&app)?;
        if let Some(existing) = settings
            .get(INSTALL_ID)
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            if existing != identity.id {
                return Err("Installation identity does not match secure storage".into());
            }
        } else {
            settings.save_values([
                (INSTALL_ID, identity.id.clone().into()),
                (EVER_ACCEPTED, serde_json::json!(false)),
            ])?;
        }
        Ok(identity.id)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _guard = IDENTITY_LOCK
            .lock()
            .map_err(|_| "Installation identity lock failed")?;
        let settings = crate::data::store::settings_handle(&app)?;
        if let Some(id) = settings
            .get(INSTALL_ID)
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            return Ok(id);
        }
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        if crate::is_dev_session() {
            use std::io::Write;
            let path = crate::app_data_dir().join("dev-legal-secret");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
            match options.open(&path) {
                Ok(mut file) => {
                    file.write_all(secret.as_bytes()).map_err(|error| error.to_string())?;
                    file.sync_all().map_err(|error| error.to_string())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let existing = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
                    if existing.len() != 64 || !existing.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
                        return Err("Invalid dev-session installation secret".into());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        } else {
            crate::data::credentials::save(&app, crate::data::store::LEGAL_INSTALL_SECRET, &secret)?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        if let Err(error) = settings.save_values([
            (INSTALL_ID, id.clone().into()),
            (EVER_ACCEPTED, serde_json::json!(false)),
        ]) {
            if crate::is_dev_session() { let _ = std::fs::remove_file(crate::app_data_dir().join("dev-legal-secret")); }
            return Err(error);
        }
        Ok(id)
    }
}

async fn legal_secret(app: &AppHandle) -> Result<String, String> {
    if crate::is_dev_session() {
        return std::fs::read_to_string(crate::app_data_dir().join("dev-legal-secret")).map_err(|error| error.to_string());
    }
    #[cfg(target_os = "android")]
    {
        Ok(crate::android::security_plugin::legal_identity(app)
            .await?
            .secret)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(crate::data::credentials::get(
            crate::data::store::LEGAL_INSTALL_SECRET,
        ))
    }
}

/// Fail closed when either document changes or deletion is pending.
pub(crate) fn permits_network(settings: &SettingsSnapshot) -> bool {
    settings.get(ACCEPTED_TERMS).and_then(|v| v.as_str()) == Some(TERMS_VERSION)
        && settings.get(ACCEPTED_PRIVACY).and_then(|v| v.as_str()) == Some(PRIVACY_VERSION)
        && !settings.get(SKIPPED).and_then(|v| v.as_bool()).unwrap_or(false)
        && !settings
            .get(DELETE_PENDING)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_download_toggle_blocks_downloads_even_with_consent() {
        enable_test_consent();
        set_local_downloads_allowed(false);
        let error = require_local_download().expect_err("toggle off must block downloads");
        assert!(error.to_string().contains("Local model downloads are turned off"));
        set_local_downloads_allowed(true);
        require_local_download().expect("toggle on with consent allows downloads");
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn acceptance_rejects_versions_not_shown_to_the_user() {
        assert!(shown_versions_are_current(TERMS_VERSION, PRIVACY_VERSION));
        assert!(!shown_versions_are_current(TERMS_VERSION, "2026-11-03"));
        assert!(!shown_versions_are_current("old", PRIVACY_VERSION));
    }

    #[tokio::test]
    async fn acceptance_conflict_is_detected_before_local_acceptance() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 2048];
            loop {
                let count = socket.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..count]);
                if String::from_utf8_lossy(&request).contains(PRIVACY_VERSION) { break; }
                assert!(count > 0);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains(TERMS_VERSION));
            assert!(request.contains(PRIVACY_VERSION));
            socket.write_all(b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let result = post_acceptance(&format!("http://{address}"), "synthetic-id", "synthetic-secret", TERMS_VERSION, PRIVACY_VERSION, 1).await;
        server.join().unwrap();
        assert_eq!(result, AcceptanceResult::Conflict);
        assert!(!permits_network(&SettingsSnapshot::from_pairs([])));
    }

    #[test]
    fn deletion_distinguishes_new_skippers_from_legacy_identities() {
        assert!(!possible_prior_acceptance(&SettingsSnapshot::from_pairs([])));
        assert!(!possible_prior_acceptance(&SettingsSnapshot::from_pairs([
            (INSTALL_ID.into(), json!("synthetic-id")),
            (EVER_ACCEPTED.into(), json!(false)),
        ])));
        assert!(possible_prior_acceptance(&SettingsSnapshot::from_pairs([
            (INSTALL_ID.into(), json!("legacy-id")),
        ])));
        assert!(possible_prior_acceptance(&SettingsSnapshot::from_pairs([
            (EVER_ACCEPTED.into(), json!(false)),
            (ACCEPTED_AT.into(), json!(1)),
        ])));
    }

    #[test]
    fn consent_requires_both_current_versions_and_no_pending_deletion() {
        let snapshot = |terms: &str, privacy: &str, pending: bool| {
            SettingsSnapshot::from_pairs([
                (ACCEPTED_TERMS.into(), json!(terms)),
                (ACCEPTED_PRIVACY.into(), json!(privacy)),
                (DELETE_PENDING.into(), json!(pending)),
            ])
        };
        assert!(!permits_network(&SettingsSnapshot::from_pairs([])));
        assert!(!permits_network(&snapshot("old", PRIVACY_VERSION, false)));
        assert!(!permits_network(&snapshot(TERMS_VERSION, "old", false)));
        assert!(!permits_network(&snapshot(TERMS_VERSION, "2026-11-03", false)));
        assert!(!permits_network(&snapshot(
            TERMS_VERSION,
            PRIVACY_VERSION,
            true
        )));
        assert!(permits_network(&snapshot(
            TERMS_VERSION,
            PRIVACY_VERSION,
            false
        )));
        assert!(!permits_network(&SettingsSnapshot::from_pairs([
            (ACCEPTED_TERMS.into(), json!(TERMS_VERSION)),
            (ACCEPTED_PRIVACY.into(), json!(PRIVACY_VERSION)),
            (SKIPPED.into(), json!(true)),
        ])));
    }

    #[test]
    fn consent_gate_prevents_mock_transport_when_skipped() {
        let calls = std::cell::Cell::new(0);
        let send = |settings: &SettingsSnapshot| {
            if consent_result(permits_network(settings)).is_ok() {
                calls.set(calls.get() + 1);
            }
        };
        send(&SettingsSnapshot::from_pairs([]));
        assert_eq!(calls.get(), 0);
        send(&SettingsSnapshot::from_pairs([
            (ACCEPTED_TERMS.into(), json!(TERMS_VERSION)),
            (ACCEPTED_PRIVACY.into(), json!(PRIVACY_VERSION)),
        ]));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn transcription_transport_requires_consent() {
        const CHILD: &str = "VERENU_CONSENT_TRANSPORT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "legal_consent::tests::transcription_transport_requires_consent"])
                .env(CHILD, "1")
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use std::io::{Read, Write};
        let test_dir = std::env::temp_dir().join(format!("verenu-consent-transport-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&test_dir).unwrap();
        std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", &test_dir);
        let settings = crate::data::store::SettingsHandle::empty_for_test(test_dir.join("settings.json"));
        settings.set(SKIPPED, json!(true)).unwrap();
        install_network_settings(settings.clone());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let analytics_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        analytics_listener.set_nonblocking(true).unwrap();
        let analytics_url = format!("http://{}/capture", analytics_listener.local_addr().unwrap());
        let analytics_enabled = AtomicBool::new(true);
        let cleanup_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        cleanup_listener.set_nonblocking(true).unwrap();
        let acceptance_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        acceptance_listener.set_nonblocking(true).unwrap();
        let acceptance_url = format!("http://{}/accept", acceptance_listener.local_addr().unwrap());
        let provider = crate::api::custom::CustomProvider {
            id: crate::api::custom::new_custom_id(), name: "fixture".into(),
            protocol: crate::api::custom::CustomProtocol::Openai,
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            requires_key: false, supports_transcription: true, supports_cleanup: true,
            auth_header: None, extra_headers: Default::default(), body_overrides: None,
            transcription_models: vec![], cleanup_models: vec![],
        };
        let request = || crate::api::transcription::transcribe(bytes::Bytes::from_static(b"fixture"), crate::api::Target::Custom(Box::new(provider.clone())), "", "en", "fixture", 0);
        let mut cleanup_provider = provider.clone();
        cleanup_provider.base_url = format!("http://{}/v1", cleanup_listener.local_addr().unwrap());
        cleanup_provider.supports_cleanup = true;
        let cleanup_request = || crate::api::cleanup::cleanup_with_alternate_and_evidence(
            "synthetic", crate::api::Target::Custom(Box::new(cleanup_provider.clone())), "", "fixture", "formal", "medium", "", "", None, None, None, 0,
        );
        assert!(request().await.is_err());
        assert!(cleanup_request().await.is_err());
        assert_eq!(cleanup_listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        assert!(crate::analytics::deliver_analytics(&analytics_url, &json!({"event": "synthetic"}), &analytics_enabled).await.is_none());
        assert_eq!(analytics_listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        assert!(!sync_acceptance_to(&settings, "synthetic-id", "synthetic-secret", 1, &acceptance_url).await);
        assert_eq!(acceptance_listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(crate::commands::check_domain_exists("127.0.0.1".into()).await.unwrap_err(), "consent_required");
        assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        settings.set(ACCEPTED_TERMS, json!(TERMS_VERSION)).unwrap();
        settings.set(ACCEPTED_PRIVACY, json!(PRIVACY_VERSION)).unwrap();
        settings.set(SKIPPED, json!(false)).unwrap();
        FORCED_OFF.store(true, Ordering::Release);
        assert!(network_permit_now().is_err());
        FORCED_OFF.store(false, Ordering::Release);
        assert!(crate::commands::check_domain_exists("127.0.0.1".into()).await.unwrap());
        let cleanup_server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match cleanup_listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => panic!("mock cleanup transport failed: {e}"),
                }
            };
            socket.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers.lines().filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok()).unwrap();
                    if request.len() >= end + 4 + length { break; }
                }
                assert!(request.len() < 1024 * 1024);
            }
            let body = r#"{"choices":[{"message":{"content":"fixture"}}]}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        assert_eq!(cleanup_request().await.unwrap(), "fixture");
        cleanup_server.join().unwrap();
        settings.set(SYNC_PENDING, json!(true)).unwrap();
        let acceptance_server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match acceptance_listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => panic!("mock acceptance transport failed: {e}"),
                }
            };
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !String::from_utf8_lossy(&request).contains("2026-11-03.1") {
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert!(!sync_acceptance_to(&settings, "synthetic-id", "synthetic-secret", 1, &acceptance_url).await);
        acceptance_server.join().unwrap();
        assert_eq!(settings.get(SYNC_PENDING), Some(json!(false)));
        let analytics_server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match analytics_listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => panic!("mock analytics transport failed: {e}"),
                }
            };
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert!(crate::analytics::deliver_analytics(&analytics_url, &json!({"event": "synthetic"}), &analytics_enabled).await.unwrap().is_ok());
        analytics_server.join().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(e) => panic!("mock transport failed: {e}"),
                }
            };
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer);
            let body = r#"{"text":"fixture"}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        assert_eq!(request().await.unwrap(), "fixture");
        server.join().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let download = tokio::spawn(async move {
            let _permit = network_permit().await.unwrap();
            started_tx.send(()).unwrap();
            while_consented(std::future::pending::<()>()).await
        });
        started_rx.await.unwrap();
        settings.set(ACCEPTED_TERMS, serde_json::Value::Null).unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), download).await.unwrap().unwrap().is_err());
        std::fs::remove_dir_all(test_dir).unwrap();
    }
}
