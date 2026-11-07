use super::*;
use crate::{
    api::t3,
    system::{
        t3_dictionary,
        t3_skills::{merged_catalog, Catalog, Skill, MIN_T3_VERSION},
    },
};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

fn operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// What Verenu last shared into T3's shared dictionary.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DictionarySync {
    /// Terms Verenu wrote, so a later sync can replace them without touching
    /// entries added in T3.
    pub synced_terms: Vec<String>,
    pub fingerprint: String,
    pub synced_at: u64,
    pub attempted_at: u64,
    pub skipped: usize,
    pub error: Option<String>,
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
    pub selected_catalog: Option<String>,
    pub catalogs: Vec<Catalog>,
    pub error: Option<String>,
    #[serde(default)]
    pub excluded_skills: Vec<String>,
    /// Scopes T3 granted to the stored credential.
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub dictionary: DictionarySync,
}

impl Connection {
    pub fn can_share_dictionary(&self) -> bool {
        self.scopes.iter().any(|scope| scope == t3::SETTINGS_SCOPE)
    }
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

fn dictionary_due(connection: &Connection, fingerprint: &str, timestamp: u64, force: bool) -> bool {
    if !connection.can_share_dictionary() {
        return false;
    }
    let state = &connection.dictionary;
    let retry_gap = if state.error.is_some() { 900 } else { 30 };
    force
        || (timestamp.saturating_sub(state.attempted_at) >= retry_gap
            && (state.fingerprint != fingerprint
                || timestamp.saturating_sub(state.synced_at) >= 86400))
}

/// Everything in Verenu's dictionary, newest first, shaped for T3.
async fn wanted_dictionary(app: &AppHandle) -> Result<Vec<t3_dictionary::Entry>, String> {
    let entries = run_db(app, "t3_dictionary", db::query_dictionary).await?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let mistakes: Vec<&str> = entry
                .corrections
                .iter()
                .map(|correction| correction.mistake.as_str())
                .chain(entry.mistake.as_deref())
                .flat_map(db::dictionary_mistake_variants)
                .collect();
            t3_dictionary::entry(&entry.term, mistakes)
        })
        .collect())
}

fn record_dictionary_result(
    state: &mut DictionarySync,
    fingerprint: &str,
    result: Result<t3::DictionarySync, String>,
    timestamp: u64,
) {
    state.attempted_at = timestamp;
    match result {
        Ok(synced) => {
            state.synced_terms = synced.synced_terms;
            state.skipped = synced.skipped;
            state.fingerprint = fingerprint.to_string();
            state.synced_at = timestamp;
            state.error = None;
        }
        Err(error) => state.error = Some(error),
    }
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
    let mut catalog = merged_catalog(&connection.environment_id, &connection.catalogs)?;
    catalog.skills.retain(|skill| !is_excluded(&connection, &skill.name));
    if catalog.skills.is_empty() {
        return None;
    }
    Some(std::sync::Arc::new(catalog))
}

fn is_excluded(connection: &Connection, name: &str) -> bool {
    connection
        .excluded_skills
        .iter()
        .any(|excluded| excluded.eq_ignore_ascii_case(name))
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
    let _consent = crate::legal_consent::network_permit().await.map_err(|error| error.to_string())?;
    if cfg!(target_os = "android") {
        return Err("T3 skill pairing requires desktop Verenu.".into());
    }
    if crate::is_dev_session() {
        return Err("Pair T3 from the installed desktop app. Browser test sessions cannot write native credentials.".into());
    }
    let target = t3::parse_pairing_link(&pairing_link)?;
    let guard = operation_lock().lock().await;
    let descriptor = t3::descriptor(&target.base_url).await?;
    let environment = descriptor
        .get("environmentId")
        .and_then(serde_json::Value::as_str)
        .ok_or("T3 did not identify its environment.")?;
    let exchange = t3::exchange(&target).await?;
    let token = exchange.token.clone();
    let catalogs = t3::fetch_catalogs(&target.base_url, &token, environment, true).await?;
    let settings = store::settings_handle(&app)?;
    let previous_snapshot = settings.snapshot()?;
    let previous_value = previous_snapshot
        .get(store::T3_CONNECTION)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let previous = connection(&previous_snapshot);
    let mut dictionary = previous
        .as_ref()
        .filter(|old| old.environment_id == environment)
        .map(|old| old.dictionary.clone())
        .unwrap_or_default();
    if exchange.scopes.iter().any(|scope| scope == t3::SETTINGS_SCOPE) {
        let result = match wanted_dictionary(&app).await {
            Ok(wanted) => {
                let fingerprint = t3_dictionary::fingerprint(&wanted);
                let synced =
                    t3::sync_dictionary(&target.base_url, &token, &dictionary.synced_terms, &wanted)
                        .await;
                (fingerprint, synced)
            }
            Err(error) => (String::new(), Err(error)),
        };
        record_dictionary_result(&mut dictionary, &result.0, result.1, now());
    }
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
        expires_at: now().saturating_add(exchange.expires_in),
        fetched_at: now(),
        attempted_at: now(),
        selected_catalog: selected,
        catalogs,
        error: None,
        excluded_skills: previous
            .as_ref()
            .filter(|old| old.environment_id == environment)
            .map(|old| old.excluded_skills.clone())
            .unwrap_or_default(),
        scopes: exchange.scopes,
        dictionary,
    };
    let value = serde_json::to_value(&current).map_err(|_| "Cannot save T3 connection.")?;
    let app_copy = app.clone();
    run_blocking("commit_t3_pairing", move || {
        let _guard = guard;
        let previous_token = crate::data::credentials::get(store::T3_INTEGRATION);
        commit_pairing(
            &settings,
            value,
            previous_value,
            &token,
            &previous_token,
            |value| crate::data::credentials::save(&app_copy, store::T3_INTEGRATION, value),
        )
    })
    .await
}

