//! App mappings, snippets, and dictionary (the user content library).

use super::*;

// ---------- app mappings ----------

#[tauri::command]
pub async fn get_installed_apps(app: AppHandle) -> Vec<InstalledApp> {
    // Android has no executables to scan; the Kotlin plugin lists the apps
    // that appear in the launcher, keyed by package name.
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Listing {
            apps: Vec<InstalledApp>,
        }
        match crate::android::permissions_plugin::run::<_, Listing>(
            &app,
            "installedApps",
            serde_json::json!({}),
        )
        .await
        {
            Ok(listing) => listing.apps,
            Err(e) => {
                log::error!("installed apps: {e}");
                Vec::new()
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &app;
        match run_blocking("get_installed_apps", || {
            Ok(crate::system::apps::list_installed_apps())
        })
        .await
        {
            Ok(apps) => apps,
            Err(e) => {
                log::error!("{e}");
                Vec::new()
            }
        }
    }
}

/// Returns a `data:image/png;base64,...` URI for `exe`'s real icon, or
/// `None` if it couldn't be resolved/extracted — the frontend falls back to
/// a colored-initial badge in that case. Deliberately not bundled into
/// `get_installed_apps`: extraction/caching is per-icon work and lazy
/// per-row loading keeps that bulk list light.
#[tauri::command]
pub async fn get_app_icon(app: AppHandle, exe: String) -> Option<String> {
    // Android: the Kotlin plugin renders the package's launcher icon.
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Icon {
            icon: Option<String>,
        }
        crate::android::permissions_plugin::run::<_, Icon>(
            &app,
            "appIcon",
            serde_json::json!({ "packageName": exe.trim() }),
        )
        .await
        .ok()
        .and_then(|reply| reply.icon)
    }
    #[cfg(not(target_os = "android"))]
    {
        run_blocking("get_app_icon", move || {
            Ok(crate::system::icons::get_icon_data_uri(&app, &exe))
        })
        .await
        .ok()
        .flatten()
    }
}

/// Returns a `data:image/...;base64,...` URI for a website target's favicon,
/// or `None` when it couldn't be resolved — the frontend falls back to a globe
/// glyph. Results (including failures) are disk-cached per hostname, so the
/// sidebar's icon stacks don't re-fetch on every render.
#[tauri::command]
pub async fn get_site_icon(app: AppHandle, domain: String) -> Option<String> {
    crate::system::icons::get_site_icon_data_uri(&app, &domain).await
}

#[tauri::command]
pub async fn get_app_mappings(app: AppHandle) -> Result<Vec<AppMapping>, String> {
    let settings = store::settings_handle(&app)?;
    let mappings = settings
        .get(store::APP_MAPPINGS)
        .and_then(|v| serde_json::from_value::<Vec<AppMapping>>(v).ok())
        .unwrap_or_default();
    Ok(mappings)
}

#[tauri::command]
pub async fn save_app_mappings(app: AppHandle, mappings: Vec<AppMapping>) -> Result<(), String> {
    let value = serde_json::to_value(mappings).map_err(|e| e.to_string())?;
    super::validate_setting(store::APP_MAPPINGS, &value)?;
    let settings = store::settings_handle(&app)?;
    run_blocking("save_app_mappings", move || {
        settings.save_value(store::APP_MAPPINGS, value)
    })
    .await
}

// ---------- snippets ----------

#[tauri::command]
pub async fn get_snippets(app: AppHandle) -> Result<Vec<db::Snippet>, String> {
    run_db(&app, "get_snippets", move |db| {
        let rows = db::query_snippets(db)?;
        if crate::system::logger::is_verbose() {
            log::info!("snippets:get count={}", rows.len());
        }
        Ok(rows)
    })
    .await
}

#[tauri::command]
pub async fn create_snippet(
    app: AppHandle,
    trigger: String,
    expansion: String,
    instructions: String,
    context_id: Option<i64>,
) -> Result<db::CreatedRecordMeta, String> {
    run_db(&app, "create_snippet", move |db| {
        log::info!(
            "snippets:create trigger_chars={} expansion_chars={} instructions_chars={}",
            trigger.chars().count(),
            expansion.chars().count(),
            instructions.chars().count()
        );
        let created =
            db::insert_snippet_returning(db, &trigger, &expansion, &instructions, context_id)
                .map_err(|e| {
                    log::warn!("snippets:create failed: {e}");
                    e
                })?;
        log::info!("snippets:create ok id={}", created.id);
        Ok(created)
    })
    .await
}

#[tauri::command]
pub async fn edit_snippet(
    app: AppHandle,
    id: i64,
    trigger: String,
    expansion: String,
    instructions: String,
) -> Result<(), String> {
    run_db(&app, "edit_snippet", move |db| {
        db::update_snippet(db, id, &trigger, &expansion, &instructions)
    })
    .await
}

