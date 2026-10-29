use super::*;

#[tauri::command]
pub async fn get_contexts(app: AppHandle) -> Result<Vec<db::Context>, String> {
    run_db(&app, "get_contexts", db::query_contexts).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_context(
    app: AppHandle,
    name: String,
    icon: Option<String>,
    tone: Option<String>,
    cleanup_intensity: Option<String>,
    custom_instructions: Option<String>,
    contextual_formatting_disabled: Option<bool>,
    contextual_formatting_override: Option<bool>,
    paste_in_chunks: Option<bool>,
) -> Result<db::Context, String> {
    run_db(&app, "create_context", move |db| {
        db::insert_context_with_override_and_delivery(
            db,
            &name,
            icon.as_deref(),
            tone.as_deref(),
            cleanup_intensity.as_deref(),
            custom_instructions.as_deref(),
            contextual_formatting_override.or_else(|| contextual_formatting_disabled.filter(|disabled| *disabled)),
            paste_in_chunks.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn duplicate_context(app: AppHandle, context_id: i64) -> Result<db::Context, String> {
    run_db(&app, "duplicate_context", move |db| db::duplicate_context(db, context_id))
        .await
}

#[tauri::command]
pub async fn set_context_t3_skill_mentions(
    app: AppHandle,
    context_id: i64,
    enabled: bool,
) -> Result<(), String> {
    run_db(&app, "set_context_t3_skill_mentions", move |db| {
        db::update_context_t3_skill_mentions(db, context_id, enabled)
    })
    .await
}

#[tauri::command]
pub async fn update_context(app: AppHandle, context_id: i64, name: String) -> Result<(), String> {
    run_db(&app, "update_context", move |db| db::update_context(db, context_id, &name))
        .await
}

#[tauri::command]
pub async fn save_context_if_unchanged(app: AppHandle, original: db::Context, updated: db::Context) -> Result<db::Context, String> {
    run_db(&app, "save_context_if_unchanged", move |db| db::save_context_if_unchanged(db, original, updated)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_context_settings(
    app: AppHandle,
    context_id: i64,
    icon: Option<String>,
    tone: Option<String>,
    cleanup_intensity: Option<String>,
    custom_instructions: Option<String>,
    contextual_formatting_disabled: Option<bool>,
    contextual_formatting_override: Option<bool>,
    paste_in_chunks: Option<bool>,
    expected: Option<serde_json::Value>,
    name: Option<String>,
    editor_patch: Option<serde_json::Value>,
) -> Result<(), String> {
    run_db(&app, "update_context_settings", move |db| {
        if let Some(expected) = expected {
            let mut patch = serde_json::json!({
                "icon": icon, "tone": tone, "cleanup_intensity": cleanup_intensity,
                "custom_instructions": custom_instructions,
                "contextual_formatting_override": contextual_formatting_override.or_else(|| contextual_formatting_disabled.filter(|disabled| *disabled)),
            });
            if let Some(paste) = paste_in_chunks {
                patch["paste_in_chunks"] = serde_json::json!(paste);
            }
            if let Some(name) = name {
                patch["name"] = serde_json::json!(name);
            }
            if let Some(editor_patch) = editor_patch {
                let fields = editor_patch.as_object().ok_or_else(|| anyhow::anyhow!("editorPatch must be an object"))?;
                for (key, value) in fields {
                    if !["color", "t3_skill_mentions_disabled"].contains(&key.as_str()) {
                        return Err(anyhow::anyhow!("Unknown editorPatch field"));
                    }
                    patch[key] = value.clone();
                }
            }
            db::patch_context_settings_extended(db, context_id, expected, patch)?;
            return Ok(());
        }
        if name.is_some() || editor_patch.is_some() {
            return Err(anyhow::anyhow!("Updating the name requires an expected Context"));
        }
        db::update_context_settings_with_override_and_delivery(
            db,
            context_id,
            icon.as_deref(),
            tone.as_deref(),
            cleanup_intensity.as_deref(),
            custom_instructions.as_deref(),
            contextual_formatting_override.or_else(|| contextual_formatting_disabled.filter(|disabled| *disabled)),
            paste_in_chunks,
        )
    })
    .await
}

#[tauri::command]
pub async fn update_context_color(
    app: AppHandle,
    context_id: i64,
    color: Option<String>,
) -> Result<(), String> {
    run_db(&app, "update_context_color", move |db| db::update_context_color(db, context_id, color.as_deref()))
        .await
}

#[tauri::command]
pub async fn get_context_stats(
    app: AppHandle,
    context_id: i64,
) -> Result<db::ContextStats, String> {
    run_db(&app, "get_context_stats", move |db| db::query_context_stats(db, context_id))
        .await
}

#[tauri::command]
pub async fn set_context_pinned(
    app: AppHandle,
    context_id: i64,
    pinned: bool,
) -> Result<(), String> {
    run_db(&app, "set_context_pinned", move |db| db::set_context_pinned(db, context_id, pinned))
        .await
}

#[tauri::command]
pub async fn delete_context(app: AppHandle, context_id: i64) -> Result<(), String> {
    run_db(&app, "delete_context", move |db| db::delete_context(db, context_id))
        .await
}

#[tauri::command]
pub async fn get_context_targets(
    app: AppHandle,
    context_id: Option<i64>,
) -> Result<Vec<db::ContextTarget>, String> {
    run_db(&app, "get_context_targets", move |db| {
        let (installed_apps, refreshed) =
            crate::system::apps::list_installed_apps_cached_with_status();
        if refreshed {
            db::reconcile_context_targets(db, &installed_apps)?;
        }
        db::query_context_targets(db, context_id)
    })
    .await
}

#[tauri::command]
pub async fn assign_context_target(
    app: AppHandle,
    context_id: i64,
    executable: String,
    app_name: Option<String>,
    developer: Option<String>,
) -> Result<db::ContextTarget, String> {
    run_db(&app, "assign_context_target", move |db| {
        db::assign_context_target_with_metadata(
            db,
            context_id,
            &executable,
            app_name.as_deref(),
            developer.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn remove_context_target(
    app: AppHandle,
    context_id: i64,
    executable: String,
) -> Result<(), String> {
    run_db(&app, "remove_context_target", move |db| db::remove_context_target(db, context_id, &executable))
        .await
}

#[tauri::command]
pub async fn get_context_websites(
    app: AppHandle,
    context_id: Option<i64>,
) -> Result<Vec<db::ContextWebsiteTarget>, String> {
    run_db(&app, "get_context_websites", move |db| db::query_context_website_targets(db, context_id))
        .await
}

#[tauri::command]
pub async fn check_domain_exists(domain: String) -> Result<bool, String> {
    let host = domain.trim().to_string();
    if host.is_empty() {
        return Ok(false);
    }
    let lookup = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (host.as_str(), 443u16)
            .to_socket_addrs()
            .map(|mut addrs| addrs.next().is_some())
            .unwrap_or(false)
    });
    match tokio::time::timeout(std::time::Duration::from_secs(4), lookup).await {
        Ok(Ok(exists)) => Ok(exists),
        _ => Ok(false),
    }
}

#[tauri::command]
pub async fn assign_context_website(
    app: AppHandle,
    context_id: i64,
    domain: String,
) -> Result<db::ContextWebsiteTarget, String> {
    run_db(&app, "assign_context_website", move |db| db::assign_context_website(db, context_id, &domain))
        .await
}

#[tauri::command]
pub async fn remove_context_website(
    app: AppHandle,
    context_id: i64,
    domain: String,
) -> Result<(), String> {
    run_db(&app, "remove_context_website", move |db| db::remove_context_website(db, context_id, &domain))
        .await
}

#[tauri::command]
pub async fn get_context_dictionary(
    app: AppHandle,
    context_id: i64,
) -> Result<Vec<db::DictionaryEntry>, String> {
    run_db(&app, "get_context_dictionary", move |db| db::query_effective_dictionary_for_context(db, context_id))
        .await
}

#[tauri::command]
pub async fn get_context_snippets(
    app: AppHandle,
    context_id: i64,
) -> Result<Vec<db::Snippet>, String> {
    run_db(&app, "get_context_snippets", move |db| db::query_effective_snippets_for_context(db, context_id))
        .await
}

#[tauri::command]
pub async fn get_dictionary_entry_contexts(
    app: AppHandle,
    term: String,
) -> Result<Vec<db::ContextAssignment>, String> {
    run_db(&app, "get_dictionary_entry_contexts", move |db| db::query_dictionary_entry_contexts(db, &term))
        .await
}

#[tauri::command]
pub async fn get_snippet_entry_contexts(
    app: AppHandle,
    trigger: String,
) -> Result<Vec<db::ContextAssignment>, String> {
    run_db(&app, "get_snippet_entry_contexts", move |db| db::query_snippet_entry_contexts(db, &trigger))
        .await
}

#[tauri::command]
pub async fn move_dictionary_entry_by_term_to_context(
    app: AppHandle,
    term: String,
    context_id: i64,
) -> Result<db::DictionaryEntry, String> {
    run_db(&app, "move_dictionary_entry_by_term_to_context", move |db| db::move_dictionary_entry_by_term_to_context(db, &term, context_id))
        .await
}

#[tauri::command]
pub async fn move_snippet_entry_to_context(
    app: AppHandle,
    trigger: String,
    context_id: i64,
    edit: Option<db::SnippetMoveEdit>,
) -> Result<db::Snippet, String> {
    run_db(&app, "move_snippet_entry_to_context", move |db| db::move_snippet_entry_to_context(db, &trigger, context_id, edit.as_ref()))
        .await
}

#[tauri::command]
pub async fn set_dictionary_context_assignment(
    app: AppHandle,
    context_id: i64,
    dictionary_id: i64,
    assigned: bool,
) -> Result<(), String> {
    let result = run_db(&app, "set_dictionary_context_assignment", move |db| db::set_dictionary_context_assignment(db, context_id, dictionary_id, assigned)).await;
    if result.is_ok() { let _ = app.emit("verenu:dictionary-updated", serde_json::json!({"context_id": context_id, "dictionary_id": dictionary_id})); }
    result
}

#[tauri::command]
pub async fn set_snippet_context_assignment(
    app: AppHandle,
    context_id: i64,
    snippet_id: i64,
    assigned: bool,
) -> Result<(), String> {
    let result = run_db(&app, "set_snippet_context_assignment", move |db| db::set_snippet_context_assignment(db, context_id, snippet_id, assigned)).await;
    if result.is_ok() { let _ = app.emit("verenu:snippets-updated", ()); }
    result
}