fn commit_pairing(
    settings: &store::SettingsHandle,
    value: serde_json::Value,
    previous_value: serde_json::Value,
    token: &str,
    previous_token: &str,
    mut save_credential: impl FnMut(&str) -> Result<(), String>,
) -> Result<Status, String> {
    save_credential(token)?;
    if let Err(error) = settings
        .set(store::T3_CONNECTION, value)
        .and_then(|_| settings.save())
    {
        settings.set(store::T3_CONNECTION, previous_value)?;
        save_credential(previous_token)?;
        return Err(error);
    }
    Ok(status(&settings.snapshot()?))
}

#[tauri::command]
pub async fn pull_t3_skills(app: AppHandle, force: Option<bool>) -> Result<Status, String> {
    let _consent = crate::legal_consent::network_permit().await.map_err(|error| error.to_string())?;
    let _guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let Some(mut current) = connection(&settings.snapshot()?) else {
        return Ok(status(&settings.snapshot()?));
    };
    let force = force.unwrap_or(false);
    let skills_due = refresh_due(&current, now(), force);
    let wanted = if current.can_share_dictionary() {
        wanted_dictionary(&app).await.ok()
    } else {
        None
    };
    let fingerprint = wanted.as_deref().map(t3_dictionary::fingerprint);
    let dictionary_due = fingerprint
        .as_deref()
        .is_some_and(|fingerprint| dictionary_due(&current, fingerprint, now(), force));
    if !skills_due && !dictionary_due {
        return Ok(status(&settings.snapshot()?));
    }
    let timestamp = now();
    if skills_due {
        current.attempted_at = timestamp;
    }
    let access = async {
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
        Ok((base, token))
    }
    .await;
    if skills_due {
        let result = match &access {
            Ok((base, token)) => {
                t3::fetch_catalogs(base, token, &current.environment_id, true).await
            }
            Err(error) => Err(error.clone()),
        };
        apply_refresh_result(&mut current, result, timestamp);
    }
    if dictionary_due {
        let (fingerprint, wanted) = (fingerprint.unwrap_or_default(), wanted.unwrap_or_default());
        let result = match &access {
            Ok((base, token)) => {
                t3::sync_dictionary(base, token, &current.dictionary.synced_terms, &wanted).await
            }
            Err(error) => Err(error.clone()),
        };
        record_dictionary_result(&mut current.dictionary, &fingerprint, result, timestamp);
    }
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
pub async fn set_t3_excluded_skills(
    app: AppHandle,
    names: Vec<String>,
) -> Result<Status, String> {
    let _guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let mut current = connection(&settings.snapshot()?).ok_or("Connect T3 first.")?;
    let mut excluded: Vec<String> = Vec::new();
    for name in names {
        if !excluded.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
            excluded.push(name);
        }
    }
    excluded.truncate(crate::system::t3_skills::MAX_SKILLS);
    current.excluded_skills = excluded;
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
    let guard = operation_lock().lock().await;
    let settings = store::settings_handle(&app)?;
    let transaction_settings = settings.clone();
    run_blocking("delete_t3_credential", move || {
        let _guard = guard;
        commit_disconnect(&transaction_settings, || {
            crate::data::credentials::delete(store::T3_INTEGRATION)
        })?;
        Ok(status(&transaction_settings.snapshot()?))
    })
    .await
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
            excluded_skills: Vec::new(),
            scopes: vec![t3::READ_SCOPE.into(), t3::SETTINGS_SCOPE.into()],
            dictionary: DictionarySync::default(),
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
    fn t3_dictionary_sync_needs_settings_scope_and_obeys_change_and_retry_intervals() {
        let mut value = sample();
        let time = 1_000_000;
        assert!(dictionary_due(&value, "a", time, false));
        value.scopes = vec![t3::READ_SCOPE.into()];
        assert!(!dictionary_due(&value, "a", time, true));
        value.scopes.push(t3::SETTINGS_SCOPE.into());

        record_dictionary_result(
            &mut value.dictionary,
            "a",
            Ok(t3::DictionarySync { synced_terms: vec!["Omarchy".into()], skipped: 2 }),
            time,
        );
        assert_eq!(value.dictionary.synced_terms, ["Omarchy"]);
        assert_eq!(value.dictionary.skipped, 2);
        assert!(!dictionary_due(&value, "a", time + 60, false));
        assert!(!dictionary_due(&value, "b", time + 29, false));
        assert!(dictionary_due(&value, "b", time + 30, false));
        assert!(dictionary_due(&value, "a", time + 86400, false));
        assert!(dictionary_due(&value, "a", time, true));

        record_dictionary_result(&mut value.dictionary, "b", Err("down".into()), time + 100);
        assert_eq!(value.dictionary.error.as_deref(), Some("down"));
        assert_eq!(value.dictionary.synced_terms, ["Omarchy"]);
        assert_eq!(value.dictionary.fingerprint, "a");
        assert!(!dictionary_due(&value, "b", time + 999, false));
        assert!(dictionary_due(&value, "b", time + 1000, false));
    }

    #[test]
    fn t3_connection_saved_before_dictionary_sharing_loads_without_scopes() {
        let mut json = serde_json::to_value(sample()).unwrap();
        json.as_object_mut().unwrap().remove("scopes");
        json.as_object_mut().unwrap().remove("dictionary");
        let loaded: Connection = serde_json::from_value(json).unwrap();
        assert!(!loaded.can_share_dictionary());
        assert!(loaded.dictionary.synced_terms.is_empty());
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
    fn t3_excluded_skills_are_hidden_from_captured_catalog_but_kept_in_status() {
        let mut value = sample();
        value.catalogs[0].skills.push(Skill {
            name: "file-pr".into(),
            display_name: None,
            description: None,
        });
        value.excluded_skills = vec!["BABYSIT-PR".into()];
        let captured = captured_catalog(&snapshot(&value)).unwrap();
        assert_eq!(captured.skills.len(), 1);
        assert_eq!(captured.skills[0].name, "file-pr");
        assert_eq!(status(&snapshot(&value)).skills.len(), 2);
        value.excluded_skills.push("file-pr".into());
        assert!(captured_catalog(&snapshot(&value)).is_none());
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

    #[test]
    fn t3_pairing_restores_credential_when_settings_cannot_be_saved() {
        let blocker = std::env::temp_dir().join(format!("verenu-t3-{}", uuid::Uuid::new_v4()));
        std::fs::write(&blocker, b"synthetic fixture").unwrap();
        let settings = store::SettingsHandle::empty_for_test(blocker.join("settings.json"));
        let previous = serde_json::to_value(sample()).unwrap();
        settings
            .set(store::T3_CONNECTION, previous.clone())
            .unwrap();
        let mut credential = "old synthetic token".to_string();
        let result = commit_pairing(
            &settings,
            serde_json::json!({"id": "replacement"}),
            previous.clone(),
            "new synthetic token",
            "old synthetic token",
            |value| {
                credential = value.to_string();
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(credential, "old synthetic token");
        assert_eq!(settings.get(store::T3_CONNECTION), Some(previous));
        std::fs::remove_file(blocker).unwrap();
    }

    #[test]
    fn t3_pairing_saves_credential_and_connection() {
        let path = std::env::temp_dir().join(format!("verenu-t3-{}.json", uuid::Uuid::new_v4()));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        let value = serde_json::to_value(sample()).unwrap();
        let mut credential = "old synthetic token".to_string();
        let result = commit_pairing(
            &settings,
            value.clone(),
            serde_json::Value::Null,
            "new synthetic token",
            "old synthetic token",
            |token| {
                credential = token.to_string();
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(credential, "new synthetic token");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved[store::T3_CONNECTION], value);
        std::fs::remove_file(path).unwrap();
    }
}