#[tauri::command]
pub async fn remove_snippet(app: AppHandle, id: i64) -> Result<(), String> {
    run_db(&app, "remove_snippet", move |db| db::delete_snippet(db, id)).await
}

// ---------- dictionary ----------

#[tauri::command]
pub async fn get_dictionary(app: AppHandle) -> Result<Vec<db::DictionaryEntry>, String> {
    run_db(&app, "get_dictionary", db::query_dictionary).await
}

#[tauri::command]
pub async fn create_dictionary_entry(
    app: AppHandle,
    term: String,
    mistake: Option<String>,
    context_id: Option<i64>,
) -> Result<db::CreatedRecordMeta, String> {
    run_db(&app, "create_dictionary_entry", move |db| {
        log::info!(
            "dictionary:create term_chars={} mistake_chars={}",
            term.chars().count(),
            mistake.as_deref().map_or(0, |m| m.chars().count())
        );
        db::insert_dictionary_entry_returning(db, &term, mistake.as_deref(), context_id).map_err(
            |e| {
                log::warn!("dictionary:create failed: {e}");
                e
            },
        )
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn edit_dictionary_entry(
    app: AppHandle,
    id: i64,
    term: String,
    mistake: Option<String>,
    dictionary_id: Option<i64>,
    context_id: Option<i64>,
    correction_id: Option<i64>,
    correction_ids: Option<Vec<i64>>,
) -> Result<(), String> {
    let event_app = app.clone();
    run_db(&app, "edit_dictionary_entry", move |db| {
        // The mapping ids are accepted for forward/backward IPC compatibility
        // and diagnostics, but the canonical row plus Context is the actual
        // edit target. The backend re-reads authoritative child mappings in a
        // transaction so stale frontend ids cannot widen the edit scope.
        let _ = (dictionary_id, correction_id, correction_ids);
        let result = match context_id {
            Some(context_id) => db::update_dictionary_entry_for_context(
                db,
                context_id,
                id,
                &term,
                mistake.as_deref(),
            ),
            None => db::update_dictionary_entry(db, id, &term, mistake.as_deref()),
        };
        if result.is_ok() {
            emit_dictionary_updated(&event_app, context_id, id);
        }
        result
    })
    .await
}

#[tauri::command]
pub async fn remove_dictionary_entry(
    app: AppHandle,
    id: i64,
    dictionary_id: Option<i64>,
    context_id: Option<i64>,
    correction_id: Option<i64>,
    correction_ids: Option<Vec<i64>>,
) -> Result<(), String> {
    let event_app = app.clone();
    run_db(&app, "remove_dictionary_entry", move |db| {
        let _ = (dictionary_id, correction_id, correction_ids);
        let result = match context_id {
            Some(context_id) => db::remove_dictionary_entry_from_context(db, context_id, id),
            None => db::delete_dictionary_entry(db, id),
        };
        if result.is_ok() {
            emit_dictionary_updated(&event_app, context_id, id);
        }
        result
    })
    .await
}

#[tauri::command]
pub async fn move_dictionary_entry_to_context(
    app: AppHandle,
    dictionary_id: i64,
    source_context_id: i64,
    target_context_id: i64,
    correction_id: Option<i64>,
    correction_ids: Option<Vec<i64>>,
) -> Result<(), String> {
    let event_app = app.clone();
    run_db(&app, "move_dictionary_entry_to_context", move |db| {
        let _ = (correction_id, correction_ids);
        db::move_dictionary_entry_to_context(
            db,
            dictionary_id,
            source_context_id,
            target_context_id,
        )?;
        emit_dictionary_updated(&event_app, Some(source_context_id), dictionary_id);
        emit_dictionary_updated(&event_app, Some(target_context_id), dictionary_id);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn get_auto_learn_status_summary(
    app: AppHandle,
) -> Result<db::AutoLearnStatusSummary, String> {
    run_db(
        &app,
        "get_auto_learn_status_summary",
        db::get_auto_learn_status_summary,
    )
    .await
}

#[tauri::command]
pub async fn get_recent_auto_learn_activity(
    app: AppHandle,
    limit: Option<i64>,
) -> Result<Vec<db::AutoLearnEvent>, String> {
    run_db(&app, "get_recent_auto_learn_activity", move |db| {
        db::get_recent_auto_learn_activity(db, limit.unwrap_or(20))
    })
    .await
}

fn emit_dictionary_updated(app: &AppHandle, context_id: Option<i64>, dictionary_id: i64) {
    let _ = app.emit(
        "verenu:dictionary-updated",
        serde_json::json!({ "context_id": context_id, "dictionary_id": dictionary_id }),
    );
}
