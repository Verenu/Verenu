use super::*;

pub(super) fn clear_changed_custom_provider_keys(
    app: &AppHandle,
    previous: &store::SettingsSnapshot,
    next: &[crate::api::custom::CustomProvider],
) -> Result<(), String> {
    if !crate::api::custom::native_credentials_available() {
        return Ok(());
    }
    clear_changed_provider_keys(previous, next, |id| {
        crate::data::credentials::delete_saved(app, id)
    })
}

pub(super) fn clear_changed_provider_keys(
    previous: &store::SettingsSnapshot,
    next: &[crate::api::custom::CustomProvider],
    mut delete: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    for old in crate::api::custom::parse_stored(previous.get(store::CUSTOM_PROVIDERS)) {
        if next.iter().find(|p| p.id == old.id).is_none_or(|p| {
            old.base_url != p.base_url
                || old.protocol != p.protocol
                || old.auth_header != p.auth_header
        }) {
            delete(&old.id)?;
        }
    }
    Ok(())
}

/// Remove the endpoint and its model references together. Credential deletion
/// happens first so a failed keychain operation leaves a retryable definition.
#[tauri::command]
pub async fn delete_custom_provider(app: AppHandle, provider: String) -> Result<(), String> {
    if !crate::api::custom::is_custom_id(&provider) {
        return Err("Invalid custom provider id.".into());
    }
    let settings = store::settings_handle(&app)?;
    run_blocking("delete_custom_provider", move || {
        let snapshot = settings.snapshot()?;
        let mut providers = crate::api::custom::parse_stored(snapshot.get(store::CUSTOM_PROVIDERS));
        if !providers.iter().any(|p| p.id == provider) {
            return Err("This provider was already removed.".into());
        }
        providers.retain(|p| p.id != provider);
        let mut changes = vec![(
            store::CUSTOM_PROVIDERS.to_string(),
            serde_json::json!(providers),
        )];
        for (default_key, fallback_key, map_key, provider_key, legacy_key, fallback) in [
            (
                store::TRANSCRIPTION_DEFAULT_MODEL,
                store::TRANSCRIPTION_FALLBACK_MODELS,
                store::TRANSCRIPTION_MODELS_BY_PROVIDER,
                store::TRANSCRIPTION_PROVIDER,
                store::TRANSCRIPTION_MODEL,
                "groq/whisper-large-v3-turbo",
            ),
            (
                store::CLEANUP_DEFAULT_MODEL,
                store::CLEANUP_FALLBACK_MODELS,
                store::CLEANUP_MODELS_BY_PROVIDER,
                store::CLEANUP_PROVIDER,
                store::CLEANUP_MODEL,
                "groq/qwen/qwen3.8-27b",
            ),
        ] {
            let belongs = |id: &str| store::parse_model_id(id).is_some_and(|(p, _)| p == provider);
            let mut fallbacks: Vec<String> = snapshot
                .get(fallback_key)
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .filter(|id| !belongs(id))
                .map(str::to_string)
                .collect();
            if snapshot
                .get(default_key)
                .and_then(|v| v.as_str())
                .is_some_and(belongs)
            {
                let next = if fallbacks.is_empty() {
                    fallback.to_string()
                } else {
                    fallbacks.remove(0)
                };
                if let Some((p, m)) = store::parse_model_id(&next) {
                    changes.push((provider_key.to_string(), serde_json::json!(p)));
                    changes.push((legacy_key.to_string(), serde_json::json!(m)));
                }
                changes.push((default_key.to_string(), serde_json::json!(next)));
            }
            changes.push((fallback_key.to_string(), serde_json::json!(fallbacks)));
            if let Some(mut map) = snapshot.get(map_key).and_then(|v| v.as_object()).cloned() {
                map.remove(&provider);
                changes.push((map_key.to_string(), serde_json::json!(map)));
            }
        }
        settings
            .save_values_with_commit(
                changes,
                |previous| clear_changed_custom_provider_keys(&app, previous, &providers),
                || Ok(()),
            )
            .map(|_| ())
    })
    .await
}
