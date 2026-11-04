use crate::data::store::SettingsSnapshot;
use rand::RngCore;
use serde::Serialize;
use std::sync::Mutex;
use tauri::AppHandle;

pub(crate) const TERMS_VERSION: &str = "2026-11-03";
pub(crate) const PRIVACY_VERSION: &str = "2026-11-03";
pub(crate) const ACCEPTED_TERMS: &str = "legal_accepted_terms_version";
pub(crate) const ACCEPTED_PRIVACY: &str = "legal_accepted_privacy_version";
pub(crate) const DELETE_PENDING: &str = "legal_delete_pending";
const INSTALL_ID: &str = "legal_install_id";
const SKIPPED: &str = "legal_skipped";
const ACCEPTED_AT: &str = "legal_accepted_at";
const SYNC_PENDING: &str = "legal_sync_pending";
static IDENTITY_LOCK: Mutex<()> = Mutex::new(());
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let status = if settings
        .get(DELETE_PENDING)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "deletion_pending"
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
    let settings = crate::data::store::settings_handle(&app)?;
    settings.save_values([
        (SKIPPED, serde_json::json!(true)),
        (ACCEPTED_TERMS, serde_json::Value::Null),
        (ACCEPTED_PRIVACY, serde_json::Value::Null),
        (ACCEPTED_AT, serde_json::Value::Null),
        (SYNC_PENDING, serde_json::json!(false)),
    ])?;
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
pub(crate) async fn delete_my_data() -> Result<ConsentState, String> {
    Err("Data deletion is not available yet".into())
}

#[tauri::command]
pub(crate) async fn accept_legal(app: AppHandle) -> Result<ConsentState, String> {
    let _sync_guard = SYNC_LOCK.lock().await;
    let settings = crate::data::store::settings_handle(&app)?;
    if settings
        .get(DELETE_PENDING)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("Data deletion is still pending".into());
    }
    let id = get_install_id(app.clone()).await?;
    let secret = crate::data::credentials::get(crate::data::store::LEGAL_INSTALL_SECRET);
    if secret.len() != 64 {
        return Err("The installation secret is unavailable".into());
    }
    let accepted_at = chrono::Utc::now().timestamp_millis();
    settings.save_values([
        (ACCEPTED_TERMS, serde_json::json!(TERMS_VERSION)),
        (ACCEPTED_PRIVACY, serde_json::json!(PRIVACY_VERSION)),
        (ACCEPTED_AT, serde_json::json!(accepted_at)),
        (SKIPPED, serde_json::json!(false)),
        (SYNC_PENDING, serde_json::json!(true)),
    ])?;
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
    sync_acceptance(&settings, &id, &secret, accepted_at).await;
    get_consent_state(app).await
}

async fn sync_acceptance(
    settings: &crate::data::store::SettingsHandle,
    id: &str,
    secret: &str,
    accepted_at: i64,
) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    let result = client.post("https://api.verenu.com/v1/legal/accept")
        .bearer_auth(secret)
        .json(&serde_json::json!({ "userId": id, "termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "acceptedAt": accepted_at }))
        .send().await;
    if result.is_ok_and(|response| response.status().is_success()) {
        let _ = settings.save_value(SYNC_PENDING, serde_json::json!(false));
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
    let secret = crate::data::credentials::get(crate::data::store::LEGAL_INSTALL_SECRET);
    if secret.len() == 64 {
        sync_acceptance(&settings, id, &secret, accepted_at).await;
    }
}

#[tauri::command]
pub(crate) async fn get_install_id(app: AppHandle) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        return Err("Secure installation identity is not available on Android yet".into());
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
        crate::data::credentials::save(&app, crate::data::store::LEGAL_INSTALL_SECRET, &secret)?;
        let id = uuid::Uuid::new_v4().to_string();
        settings.save_value(INSTALL_ID, id.clone().into())?;
        Ok(id)
    }
}

/// Fail closed when either document changes or deletion is pending.
pub(crate) fn permits_network(settings: &SettingsSnapshot) -> bool {
    settings.get(ACCEPTED_TERMS).and_then(|v| v.as_str()) == Some(TERMS_VERSION)
        && settings.get(ACCEPTED_PRIVACY).and_then(|v| v.as_str()) == Some(PRIVACY_VERSION)
        && !settings
            .get(DELETE_PENDING)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    }
}
