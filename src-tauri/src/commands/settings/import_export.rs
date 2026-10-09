use super::*;

use anyhow::Result as AnyhowResult;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use uuid::Uuid;

mod backup_file;

// Stats are included in the backup for informational reference only; they derive
// from transcription history which is not backed up and cannot be restored.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct ExportStats {
    pub total_words: i64,
    pub avg_wpm: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportDictionaryEntry {
    pub term: String,
    /// Retained for v1 compatibility. v2 stores correction mappings below the
    /// Context that owns them and always writes this as `null`.
    #[serde(default)]
    pub mistake: Option<String>,
    pub auto_learned: bool,
    pub confidence_tier: String,
    pub correction_count: i64,
    pub created_at: String,
    /// Canonical dictionary identity is useful when inspecting a backup, but
    /// v2 import still uses the unique canonical term as its natural key so a
    /// backup can be restored into a database with different integer IDs.
    #[serde(default)]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportSnippet {
    pub trigger: String,
    pub expansion: String,
    pub instructions: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportDictionaryCorrection {
    #[serde(default)]
    pub uuid: Option<String>,
    pub mistake: String,
    pub auto_learned: bool,
    pub confidence_tier: String,
    pub correction_count: i64,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportContextDictionaryEntry {
    pub term: String,
    #[serde(default)]
    pub dictionary_uuid: Option<String>,
    #[serde(default)]
    pub corrections: Vec<ExportDictionaryCorrection>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportContextTarget {
    pub executable: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub developer: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportContextWebsiteTarget {
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportSubApp {
    pub executable: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub title_pattern: String,
    #[serde(default = "default_export_match_mode")]
    pub match_mode: String,
}

fn default_export_match_mode() -> String {
    "contains".to_string()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportContext {
    /// Used only to recognize a renamed Context during import. New local
    /// Context rows receive a fresh identity; a JSON backup is not a sync
    /// transport and must not overwrite local row identities.
    #[serde(default)]
    pub uuid: Option<String>,
    pub name: String,
    #[serde(default)]
    pub is_everywhere: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub cleanup_intensity: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub custom_instructions: Option<String>,
    #[serde(default)]
    pub contextual_formatting_disabled: bool,
    #[serde(default)]
    pub paste_in_chunks: bool,
    #[serde(default)]
    pub pinned_at: Option<String>,
    #[serde(default)]
    pub dictionary: Vec<ExportContextDictionaryEntry>,
    #[serde(default)]
    pub snippets: Vec<ExportSnippet>,
    #[serde(default)]
    pub targets: Vec<ExportContextTarget>,
    #[serde(default)]
    pub website_targets: Vec<ExportContextWebsiteTarget>,
    /// App + window-title rules. Older backups omit the field.
    #[serde(default)]
    pub sub_apps: Vec<ExportSubApp>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportPayload {
    pub version: String,
    pub app_version: String,
    pub exported_at: String,
    #[serde(default, skip_deserializing)]
    pub stats: ExportStats,
    pub settings: serde_json::Value,
    #[serde(default)]
    pub dictionary: Vec<ExportDictionaryEntry>,
    #[serde(default)]
    pub snippets: Vec<ExportSnippet>,
    /// v2's Context graph. Empty for v1 backups.
    #[serde(default)]
    pub contexts: Vec<ExportContext>,
    /// Sub-apps not assigned to any Context. Assigned ones live under their
    /// Context in `contexts[].sub_apps`.
    #[serde(default)]
    pub sub_apps: Vec<ExportSubApp>,
}

#[derive(Debug, Default)]
struct LibraryImportStats {
    contexts_inserted: usize,
    contexts_skipped: usize,
    contexts_already_existed: usize,
    dictionary_inserted: usize,
    dictionary_skipped: usize,
    dictionary_already_existed: usize,
    dictionary_assignments_inserted: usize,
    dictionary_assignments_skipped: usize,
    dictionary_corrections_inserted: usize,
    dictionary_corrections_skipped: usize,
    snippets_inserted: usize,
    snippets_skipped: usize,
    snippets_already_existed: usize,
    app_targets_kept: usize,
    app_targets_matched: usize,
    app_targets_dropped: usize,
    sub_apps_imported: usize,
    sub_apps_dropped: usize,
}

struct ImportCorrection<'a> {
    context_id: i64,
    dictionary_id: i64,
    mistake: &'a str,
    auto_learned: bool,
    correction_count: i64,
    confidence_tier: &'a str,
    last_seen_at: Option<&'a str>,
    created_at: Option<&'a str>,
    source_uuid: Option<&'a str>,
}

type ExistingCorrection = (i64, Option<String>, bool, i64, String, Option<String>);

#[derive(serde::Serialize)]
pub struct ImportSummary {
    pub settings_applied: usize,
    pub settings_skipped: usize,
    pub contexts_inserted: usize,
    pub contexts_skipped: usize,
    pub contexts_already_existed: usize,
    pub dictionary_inserted: usize,
    pub dictionary_skipped: usize,
    pub dictionary_already_existed: usize,
    pub dictionary_assignments_inserted: usize,
    pub dictionary_assignments_skipped: usize,
    pub dictionary_corrections_inserted: usize,
    pub dictionary_corrections_skipped: usize,
    pub snippets_inserted: usize,
    pub snippets_skipped: usize,
    pub snippets_already_existed: usize,
    /// App targets restored as-is (native to this platform or installed).
    pub app_targets_kept: usize,
    /// Targets from another platform/device rebound to the closest local app.
    pub app_targets_matched: usize,
    /// Targets with no local match; left out so the user can add them later.
    pub app_targets_dropped: usize,
    pub sub_apps_imported: usize,
    /// Sub-apps whose app is not installed here; left out like app targets.
    pub sub_apps_dropped: usize,
}

#[tauri::command]
pub async fn export_data(
    app: AppHandle,
    db: tauri::State<'_, crate::DbHandle>,
) -> Result<String, String> {
    let db = db.inner().clone();
    run_blocking("export_data", move || {
        let settings = store::settings_snapshot(&app)?;
        let mut settings_map = serde_json::Map::new();
        for key in exportable_setting_keys() {
            if let Some(value) = settings.get_cloned(key) {
                settings_map.insert(key.to_string(), value);
            }
        }

        let stats = db::query_stats(&db).map_err(|e| e.to_string())?;
        let (dictionary, contexts) = export_contextual_library(&db).map_err(|e| e.to_string())?;

        let now = chrono::Local::now();
        let payload = ExportPayload {
            version: "2".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at: now.to_rfc3339(),
            stats: ExportStats {
                total_words: stats.total_words,
                avg_wpm: stats.avg_wpm,
            },
            settings: serde_json::Value::Object(settings_map),
            dictionary,
            // v2 stores snippets under their owning Context, just like
            // context-specific correction mappings. Keeping this top-level
            // vector empty prevents a shared snippet from being broadened to
            // Everywhere during a restore.
            snippets: Vec::new(),
            contexts,
            sub_apps: export_unassigned_sub_apps(&db).map_err(|e| e.to_string())?,
        };

        let json = serde_json::to_string_pretty(&payload)
            .map_err(|e| format!("Serialization failed: {e}"))?;

        let downloads = app
            .path()
            .download_dir()
            .map_err(|e| format!("Failed to resolve Downloads directory: {e}"))?;
        std::fs::create_dir_all(&downloads)
            .map_err(|e| format!("Failed to create Downloads path: {e}"))?;
        let path = downloads.join(format!(
            "verenu-backup-{}.json",
            now.format("%Y%m%d-%H%M%S")
        ));
        backup_file::write_backup(&path, json.as_bytes())
            .map_err(|e| format!("Failed to write backup file: {e}"))?;

        let path_label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("verenu-backup.json");
        log::info!("export_data: wrote backup_file={path_label}");
        Ok(path.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    db: tauri::State<'_, crate::DbHandle>,
    json: String,
) -> Result<ImportSummary, String> {
    let db = db.inner().clone();
    run_blocking("import_data", move || {
        with_import_payload(&json, |payload| {
            let settings = store::settings_handle(&app)?;
            let installed_apps = crate::system::apps::list_installed_apps_cached_with_status().0;
            import_payload(
                &settings,
                &db,
                payload,
                &installed_apps,
                |providers| clear_changed_custom_provider_keys(&app, providers),
                |effects| {
                    if effects.runtime_icons {
                        crate::apply_runtime_icons(&app, None);
                    }
                    #[cfg(target_os = "windows")]
                    if effects.appearance {
                        crate::system::windows_titlebar::refresh_for_app(&app);
                    }
                    if let Some(days) = effects.history_prune_days {
                        match db::prune_transcriptions_older_than(&db, days) {
                            Ok(deleted) if deleted > 0 => {
                                log::info!("import_data: pruned {deleted} transcriptions older than {days} days");
                                let _ = app.emit("verenu:history-pruned", ());
                            }
                            Ok(_) => {}
                            Err(e) => {
                                log::warn!("import_data: history prune after import failed: {e}")
                            }
                        }
                    }
                    // Mirrored frontend settings refresh only after both stores succeed.
                    let _ = app.emit("verenu:settings-imported", ());
                },
            )
        })
    })
    .await
}

struct ImportEffects {
    runtime_icons: bool,
    #[cfg(target_os = "windows")]
    appearance: bool,
    history_prune_days: Option<i64>,
}

fn import_payload(
    settings: &store::SettingsHandle,
    db: &crate::DbHandle,
    payload: ExportPayload,
    installed_apps: &[crate::system::apps::InstalledApp],
    clear_provider_keys: impl FnOnce(&[crate::api::custom::CustomProvider]) -> Result<(), String>,
    completed: impl FnOnce(ImportEffects),
) -> Result<ImportSummary, String> {
    let mut pending_settings = Vec::new();
    let mut settings_applied = 0usize;
    let mut settings_skipped = 0usize;
    let mut runtime_icon_setting_applied = false;
    #[cfg(target_os = "windows")]
    let mut appearance_setting_applied = false;
    let mut history_prune_days: Option<i64> = None;

    if !payload.settings.is_object() {
        log::warn!(
            "import_data: 'settings' field is not a JSON object — skipping settings restore"
        );
    }
    if let Some(obj) = payload.settings.as_object() {
        // A backup may reuse a UUID with a different destination. Never
        // carry its previous host credential into that restored endpoint.
        if let Some(value) = obj.get(store::CUSTOM_PROVIDERS) {
            if let Ok(providers) = crate::api::custom::normalize_list(value) {
                clear_provider_keys(&providers)?;
            }
        }
        for (key, value) in obj {
            if !is_exportable_setting_key(key) {
                settings_skipped += 1;
                continue;
            }
            match validate_setting(key, value) {
                Ok(()) => {
                    let value_to_save = if key == store::CUSTOM_PROVIDERS {
                        serde_json::to_value(crate::api::custom::normalize_list(value)?)
                            .map_err(|_| "Could not encode restored providers.".to_string())?
                    } else {
                        value.clone()
                    };
                    pending_settings.push((key.clone(), value_to_save));
                    if crate::app_tray::setting_updates_runtime_icons(key) {
                        runtime_icon_setting_applied = true;
                    }
                    #[cfg(target_os = "windows")]
                    if key == store::APPEARANCE_MODE || key == store::CUSTOM_THEME {
                        appearance_setting_applied = true;
                    }
                    // Mirror save_setting's side effect: a backup that
                    // tightens history retention must prune immediately,
                    // not silently wait for the next app restart.
                    if key == store::HISTORY_RETENTION {
                        history_prune_days = value.as_str().and_then(store::history_retention_days);
                    }
                    settings_applied += 1;
                }
                Err(e) => {
                    log::warn!("import_data: skipping invalid setting '{key}': {e}");
                    settings_skipped += 1;
                }
            }
        }
    }

    let mut library_stats = LibraryImportStats::default();

    // Bulk-import dictionary entries and snippets inside a single
    // transaction (and a single lock acquisition) instead of one
    // implicit transaction per row - hundreds of individually committed
    // inserts each force a disk sync, which is slow, and leaves a
    // partially-imported database if the process dies mid-import.
    {
        let mut conn = db
            .lock()
            .map_err(|_| "Database lock was poisoned".to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        if payload.version == "1" {
            import_legacy_library_conn(&tx, &payload, &mut library_stats)
                .map_err(|e| e.to_string())?;
        } else {
            import_contextual_library_conn(&tx, &payload, installed_apps, &mut library_stats)
                .map_err(|e| e.to_string())?;
            // A hand-authored v2 payload may omit the Context graph. In
            // that case retain the same safe fallback as v1. A v2 payload
            // that does contain Contexts never imports its top-level
            // snippets, because doing so would leak targeted content to
            // Everywhere.
            if payload.contexts.is_empty() {
                import_legacy_snippets_conn(&tx, &payload.snippets, &mut library_stats)
                    .map_err(|e| e.to_string())?;
            }
        }

        // Rows remain uncommitted if settings persistence fails. On a
        // returned commit error, restore disk settings before returning;
        // runtime settings remain unpublished throughout both steps.
        settings
            .save_values_with_commit(pending_settings, || tx.commit().map_err(|e| e.to_string()))?;
    }

    completed(ImportEffects {
        runtime_icons: runtime_icon_setting_applied,
        #[cfg(target_os = "windows")]
        appearance: appearance_setting_applied,
        history_prune_days,
    });

    log::info!(
            "import_data: settings={}/skip={} contexts={}/skip={}/existed={} dict={}/skip={}/existed={} assignments={}/skip={} corrections={}/skip={} snip={}/skip={}/existed={}",
            settings_applied, settings_skipped,
            library_stats.contexts_inserted,
            library_stats.contexts_skipped,
            library_stats.contexts_already_existed,
            library_stats.dictionary_inserted,
            library_stats.dictionary_skipped,
            library_stats.dictionary_already_existed,
            library_stats.dictionary_assignments_inserted,
            library_stats.dictionary_assignments_skipped,
            library_stats.dictionary_corrections_inserted,
            library_stats.dictionary_corrections_skipped,
            library_stats.snippets_inserted,
            library_stats.snippets_skipped,
            library_stats.snippets_already_existed,
        );

    Ok(ImportSummary {
        settings_applied,
        settings_skipped,
        contexts_inserted: library_stats.contexts_inserted,
        contexts_skipped: library_stats.contexts_skipped,
        contexts_already_existed: library_stats.contexts_already_existed,
        dictionary_inserted: library_stats.dictionary_inserted,
        dictionary_skipped: library_stats.dictionary_skipped,
        dictionary_already_existed: library_stats.dictionary_already_existed,
        dictionary_assignments_inserted: library_stats.dictionary_assignments_inserted,
        dictionary_assignments_skipped: library_stats.dictionary_assignments_skipped,
        dictionary_corrections_inserted: library_stats.dictionary_corrections_inserted,
        dictionary_corrections_skipped: library_stats.dictionary_corrections_skipped,
        snippets_inserted: library_stats.snippets_inserted,
        snippets_skipped: library_stats.snippets_skipped,
        snippets_already_existed: library_stats.snippets_already_existed,
        app_targets_kept: library_stats.app_targets_kept,
        app_targets_matched: library_stats.app_targets_matched,
        app_targets_dropped: library_stats.app_targets_dropped,
        sub_apps_imported: library_stats.sub_apps_imported,
        sub_apps_dropped: library_stats.sub_apps_dropped,
    })
}

// Keep every restore side effect behind complete decoding and version checks.
fn with_import_payload<T>(
    json: &str,
    apply: impl FnOnce(ExportPayload) -> Result<T, String>,
) -> Result<T, String> {
    let payload: ExportPayload = serde_json::from_str(json)
        .map_err(|e| format!("Invalid backup file: {e}"))?;
    if payload.version != "1" && payload.version != "2" {
        return Err(format!(
            "Unsupported backup version '{}'. Versions '1' and '2' are supported.",
            payload.version
        ));
    }
    apply(payload)
}

#[cfg(test)]
mod import_safety_tests;

#[cfg(test)]
mod cross_store_tests;

// ---------------------------------------------------------------------------
// Context-aware library backup helpers
// ---------------------------------------------------------------------------

/// Export the persistent vocabulary graph without exporting AutoLearn's
/// short-lived evidence.  The canonical dictionary row is emitted once, while
/// assignments and correction mappings live under the Context that owns them.
/// This mirrors the runtime data model and means a restore cannot accidentally
/// turn a targeted mapping into an Everywhere mapping.
fn export_contextual_library(
    db: &db::Db,
) -> AnyhowResult<(Vec<ExportDictionaryEntry>, Vec<ExportContext>)> {
    let conn = db
        .lock()
        .map_err(|_| anyhow::anyhow!("Database lock was poisoned"))?;

    let dictionary = {
        let mut stmt = conn.prepare(
            "SELECT uuid, term, auto_learned, correction_count, confidence_tier,
                    created_at
               FROM dictionary
              ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ExportDictionaryEntry {
                    uuid: row.get(0)?,
                    term: row.get(1)?,
                    // v2 never uses the retired global projection. Scoped mappings
                    // are carried below in each Context.
                    mistake: None,
                    auto_learned: row.get::<_, i64>(2)? != 0,
                    correction_count: row.get(3)?,
                    confidence_tier: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut corrections_by_key: HashMap<(i64, i64), Vec<ExportDictionaryCorrection>> =
        HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT context_id, dictionary_id, uuid, mistake, auto_learned,
                    correction_count, confidence_tier, last_seen_at, created_at
               FROM dictionary_corrections
              ORDER BY context_id, dictionary_id, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                ExportDictionaryCorrection {
                    uuid: row.get(2)?,
                    mistake: row.get(3)?,
                    auto_learned: row.get::<_, i64>(4)? != 0,
                    correction_count: row.get(5)?,
                    confidence_tier: row.get(6)?,
                    last_seen_at: row.get(7)?,
                    created_at: row.get(8)?,
                },
            ))
        })?;
        for row in rows {
            let (context_id, dictionary_id, correction) = row?;
            corrections_by_key
                .entry((context_id, dictionary_id))
                .or_default()
                .push(correction);
        }
    }

    let mut contexts = Vec::new();
    let mut context_stmt = conn.prepare(
        "SELECT id, uuid, name, is_everywhere, icon, tone, cleanup_intensity, color,
                custom_instructions, contextual_formatting_disabled, pinned_at, paste_in_chunks
           FROM contexts
          ORDER BY is_everywhere DESC, id",
    )?;
    let context_rows = context_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            ExportContext {
                uuid: row.get(1)?,
                name: row.get(2)?,
                is_everywhere: row.get::<_, i64>(3)? != 0,
                icon: row.get(4)?,
                tone: row.get(5)?,
                cleanup_intensity: row.get(6)?,
                color: row.get(7)?,
                custom_instructions: row.get(8)?,
                contextual_formatting_disabled: row.get::<_, i64>(9)? != 0,
                pinned_at: row.get(10)?,
                paste_in_chunks: row.get::<_, i64>(11)? != 0,
                dictionary: Vec::new(),
                snippets: Vec::new(),
                targets: Vec::new(),
                website_targets: Vec::new(),
                sub_apps: Vec::new(),
            },
        ))
    })?;

    let mut dictionary_stmt = conn.prepare(
        "SELECT d.term, d.uuid, d.id
           FROM dictionary d
           INNER JOIN dictionary_contexts dc ON dc.dictionary_id = d.id
          WHERE dc.context_id = ?1
          ORDER BY d.id",
    )?;
    let mut snippet_stmt = conn.prepare(
        "SELECT trigger, expansion, instructions, created_at
           FROM snippets s
           INNER JOIN snippet_contexts sc ON sc.snippet_id = s.id
          WHERE sc.context_id = ?1
          ORDER BY s.id",
    )?;
    let mut target_stmt = conn.prepare(
        "SELECT executable, app_name, developer, platform
           FROM context_targets
          WHERE context_id = ?1
          ORDER BY id",
    )?;
    let mut website_stmt = conn.prepare(
        "SELECT domain
           FROM context_website_targets
          WHERE context_id = ?1
          ORDER BY id",
    )?;
    let mut sub_app_stmt = conn.prepare(
        "SELECT executable, app_name, platform, label, icon, title_pattern, match_mode
           FROM context_sub_apps
          WHERE context_id IS ?1 AND executable NOT LIKE '?::%'
          ORDER BY id",
    )?;

    for row in context_rows {
        let (context_id, mut context) = row?;

        {
            let rows = dictionary_stmt.query_map(params![context_id], |row| {
                let dictionary_id = row.get::<_, i64>(2)?;
                Ok(ExportContextDictionaryEntry {
                    term: row.get(0)?,
                    dictionary_uuid: row.get(1)?,
                    corrections: corrections_by_key
                        .remove(&(context_id, dictionary_id))
                        .unwrap_or_default(),
                })
            })?;
            context.dictionary = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        }

        {
            context.snippets = snippet_stmt
                .query_map(params![context_id], |row| {
                    Ok(ExportSnippet {
                        trigger: row.get(0)?,
                        expansion: row.get(1)?,
                        instructions: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        {
            context.targets = target_stmt
                .query_map(params![context_id], |row| {
                    Ok(ExportContextTarget {
                        executable: row.get(0)?,
                        app_name: row.get(1)?,
                        developer: row.get(2)?,
                        platform: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        {
            context.website_targets = website_stmt
                .query_map(params![context_id], |row| {
                    Ok(ExportContextWebsiteTarget {
                        domain: row.get(0)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        {
            context.sub_apps = sub_app_stmt
                .query_map(params![context_id], export_sub_app_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        contexts.push(context);
    }

    Ok((dictionary, contexts))
}

fn export_sub_app_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExportSubApp> {
    Ok(ExportSubApp {
        executable: row.get(0)?,
        app_name: row.get(1)?,
        platform: row.get(2)?,
        label: row.get(3)?,
        icon: row.get(4)?,
        title_pattern: row.get(5)?,
        match_mode: row.get(6)?,
    })
}

/// Sub-apps waiting in the list, not assigned to a Context.
fn export_unassigned_sub_apps(db: &db::Db) -> AnyhowResult<Vec<ExportSubApp>> {
    let conn = db.lock().map_err(|_| anyhow::anyhow!("Database lock was poisoned"))?;
    let mut stmt = conn.prepare(
        "SELECT executable, app_name, platform, label, icon, title_pattern, match_mode
           FROM context_sub_apps
          WHERE context_id IS NULL AND executable NOT LIKE '?::%'
          ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], export_sub_app_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn import_legacy_library_conn(
    conn: &Connection,
    payload: &ExportPayload,
    stats: &mut LibraryImportStats,
) -> AnyhowResult<()> {
    for (index, entry) in payload.dictionary.iter().enumerate() {
        if entry.term.trim().is_empty() {
            stats.dictionary_skipped += 1;
            continue;
        }
        match db::insert_dictionary_entry_from_backup_conn(
            conn,
            &entry.term,
            entry.mistake.as_deref(),
            entry.auto_learned,
            &entry.confidence_tier,
            entry.correction_count,
        ) {
            Ok(()) => stats.dictionary_inserted += 1,
            Err(error) => {
                let message = error.to_string();
                if message.contains("UNIQUE constraint failed") {
                    stats.dictionary_already_existed += 1;
                } else {
                    log::warn!(
                        "import_data: dictionary insert error row={} chars={} error={message}",
                        index,
                        entry.term.chars().count()
                    );
                    stats.dictionary_skipped += 1;
                }
            }
        }
    }
    import_legacy_snippets_conn(conn, &payload.snippets, stats)
}

fn import_legacy_snippets_conn(
    conn: &Connection,
    snippets: &[ExportSnippet],
    stats: &mut LibraryImportStats,
) -> AnyhowResult<()> {
    for (index, snippet) in snippets.iter().enumerate() {
        if snippet.trigger.trim().is_empty() || snippet.expansion.trim().is_empty() {
            stats.snippets_skipped += 1;
            continue;
        }
        match db::insert_snippet_returning_conn(
            conn,
            &snippet.trigger,
            &snippet.expansion,
            &snippet.instructions,
            None,
        ) {
            Ok(_) => stats.snippets_inserted += 1,
            Err(error) => {
                let message = error.to_string();
                if message.contains("UNIQUE constraint failed") {
                    stats.snippets_already_existed += 1;
                } else {
                    log::warn!(
                        "import_data: snippet insert error row={} trigger_chars={} expansion_chars={} error={message}",
                        index,
                        snippet.trigger.chars().count(),
                        snippet.expansion.chars().count()
                    );
                    stats.snippets_skipped += 1;
                }
            }
        }
    }
    Ok(())
}

fn import_contextual_library_conn(
    conn: &Connection,
    payload: &ExportPayload,
    installed_apps: &[crate::system::apps::InstalledApp],
    stats: &mut LibraryImportStats,
) -> AnyhowResult<()> {
    let mut dictionary_ids: HashMap<String, i64> = HashMap::new();

    // Create canonical rows first.  The canonical term remains globally unique
    // and shared; Context-specific correction rows are restored in the second
    // pass below.
    for entry in &payload.dictionary {
        let Some(dictionary_id) = import_canonical_dictionary_conn(conn, entry, stats)? else {
            continue;
        };
        dictionary_ids.insert(entry.term.trim().to_string(), dictionary_id);
    }

    if payload.contexts.is_empty() {
        // A v2-shaped hand-authored payload without a Context graph has no safe
        // targeted owner. Treat its top-level rows as a legacy Everywhere
        // backup, preserving compatibility without inventing a target Context.
        let everywhere_id = db::ensure_everywhere_context_conn(conn)?;
        for entry in &payload.dictionary {
            let dictionary_id =
                if let Some(dictionary_id) = dictionary_ids.get(entry.term.trim()).copied() {
                    Some(dictionary_id)
                } else {
                    let dictionary_id = import_canonical_dictionary_conn(conn, entry, stats)?;
                    if let Some(dictionary_id) = dictionary_id {
                        dictionary_ids.insert(entry.term.trim().to_string(), dictionary_id);
                    }
                    dictionary_id
                };
            let Some(dictionary_id) = dictionary_id else {
                continue;
            };
            let assigned = conn.execute(
                "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
                 VALUES (?1, ?2)",
                params![everywhere_id, dictionary_id],
            )?;
            stats.dictionary_assignments_inserted += usize::from(assigned > 0);
            if let Some(mistake) = entry.mistake.as_deref() {
                import_correction_conn(
                    conn,
                    ImportCorrection {
                        context_id: everywhere_id,
                        dictionary_id,
                        mistake,
                        auto_learned: entry.auto_learned,
                        correction_count: entry.correction_count,
                        confidence_tier: &entry.confidence_tier,
                        last_seen_at: None,
                        created_at: None,
                        source_uuid: None,
                    },
                    stats,
                )?;
            }
        }
        return Ok(());
    }

    let mut context_ids = HashMap::new();
    for context in &payload.contexts {
        let Some(context_id) = import_context_conn(conn, context, stats)? else {
            continue;
        };
        context_ids.insert(context_key(context), context_id);
    }

    // Context-owned vocabulary and correction mappings.
    for context in &payload.contexts {
        let Some(context_id) = context_ids.get(&context_key(context)).copied() else {
            continue;
        };
        for entry in &context.dictionary {
            let dictionary_id = if let Some(id) = dictionary_ids.get(entry.term.trim()).copied() {
                id
            } else {
                let canonical = ExportDictionaryEntry {
                    term: entry.term.clone(),
                    mistake: None,
                    auto_learned: false,
                    confidence_tier: "low".to_string(),
                    correction_count: 0,
                    created_at: String::new(),
                    uuid: entry.dictionary_uuid.clone(),
                };
                let Some(id) = import_canonical_dictionary_conn(conn, &canonical, stats)? else {
                    stats.dictionary_assignments_skipped += 1;
                    continue;
                };
                dictionary_ids.insert(entry.term.trim().to_string(), id);
                id
            };

            let assigned = conn.execute(
                "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
                 VALUES (?1, ?2)",
                params![context_id, dictionary_id],
            )?;
            stats.dictionary_assignments_inserted += usize::from(assigned > 0);

            for correction in &entry.corrections {
                import_correction_conn(
                    conn,
                    ImportCorrection {
                        context_id,
                        dictionary_id,
                        mistake: &correction.mistake,
                        auto_learned: correction.auto_learned,
                        correction_count: correction.correction_count,
                        confidence_tier: &correction.confidence_tier,
                        last_seen_at: correction.last_seen_at.as_deref(),
                        created_at: Some(correction.created_at.as_str()),
                        source_uuid: correction.uuid.as_deref(),
                    },
                    stats,
                )?;
            }
        }

        for target in &context.targets {
            let Some(resolved) = resolve_import_target(
                target,
                installed_apps,
                db::current_platform_tag(),
            ) else {
                stats.app_targets_dropped += 1;
                continue;
            };
            if resolved.matched {
                stats.app_targets_matched += 1;
            } else {
                stats.app_targets_kept += 1;
            }
            let ResolvedImportTarget {
                executable,
                app_name,
                developer,
                platform,
                ..
            } = resolved;
            conn.execute(
                "INSERT INTO context_targets
                   (context_id, executable, app_name, developer, platform)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(executable) DO UPDATE SET
                   context_id = excluded.context_id,
                   app_name = excluded.app_name,
                   developer = excluded.developer,
                   platform = excluded.platform",
                params![context_id, executable, app_name, developer, platform],
            )?;
        }

        for target in &context.website_targets {
            let domain = target.domain.trim().to_lowercase();
            if domain.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT INTO context_website_targets (context_id, domain)
                 VALUES (?1, ?2)
                 ON CONFLICT(domain) DO UPDATE SET context_id = excluded.context_id",
                params![context_id, domain],
            )?;
        }

        for sub_app in &context.sub_apps {
            import_sub_app_conn(conn, sub_app, Some(context_id), installed_apps, stats)?;
        }

        for snippet in &context.snippets {
            if snippet.trigger.trim().is_empty() || snippet.expansion.trim().is_empty() {
                stats.snippets_skipped += 1;
                continue;
            }
            let existing: bool = conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM snippets s
                   INNER JOIN snippet_contexts sc ON sc.snippet_id = s.id
                    WHERE s.trigger = ?1 AND sc.context_id = ?2
                 )",
                params![snippet.trigger.trim(), context_id],
                |row| row.get(0),
            )?;
            if existing {
                // The contextual import is idempotent. Avoid calling the
                // insert helper for an assignment that is already present:
                // it intentionally rejects duplicate Context membership and
                // would otherwise misclassify a successful no-op as skipped.
                stats.snippets_already_existed += 1;
                continue;
            }
            match db::insert_snippet_returning_conn(
                conn,
                &snippet.trigger,
                &snippet.expansion,
                &snippet.instructions,
                Some(context_id),
            ) {
                Ok(_) => stats.snippets_inserted += 1,
                Err(error) => {
                    log::warn!(
                        "import_data: contextual snippet import failed trigger_chars={} error={error}",
                        snippet.trigger.chars().count()
                    );
                    stats.snippets_skipped += 1;
                }
            }
        }
    }

    for sub_app in &payload.sub_apps {
        import_sub_app_conn(conn, sub_app, None, installed_apps, stats)?;
    }

    Ok(())
}

#[derive(Debug, PartialEq)]
struct ResolvedImportTarget {
    executable: String,
    app_name: Option<String>,
    developer: Option<String>,
    platform: Option<String>,
    /// True when the target was rebound to a different local app.
    matched: bool,
}

/// Whether an executable identity has the shape this OS uses: `.exe` on
/// Windows, `.app` bundles on macOS, suffix-less desktop ids/classes on Linux.
fn looks_native_target(executable: &str, platform: Option<&str>) -> bool {
    match platform {
        Some("windows") => executable.ends_with(".exe"),
        Some("macos") => executable.ends_with(".app"),
        Some("linux") => !executable.ends_with(".exe") && !executable.ends_with(".app"),
        _ => true,
    }
}

/// Maps a backed-up app target onto this device. Targets from this platform
/// are restored verbatim; targets from another platform (or an unresolved
/// sync marker) are rebound to the closest installed app, and dropped when
/// nothing is close enough so the Context still imports cleanly.
fn resolve_import_target(
    target: &ExportContextTarget,
    installed_apps: &[crate::system::apps::InstalledApp],
    current_platform: Option<&str>,
) -> Option<ResolvedImportTarget> {
    let raw = target.executable.trim().to_lowercase();
    let unresolved = raw.starts_with("?::");
    let executable = raw.trim_start_matches("?::").split('#').next().unwrap_or("").trim().to_string();
    if executable.is_empty() {
        return None;
    }
    if let Some(app) = installed_apps
        .iter()
        .find(|app| app.exe.trim().eq_ignore_ascii_case(&executable))
    {
        return Some(ResolvedImportTarget {
            executable,
            app_name: target.app_name.clone().or_else(|| Some(app.name.clone())),
            developer: target.developer.clone().or_else(|| app.developer.clone()),
            platform: current_platform.map(str::to_string),
            matched: false,
        });
    }
    let same_platform = match target.platform.as_deref() {
        Some(platform) => current_platform == Some(platform),
        None => looks_native_target(&executable, current_platform),
    };
    if !unresolved && same_platform {
        return Some(ResolvedImportTarget {
            executable,
            app_name: target.app_name.clone(),
            developer: target.developer.clone(),
            platform: current_platform.map(str::to_string),
            matched: false,
        });
    }
    let app = crate::system::apps::closest_installed_app(
        &executable,
        target.app_name.as_deref(),
        target.developer.as_deref(),
        installed_apps,
    )?;
    Some(ResolvedImportTarget {
        executable: app.exe.trim().to_lowercase(),
        app_name: Some(app.name.clone()),
        developer: app.developer.clone(),
        platform: current_platform.map(str::to_string),
        matched: true,
    })
}

/// Rebinds a backed-up sub-app to a local app (same matching as app targets)
/// and inserts it, assigned to `context_id` or left in the list.
fn import_sub_app_conn(
    conn: &Connection,
    sub_app: &ExportSubApp,
    context_id: Option<i64>,
    installed_apps: &[crate::system::apps::InstalledApp],
    stats: &mut LibraryImportStats,
) -> AnyhowResult<()> {
    let as_target = ExportContextTarget {
        executable: sub_app.executable.clone(),
        app_name: sub_app.app_name.clone(),
        developer: None,
        platform: sub_app.platform.clone(),
    };
    let (Some(resolved), Ok(mode)) = (
        resolve_import_target(&as_target, installed_apps, db::current_platform_tag()),
        db::TitleMatchMode::parse(&sub_app.match_mode),
    ) else {
        stats.sub_apps_dropped += 1;
        return Ok(());
    };
    let pattern = sub_app.title_pattern.trim();
    let label = sub_app.label.trim();
    if pattern.is_empty() || label.is_empty() {
        stats.sub_apps_dropped += 1;
        return Ok(());
    }
    let inserted = conn.execute(
        "INSERT INTO context_sub_apps
           (uuid, context_id, executable, app_name, label, icon, title_pattern, match_mode, platform)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(executable, title_pattern, match_mode) DO UPDATE SET
           context_id = COALESCE(excluded.context_id, context_sub_apps.context_id),
           label = excluded.label, icon = excluded.icon, updated_at = datetime('now')",
        params![
            Uuid::new_v4().to_string(),
            context_id,
            resolved.executable,
            resolved.app_name,
            label,
            sub_app.icon,
            pattern,
            mode.as_str(),
            resolved.platform,
        ],
    )?;
    stats.sub_apps_imported += usize::from(inserted > 0);
    Ok(())
}

fn context_key(context: &ExportContext) -> String {
    context
        .uuid
        .as_deref()
        .filter(|uuid| !uuid.trim().is_empty())
        .map(|uuid| format!("uuid:{uuid}"))
        .unwrap_or_else(|| format!("name:{}", context.name.trim().to_lowercase()))
}

fn import_context_conn(
    conn: &Connection,
    source: &ExportContext,
    stats: &mut LibraryImportStats,
) -> AnyhowResult<Option<i64>> {
    let name = source.name.trim();
    if name.is_empty() {
        stats.contexts_skipped += 1;
        return Ok(None);
    }
    if source.is_everywhere {
        let id = db::ensure_everywhere_context_conn(conn)?;
        conn.execute(
            "UPDATE contexts SET icon = ?1, tone = ?2, cleanup_intensity = ?3,
                    color = ?4, custom_instructions = ?5,
                    contextual_formatting_disabled = ?6, pinned_at = ?7, paste_in_chunks = ?9,
                    updated_at = datetime('now')
              WHERE id = ?8",
            params![
                source.icon,
                source.tone,
                source.cleanup_intensity,
                source.color,
                source.custom_instructions,
                source.contextual_formatting_disabled as i64,
                source.pinned_at,
                id,
                source.paste_in_chunks,
            ],
        )?;
        stats.contexts_already_existed += 1;
        return Ok(Some(id));
    }

    let existing_by_uuid: Option<i64> = if let Some(uuid) = source
        .uuid
        .as_deref()
        .filter(|uuid| !uuid.trim().is_empty())
    {
        conn.query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            params![uuid],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    let existing_by_name: Option<i64> = conn
        .query_row(
            "SELECT id FROM contexts WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |row| row.get(0),
        )
        .optional()?;
    let existing: Option<i64> = existing_by_uuid.or(existing_by_name);

    if let Some(id) = existing {
        // A UUID identifies the local Context even when another local Context
        // already owns the imported name. Preserve that stable identity and
        // local name rather than aborting the whole import on the UNIQUE name
        // constraint; the remaining settings still import deterministically.
        if existing_by_name.is_none() || existing_by_name == Some(id) {
            conn.execute(
                "UPDATE contexts SET name = ?1, icon = ?2, tone = ?3,
                        cleanup_intensity = ?4, color = ?5, custom_instructions = ?6,
                        contextual_formatting_disabled = ?7, pinned_at = ?8, paste_in_chunks = ?10,
                        updated_at = datetime('now')
                  WHERE id = ?9",
                params![
                    name,
                    source.icon,
                    source.tone,
                    source.cleanup_intensity,
                    source.color,
                    source.custom_instructions,
                    source.contextual_formatting_disabled as i64,
                    source.pinned_at,
                    id,
                    source.paste_in_chunks,
                ],
            )?;
        } else {
            conn.execute(
                "UPDATE contexts SET icon = ?1, tone = ?2,
                        cleanup_intensity = ?3, color = ?4, custom_instructions = ?5,
                        contextual_formatting_disabled = ?6, pinned_at = ?7, paste_in_chunks = ?9,
                        updated_at = datetime('now')
                  WHERE id = ?8",
                params![
                    source.icon,
                    source.tone,
                    source.cleanup_intensity,
                    source.color,
                    source.custom_instructions,
                    source.contextual_formatting_disabled as i64,
                    source.pinned_at,
                    id,
                    source.paste_in_chunks,
                ],
            )?;
        }
        stats.contexts_already_existed += 1;
        return Ok(Some(id));
    }

    let user_context_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM contexts WHERE is_everywhere = 0",
        [],
        |row| row.get(0),
    )?;
    if user_context_count >= db::MAX_USER_CONTEXTS {
        stats.contexts_skipped += 1;
        return Ok(None);
    }

    let uuid = source
        .uuid
        .as_deref()
        .filter(|uuid| Uuid::parse_str(uuid).is_ok())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let inserted = conn.execute(
        "INSERT INTO contexts
           (uuid, name, is_everywhere, icon, tone, cleanup_intensity, color,
            custom_instructions, contextual_formatting_disabled, pinned_at, paste_in_chunks)
         VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            uuid,
            name,
            source.icon,
            source.tone,
            source.cleanup_intensity,
            source.color,
            source.custom_instructions,
            source.contextual_formatting_disabled as i64,
            source.pinned_at,
            source.paste_in_chunks,
        ],
    );
    match inserted {
        Ok(_) => {
            stats.contexts_inserted += 1;
            Ok(Some(conn.last_insert_rowid()))
        }
        Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
            // A backup can carry a UUID already used by a different local
            // record. The name is the conservative fallback; if it also
            // conflicts, skip the graph rather than merging into the wrong
            // Context.
            let by_name: Option<i64> = conn
                .query_row(
                    "SELECT id FROM contexts WHERE name = ?1 COLLATE NOCASE",
                    params![name],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(id) = by_name {
                stats.contexts_already_existed += 1;
                Ok(Some(id))
            } else {
                stats.contexts_skipped += 1;
                Ok(None)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn import_canonical_dictionary_conn(
    conn: &Connection,
    source: &ExportDictionaryEntry,
    stats: &mut LibraryImportStats,
) -> AnyhowResult<Option<i64>> {
    let term = source.term.trim();
    if term.is_empty() {
        stats.dictionary_skipped += 1;
        return Ok(None);
    }
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM dictionary WHERE term = ?1",
            params![term],
            |row| row.get(0),
        )
        .optional()?
    {
        stats.dictionary_already_existed += 1;
        return Ok(Some(id));
    }

    let uuid = source
        .uuid
        .as_deref()
        .filter(|uuid| Uuid::parse_str(uuid).is_ok())
        .filter(|uuid| {
            conn.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM dictionary WHERE uuid = ?1)",
                params![uuid],
                |row| row.get(0),
            )
            .unwrap_or(false)
        })
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    match conn.execute(
        "INSERT INTO dictionary
           (uuid, term, mistake, auto_learned, correction_count, confidence_tier, created_at)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, CASE WHEN ?6 = '' THEN datetime('now') ELSE ?6 END)",
        params![
            uuid,
            term,
            source.auto_learned as i64,
            source.correction_count.max(0),
            source.confidence_tier,
            source.created_at,
        ],
    ) {
        Ok(_) => {
            stats.dictionary_inserted += 1;
            Ok(Some(conn.last_insert_rowid()))
        }
        Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
            let id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM dictionary WHERE term = ?1",
                    params![term],
                    |row| row.get(0),
                )
                .optional()?;
            if id.is_some() {
                stats.dictionary_already_existed += 1;
            } else {
                stats.dictionary_skipped += 1;
            }
            Ok(id)
        }
        Err(error) => {
            stats.dictionary_skipped += 1;
            log::warn!(
                "import_data: canonical dictionary insert failed term_chars={} error={error}",
                term.chars().count()
            );
            Ok(None)
        }
    }
}

fn import_correction_conn(
    conn: &Connection,
    correction: ImportCorrection<'_>,
    stats: &mut LibraryImportStats,
) -> AnyhowResult<()> {
    let context_id = correction.context_id;
    let dictionary_id = correction.dictionary_id;
    let mistake = correction.mistake.trim();
    if mistake.is_empty() {
        stats.dictionary_corrections_skipped += 1;
        return Ok(());
    }

    let competing: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM dictionary_corrections
              WHERE context_id = ?1 AND mistake = ?2 AND dictionary_id != ?3
         )",
        params![context_id, mistake, dictionary_id],
        |row| row.get(0),
    )?;
    if competing {
        // One wrong spelling cannot mechanically map to two canonical terms in
        // a Context. Preserve the effective local mapping instead of letting a
        // backup import silently change dictation behavior.
        stats.dictionary_corrections_skipped += 1;
        return Ok(());
    }

    let existing: Option<ExistingCorrection> = conn
        .query_row(
            "SELECT id, uuid, auto_learned, correction_count, confidence_tier, last_seen_at
               FROM dictionary_corrections
              WHERE context_id = ?1 AND dictionary_id = ?2 AND mistake = ?3",
            params![context_id, dictionary_id, mistake],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;

    if let Some((
        id,
        _existing_uuid,
        existing_auto,
        existing_count,
        existing_tier,
        existing_last_seen,
    )) = existing
    {
        if !existing_auto && correction.auto_learned {
            stats.dictionary_corrections_skipped += 1;
            return Ok(());
        }
        if !correction.auto_learned && existing_auto {
            // An explicit manual import is allowed to promote an existing
            // automatic mapping to manual authority in this Context. The
            // reverse (automatic over manual) is never allowed.
            conn.execute(
                "UPDATE dictionary_corrections
                    SET auto_learned = 0, correction_count = 0,
                        confidence_tier = 'manual', last_seen_at = NULL
                  WHERE id = ?1",
                params![id],
            )?;
            stats.dictionary_corrections_skipped += 1;
            return Ok(());
        }
        if existing_auto && correction.auto_learned {
            let tier =
                if confidence_rank(correction.confidence_tier) > confidence_rank(&existing_tier) {
                    correction.confidence_tier
                } else {
                    &existing_tier
                };
            let last_seen = later_timestamp(
                existing_last_seen,
                correction.last_seen_at.map(str::to_string),
            );
            conn.execute(
                "UPDATE dictionary_corrections
                    SET correction_count = ?1, confidence_tier = ?2, last_seen_at = ?3
                  WHERE id = ?4",
                params![
                    existing_count.max(correction.correction_count.max(0)),
                    tier,
                    last_seen,
                    id,
                ],
            )?;
        }
        // Existing manual data remains authoritative, including when the
        // backup also contains a manual copy of the same mapping.
        stats.dictionary_corrections_skipped += 1;
        return Ok(());
    }

    let mapping_uuid = correction
        .source_uuid
        .filter(|uuid| Uuid::parse_str(uuid).is_ok())
        .filter(|uuid| {
            conn.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM dictionary_corrections WHERE uuid = ?1)",
                params![uuid],
                |row| row.get(0),
            )
            .unwrap_or(false)
        })
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let created_at = correction
        .created_at
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    conn.execute(
        "INSERT INTO dictionary_corrections
           (uuid, context_id, dictionary_id, mistake, auto_learned,
            correction_count, confidence_tier, last_seen_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 CASE WHEN ?9 = '' THEN datetime('now') ELSE ?9 END)",
        params![
            mapping_uuid,
            context_id,
            dictionary_id,
            mistake,
            correction.auto_learned as i64,
            correction.correction_count.max(0),
            if correction.confidence_tier.trim().is_empty() {
                "low"
            } else {
                correction.confidence_tier
            },
            correction.last_seen_at,
            created_at,
        ],
    )?;
    stats.dictionary_corrections_inserted += 1;
    Ok(())
}

fn confidence_rank(tier: &str) -> u8 {
    match tier {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn later_timestamp(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db;

    #[test]
    fn paste_chunks_backup_round_trip_and_old_payload_default() {
        let source = db::open(":memory:").unwrap();
        db::insert_context_with_delivery(&source, "CLI chunks", None, None, None, None, false, true).unwrap();
        let (_, contexts) = export_contextual_library(&source).unwrap();
        let context = contexts.into_iter().find(|c| c.name == "CLI chunks").unwrap();
        assert!(context.paste_in_chunks);
        let target = db::open(":memory:").unwrap();
        let conn = target.lock().unwrap();
        let mut stats = LibraryImportStats::default();
        let id = import_context_conn(&conn, &context, &mut stats).unwrap().unwrap();
        assert!(conn.query_row("SELECT paste_in_chunks FROM contexts WHERE id = ?1", [id], |row| row.get::<_, bool>(0)).unwrap());
        import_context_conn(&conn, &context, &mut stats).unwrap();
        assert!(conn.query_row("SELECT paste_in_chunks FROM contexts WHERE id = ?1", [id], |row| row.get::<_, bool>(0)).unwrap());
        let mut old = serde_json::to_value(&context).unwrap();
        old.as_object_mut().unwrap().remove("paste_in_chunks");
        let old: ExportContext = serde_json::from_value(old).unwrap();
        assert!(!old.paste_in_chunks);
    }

    #[test]
    fn contextual_backup_round_trip_preserves_context_scoped_corrections() {
        let source = db::open(":memory:").expect("source db");
        let development =
            db::insert_context_returning(&source, "Development", None, None, None, None, false)
                .expect("development context");
        let writing =
            db::insert_context_returning(&source, "Writing", None, None, None, None, false)
                .expect("writing context");
        db::assign_context_target(&source, development.id, "code.exe").expect("app target");
        db::assign_context_website(&source, writing.id, "mail.example.com")
            .expect("website target");
        let canonical = db::insert_dictionary_entry_returning(
            &source,
            "Groq",
            Some("grock"),
            Some(development.id),
        )
        .expect("development mapping");
        db::set_dictionary_context_assignment(&source, writing.id, canonical.id, true)
            .expect("share canonical identity");
        db::insert_dictionary_entry_auto_learned_for_context(
            &source,
            writing.id,
            "Groq",
            Some("rock"),
            "high",
        )
        .expect("writing mapping");
        db::insert_snippet_returning(&source, "sig", "signature", "", Some(development.id))
            .expect("snippet");

        let (dictionary, contexts) = export_contextual_library(&source).expect("export");
        assert_eq!(contexts.len(), 3);
        assert!(contexts
            .iter()
            .find(|context| context.name == "Development")
            .expect("development export")
            .dictionary
            .iter()
            .flat_map(|entry| entry.corrections.iter())
            .any(|correction| correction.mistake == "grock"));
        assert!(contexts
            .iter()
            .find(|context| context.name == "Writing")
            .expect("writing export")
            .dictionary
            .iter()
            .flat_map(|entry| entry.corrections.iter())
            .any(|correction| correction.mistake == "rock"));

        let target = db::open(":memory:").expect("target db");
        let payload = ExportPayload {
            version: "2".to_string(),
            app_version: "test".to_string(),
            exported_at: "test".to_string(),
            stats: ExportStats::default(),
            settings: serde_json::Value::Null,
            dictionary,
            snippets: Vec::new(),
            contexts,
            sub_apps: Vec::new(),
        };
        let mut first_stats = LibraryImportStats::default();
        {
            let mut conn = target.lock().expect("target lock");
            let tx = conn.transaction().expect("target transaction");
            import_contextual_library_conn(&tx, &payload, &[], &mut first_stats).expect("import");
            tx.commit().expect("commit");
        }
        let target_contexts = db::query_contexts(&target).expect("contexts");
        let target_development = target_contexts
            .iter()
            .find(|context| context.name == "Development")
            .expect("target development");
        let target_writing = target_contexts
            .iter()
            .find(|context| context.name == "Writing")
            .expect("target writing");
        let development_entry = db::query_dictionary_for_context(&target, target_development.id)
            .expect("development dictionary")
            .into_iter()
            .find(|entry| entry.term == "Groq")
            .expect("development canonical");
        let writing_entry = db::query_dictionary_for_context(&target, target_writing.id)
            .expect("writing dictionary")
            .into_iter()
            .find(|entry| entry.term == "Groq")
            .expect("writing canonical");
        assert_eq!(development_entry.mistake.as_deref(), Some("grock"));
        assert_eq!(writing_entry.mistake.as_deref(), Some("rock"));
        assert_eq!(first_stats.dictionary_corrections_inserted, 2);
        assert_eq!(
            db::query_context_targets(&target, Some(target_development.id))
                .expect("target app")
                .len(),
            1
        );
        assert_eq!(
            db::query_context_website_targets(&target, Some(target_writing.id))
                .expect("target website")
                .len(),
            1
        );

        let before = db::query_dictionary_for_context(&target, target_writing.id)
            .expect("before repeated import")
            .into_iter()
            .find(|entry| entry.term == "Groq")
            .expect("writing before repeated import")
            .corrections
            .len();
        let mut second_stats = LibraryImportStats::default();
        {
            let mut conn = target.lock().expect("target relock");
            let tx = conn.transaction().expect("repeat transaction");
            import_contextual_library_conn(&tx, &payload, &[], &mut second_stats)
                .expect("repeat import");
            tx.commit().expect("repeat commit");
        }
        let after = db::query_dictionary_for_context(&target, target_writing.id)
            .expect("after repeated import")
            .into_iter()
            .find(|entry| entry.term == "Groq")
            .expect("writing after repeated import")
            .corrections
            .len();
        assert_eq!(before, after);
        assert_eq!(second_stats.snippets_already_existed, 1);
        assert_eq!(second_stats.snippets_skipped, 0);
    }

    fn app(name: &str, exe: &str) -> crate::system::apps::InstalledApp {
        crate::system::apps::InstalledApp {
            name: name.to_string(),
            exe: exe.to_string(),
            developer: None,
        }
    }

    fn target(executable: &str, name: Option<&str>, platform: Option<&str>) -> ExportContextTarget {
        ExportContextTarget {
            executable: executable.to_string(),
            app_name: name.map(str::to_string),
            developer: None,
            platform: platform.map(str::to_string),
        }
    }

    #[test]
    fn import_rebinds_foreign_targets_to_closest_local_app() {
        let installed = [app("Google Chrome", "google-chrome"), app("Slack", "slack")];
        let resolved = resolve_import_target(
            &target("chrome.exe", Some("Google Chrome"), Some("windows")),
            &installed,
            Some("linux"),
        )
        .expect("closest match");
        assert_eq!(resolved.executable, "google-chrome");
        assert_eq!(resolved.platform.as_deref(), Some("linux"));
        assert!(resolved.matched);
    }

    #[test]
    fn import_drops_foreign_targets_without_a_local_match() {
        let installed = [app("Slack", "slack")];
        assert_eq!(
            resolve_import_target(
                &target("winword.exe", Some("Microsoft Word"), Some("windows")),
                &installed,
                Some("linux"),
            ),
            None
        );
        assert_eq!(
            resolve_import_target(&target("?::photoshop.app", None, None), &installed, Some("linux")),
            None
        );
    }

    #[test]
    fn import_keeps_same_platform_targets_even_when_not_installed() {
        let resolved = resolve_import_target(
            &target("Obsidian.exe", Some("Obsidian"), Some("windows")),
            &[],
            Some("windows"),
        )
        .expect("native target kept");
        assert_eq!(resolved.executable, "obsidian.exe");
        assert!(!resolved.matched);
        // Untagged legacy targets are judged by their shape.
        assert!(resolve_import_target(&target("code.exe", None, None), &[], Some("windows")).is_some());
        assert!(resolve_import_target(&target("code.exe", None, None), &[], Some("linux")).is_none());
    }

    #[test]
    fn sub_apps_round_trip_and_unmatched_apps_are_left_out() {
        let source = db::open(":memory:").expect("source");
        let work = db::insert_context_returning(&source, "Work", None, None, None, None, false)
            .expect("ctx");
        let mut ids = Vec::new();
        for (exe, label) in [("discord", "Acme"), ("photoshop.app", "Retouch"), ("slack", "Loose")] {
            ids.push(
                db::create_sub_app(
                    &source,
                    db::NewSubApp {
                        executable: exe,
                        app_name: None,
                        label,
                        icon: Some("briefcase"),
                        title_pattern: label,
                        match_mode: db::TitleMatchMode::Contains,
                    },
                )
                .expect("sub-app")
                .id,
            );
        }
        // Two are assigned; "Loose" stays in the list.
        db::assign_sub_app(&source, ids[0], Some(work.id)).expect("assign");
        db::assign_sub_app(&source, ids[1], Some(work.id)).expect("assign");
        // The Photoshop rule was captured on a Mac.
        source
            .lock()
            .expect("lock")
            .execute("UPDATE context_sub_apps SET platform = 'macos' WHERE executable = 'photoshop.app'", [])
            .expect("tag");
        let (dictionary, contexts) = export_contextual_library(&source).expect("export");
        let payload = ExportPayload {
            version: "2".to_string(),
            app_version: "test".to_string(),
            exported_at: "test".to_string(),
            stats: ExportStats::default(),
            settings: serde_json::Value::Null,
            dictionary,
            snippets: Vec::new(),
            contexts,
            sub_apps: export_unassigned_sub_apps(&source).expect("unassigned"),
        };

        let target = db::open(":memory:").expect("target");
        let installed = [app("Discord", "discord"), app("Slack", "slack")];
        let mut stats = LibraryImportStats::default();
        {
            let mut conn = target.lock().expect("lock");
            let tx = conn.transaction().expect("tx");
            import_contextual_library_conn(&tx, &payload, &installed, &mut stats).expect("import");
            tx.commit().expect("commit");
        }
        let imported = db::query_sub_apps(&target).expect("sub-apps");
        let acme = imported.iter().find(|s| s.label == "Acme").expect("acme");
        assert!(acme.context_id.is_some(), "assigned sub-app keeps its Context");
        assert_eq!(acme.icon.as_deref(), Some("briefcase"));
        let loose = imported.iter().find(|s| s.label == "Loose").expect("loose");
        assert!(loose.context_id.is_none(), "unassigned sub-app returns to the list");
        // The macOS-only .app has no match off macOS and is left out.
        let expected_dropped = if cfg!(target_os = "macos") { 0 } else { 1 };
        assert_eq!(stats.sub_apps_dropped, expected_dropped);
        assert_eq!(stats.sub_apps_imported, 3 - expected_dropped);
    }
}
