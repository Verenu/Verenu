use super::*;
use crate::{
    api::t3,
    system::t3_skills::{merged_catalog, Catalog, Skill, MIN_T3_VERSION},
};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

fn operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub base_url: String,
    pub environment_id: String,
    pub label: String,
    pub version: String,
    pub expires_at: u64,
    pub fetched_at: u64,
    pub attempted_at: u64,
    /// Legacy setting retained for existing connections; shared skills ignore it.
    pub selected_catalog: Option<String>,
    pub catalogs: Vec<Catalog>,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub minimum_version: &'static str,
    pub connection: Option<Connection>,
    pub skills: Vec<Skill>,
}

fn now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

fn refresh_due(connection: &Connection, timestamp: u64, force: bool) -> bool {
    force
        || (timestamp.saturating_sub(connection.fetched_at) >= 86400
            && timestamp.saturating_sub(connection.attempted_at) >= 900)
}

fn connection(settings: &store::SettingsSnapshot) -> Option<Connection> {
    settings
        .get(store::T3_CONNECTION)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn status(settings: &store::SettingsSnapshot) -> Status {
    let connection = connection(settings);
    let skills = connection
        .as_ref()
        .and_then(|value| merged_catalog(&value.environment_id, &value.catalogs))
        .map(|catalog| catalog.skills)
        .unwrap_or_default();
    Status {
        minimum_version: MIN_T3_VERSION,
        connection,
        skills,
    }
}

pub fn captured_catalog(settings: &store::SettingsSnapshot) -> Option<std::sync::Arc<Catalog>> {
    let connection = connection(settings)?;
    // Credential expiry controls network access, not use of metadata already
    // imported. Keep the last-good catalog until replaced or disconnected.
    merged_catalog(&connection.environment_id, &connection.catalogs).map(std::sync::Arc::new)
}

pub fn catalog_for_destination(
    settings: &store::SettingsSnapshot,
    executable: &str,
    disabled: bool,
) -> Option<std::sync::Arc<Catalog>> {
    let t3_destination = crate::system::t3_skills::is_t3_app(executable);
    if t3_destination && disabled {
        log::info!("t3 skills: capture enabled=false catalog=false");
    }
    if disabled || !t3_destination {
        log::debug!(
            "t3 skills: skipped destination_t3={t3_destination} context_disabled={disabled}"
        );
        return None;
    }
    let catalog = captured_catalog(settings);
    // Useful with default logging; no app identities or skill names are logged.
    log::info!(
        "t3 skills: capture enabled=true catalog={}",
        catalog.is_some()
    );
    catalog
}

#[tauri::command]
pub async fn get_t3_skills(app: AppHandle) -> Result<Status, String> {
    Ok(status(&store::settings_snapshot(&app)?))
}

#[tauri::command]
pub async fn connect_t3(app: AppHandle, pairing_link: String) -> Result<Status, String> {
    if cfg!(target_os = "android") {
        return Err("T3 skill pairing requires desktop Verenu.".into());
    }
    if crate::is_dev_session() {
        return Err("Pair T3 from the installed desktop app. Browser test sessions cannot write native credentials.".into());
    }
    let target = t3::parse_pairing_link(&pairing_link)?;
    let _guard = operation_lock().lock().await;
    let descriptor = t3::descriptor(&target.base_url).await?;
    let environment = descriptor
        .get("environmentId")
        .and_then(serde_json::Value::as_str)
        .ok_or("T3 did not identify its environment.")?;
    let (token, expires_in) = t3::exchange(&target).await?;
    let catalogs = t3::fetch_catalogs(&target.base_url, &token, environment, true).await?;
    let settings = store::settings_handle(&app)?;
    let previous_snapshot = settings.snapshot()?;
    let previous_value = previous_snapshot
        .get(store::T3_CONNECTION)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let previous = connection(&previous_snapshot);
    let selected = previous
        .as_ref()
        .filter(|old| old.environment_id == environment)
        .and_then(|old| old.selected_catalog.clone())
        .filter(|id| catalogs.iter().any(|catalog| catalog.id == *id))
        .or_else(|| (catalogs.len() == 1).then(|| catalogs[0].id.clone()));
    let current = Connection {
        id: uuid::Uuid::new_v4().to_string(),
        base_url: target.base_url.to_string(),
        environment_id: environment.to_string(),
        label: descriptor
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("T3 Code")
            .to_string(),
        version: descriptor
            .get("serverVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        expires_at: now().saturating_add(expires_in),
        fetched_at: now(),
        attempted_at: now(),
        selected_catalog: selected,
        catalogs,
        error: None,
    };
    let value = serde_json::to_value(&current).map_err(|_| "Cannot save T3 connection.")?;
    let previous_token = run_blocking("read_previous_t3_credential", || {
        Ok(crate::data::credentials::get(store::T3_INTEGRATION))
    })
    .await?;
    let app_copy = app.clone();
    run_blocking("save_t3_credential", move || {
        crate::data::credentials::save(&app_copy, store::T3_INTEGRATION, &token)
    })
    .await?;
    if let Err(error) = settings
        .set(store::T3_CONNECTION, value)
        .and_then(|_| settings.save())
    {
        settings.set(store::T3_CONNECTION, previous_value)?;
        let app_copy = app.clone();
        run_blocking("restore_t3_credential", move || {
            crate::data::credentials::save(&app_copy, store::T3_INTEGRATION, &previous_token)
        })
        .await?;
        return Err(error);
    }
    Ok(status(&settings.snapshot()?))
}

#[tauri::command]
pub async fn pull_t3_skills(app: AppHandle, force: Option<bool>) -> Result<Status, String> {
    let _guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let Some(mut current) = connection(&settings.snapshot()?) else {
        return Ok(status(&settings.snapshot()?));
    };
    if !refresh_due(&current, now(), force.unwrap_or(false)) {
        return Ok(status(&settings.snapshot()?));
    }
    current.attempted_at = now();
    let result = async {
        if current.expires_at <= now() {
            return Err("T3 pairing expired. Reconnect with a fresh pairing link.".to_string());
        }
        let token = run_blocking("read_t3_credential", || {
            let token = crate::data::credentials::get(store::T3_INTEGRATION);
            if token.is_empty() {
                Err("T3 credential is unavailable. Reconnect from the desktop app.".into())
            } else {
                Ok(token)
            }
        })
        .await?;
        let base = reqwest::Url::parse(&current.base_url)
            .map_err(|_| "Reconnect T3 with a valid pairing link.")?;
        let descriptor = t3::descriptor(&base).await?;
        if descriptor
            .get("environmentId")
            .and_then(serde_json::Value::as_str)
            != Some(current.environment_id.as_str())
        {
            return Err(
                "This address now belongs to another T3 environment. Pair it again.".into(),
            );
        }
        t3::fetch_catalogs(&base, &token, &current.environment_id, true).await
    }
    .await;
    apply_refresh_result(&mut current, result, now());
    settings.set(
        store::T3_CONNECTION,
        serde_json::to_value(&current).map_err(|_| "Cannot save T3 skills.")?,
    )?;
    settings.save()?;
    Ok(status(&settings.snapshot()?))
}

fn apply_refresh_result(
    current: &mut Connection,
    result: Result<Vec<Catalog>, String>,
    timestamp: u64,
) {
    match result {
        Ok(catalogs) => {
            current.selected_catalog = current
                .selected_catalog
                .take()
                .filter(|id| catalogs.iter().any(|catalog| catalog.id == *id))
                .or_else(|| (catalogs.len() == 1).then(|| catalogs[0].id.clone()));
            current.catalogs = catalogs;
            current.fetched_at = timestamp;
            current.error = None;
        }
        Err(error) => {
            // A failed pull never replaces data from the previously paired
            // environment. Reconnect to refresh, or disconnect to remove it.
            current.error = Some(error);
        }
    }
}

#[tauri::command]
pub async fn select_t3_catalog(app: AppHandle, catalog_id: String) -> Result<Status, String> {
    let _guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let mut current = connection(&settings.snapshot()?).ok_or("Connect T3 first.")?;
    if !current
        .catalogs
        .iter()
        .any(|catalog| catalog.id == catalog_id)
    {
        return Err("Pull skills again to choose an available catalog.".into());
    }
    current.selected_catalog = Some(catalog_id);
    settings.set(
        store::T3_CONNECTION,
        serde_json::to_value(&current).map_err(|_| "Cannot save T3 skill selection.")?,
    )?;
    settings.save()?;
    Ok(status(&settings.snapshot()?))
}

#[tauri::command]
pub async fn disconnect_t3(app: AppHandle) -> Result<Status, String> {
    if cfg!(target_os = "android") {
        return Err("T3 skill pairing requires desktop Verenu.".into());
    }
    if crate::is_dev_session() {
        return Err("Disconnect T3 from the installed desktop app.".into());
    }
    let _guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let transaction_settings = settings.clone();
    run_blocking("delete_t3_credential", move || {
        commit_disconnect(&transaction_settings, || {
            crate::data::credentials::delete(store::T3_INTEGRATION)
        })
    })
    .await?;
    Ok(status(&settings.snapshot()?))
}

fn commit_disconnect(
    settings: &store::SettingsHandle,
    delete_credential: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    settings
        .save_values_with_commit(
            [(store::T3_CONNECTION, serde_json::Value::Null)],
            |_| Ok(()),
            delete_credential,
        )
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Connection {
        Connection {
            id: "sample".into(),
            base_url: "https://synthetic.invalid/".into(),
            environment_id: "sample".into(),
            label: "Synthetic T3".into(),
            version: MIN_T3_VERSION.into(),
            expires_at: now() + 86400,
            fetched_at: now(),
            attempted_at: now(),
            selected_catalog: Some("catalog".into()),
            error: None,
            catalogs: vec![Catalog {
                environment_id: "sample".into(),
                id: "catalog".into(),
                label: "Codex".into(),
                provider_instance_id: "codex".into(),
                workspace_id: "default".into(),
                revision: "1".into(),
                skills: vec![crate::system::t3_skills::Skill {
                    name: "babysit-pr".into(),
                    display_name: None,
                    description: None,
                }],
            }],
        }
    }
    fn snapshot(value: &Connection) -> store::SettingsSnapshot {
        store::SettingsSnapshot::from_pairs([(
            store::T3_CONNECTION.into(),
            serde_json::to_value(value).unwrap(),
        )])
    }
    #[test]
    fn t3_catalog_is_scoped_to_destination_and_context_and_captured_immutably() {
        let mut value = sample();
        let captured = catalog_for_destination(&snapshot(&value), "t3code", false).unwrap();
        assert!(catalog_for_destination(&snapshot(&value), "chrome.exe", false).is_none());
        assert!(catalog_for_destination(&snapshot(&value), "t3code", true).is_none());
        value.catalogs[0].skills.clear();
        assert_eq!(captured.skills[0].name, "babysit-pr");
        assert!(captured_catalog(&snapshot(&value)).is_none());
        value = sample();
        value.expires_at = now();
        assert!(captured_catalog(&snapshot(&value)).is_some());
        value = sample();
        value.fetched_at = now() - 90 * 86400;
        assert!(captured_catalog(&snapshot(&value)).is_some());
    }
    #[test]
    fn t3_refresh_obeys_daily_success_and_failure_retry_intervals() {
        let mut value = sample();
        let time = value.fetched_at;
        assert!(!refresh_due(&value, time + 86399, false));
        assert!(refresh_due(&value, time + 86400, false));
        value.attempted_at = time + 86400;
        assert!(!refresh_due(&value, time + 86401, false));
        assert!(refresh_due(&value, time + 87300, false));
        assert!(refresh_due(&value, time, true));
    }

    #[test]
    fn t3_shared_skills_need_no_selection_and_ignore_old_selection() {
        let mut value = sample();
        let mut second = value.catalogs[0].clone();
        second.id = "other-workspace".into();
        second.provider_instance_id = "claude".into();
        second.skills.push(Skill {
            name: "skill-designer".into(),
            display_name: None,
            description: Some("Create skills".into()),
        });
        value.catalogs.push(second);
        value.selected_catalog = None;
        let captured = captured_catalog(&snapshot(&value)).unwrap();
        assert_eq!(captured.skills.len(), 2);
        assert_eq!(status(&snapshot(&value)).skills, captured.skills);
        value.selected_catalog = Some("catalog".into());
        assert_eq!(*captured_catalog(&snapshot(&value)).unwrap(), *captured);
        value.selected_catalog = Some("removed-workspace".into());
        assert_eq!(*captured_catalog(&snapshot(&value)).unwrap(), *captured);
    }

    #[test]
    fn t3_failed_refresh_preserves_saved_skills_until_success_or_disconnect() {
        for error in [
            "Cannot reach T3.",
            "T3 pairing expired.",
            "T3 credential is unavailable.",
            "This address now belongs to another T3 environment.",
            "Requires T3 Code 0.46 or newer.",
            "This T3 protocol version is not supported.",
        ] {
            let mut value = sample();
            value.expires_at = now() - 1;
            value.fetched_at = now() - 90 * 86400;
            let previous = value.catalogs.clone();
            let fetched_at = value.fetched_at;
            apply_refresh_result(&mut value, Err(error.into()), now());
            assert_eq!(value.catalogs, previous);
            assert_eq!(value.selected_catalog.as_deref(), Some("catalog"));
            assert_eq!(value.fetched_at, fetched_at);
            assert_eq!(value.error.as_deref(), Some(error));
            // Settings reload retains the cache even when its credential expired.
            assert_eq!(
                captured_catalog(&snapshot(&value)).unwrap().skills[0].name,
                "babysit-pr"
            );
            let mut updated = previous;
            updated[0].revision = "2".into();
            updated[0].skills[0].name = "file-pr".into();
            let timestamp = now();
            apply_refresh_result(&mut value, Ok(updated), timestamp);
            assert_eq!(value.fetched_at, timestamp);
            assert!(value.error.is_none());
            assert_eq!(
                captured_catalog(&snapshot(&value)).unwrap().skills[0].name,
                "file-pr"
            );
        }
        assert!(captured_catalog(&store::SettingsSnapshot::from_pairs([(
            store::T3_CONNECTION.into(),
            serde_json::Value::Null
        )]))
        .is_none());
        let mut invalid = sample();
        invalid.catalogs[0].skills[0].name = "invalid name".into();
        assert!(captured_catalog(&snapshot(&invalid)).is_none());
    }

    #[test]
    fn t3_disconnect_keeps_connection_and_cached_skills_when_credential_delete_fails() {
        let path = std::env::temp_dir().join(format!(
            "verenu_t3_disconnect_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        let previous = serde_json::to_value(sample()).unwrap();
        settings
            .save_value(store::T3_CONNECTION, previous.clone())
            .unwrap();
        let previous_bytes = std::fs::read(&path).unwrap();

        let error = commit_disconnect(&settings, || {
            Err("Synthetic credential deletion failure.".into())
        })
        .unwrap_err();

        assert_eq!(error, "Synthetic credential deletion failure.");
        assert_eq!(settings.get(store::T3_CONNECTION), Some(previous.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), previous_bytes);
        assert_eq!(
            captured_catalog(&settings.snapshot().unwrap())
                .unwrap()
                .skills[0]
                .name,
            "babysit-pr"
        );

        commit_disconnect(&settings, || Ok(())).unwrap();
        assert_eq!(
            settings.get(store::T3_CONNECTION),
            Some(serde_json::Value::Null)
        );
        assert!(captured_catalog(&settings.snapshot().unwrap()).is_none());
        let _ = std::fs::remove_file(path);
    }
}
