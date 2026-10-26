use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::*;

pub const EVERYWHERE_CONTEXT_ID: i64 = 1;

pub const MAX_USER_CONTEXTS: i64 = 200;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Context {
    pub id: i64,
    pub name: String,
    pub is_everywhere: bool,
    pub icon: Option<String>,
    pub tone: Option<String>,
    pub cleanup_intensity: Option<String>,
    pub color: Option<String>,
    pub custom_instructions: Option<String>,
    pub contextual_formatting_disabled: bool,
    #[serde(default)]
    pub paste_in_chunks: bool,
    #[serde(default)]
    pub t3_skill_mentions_disabled: bool,
    /// `None` inherits the global smart-formatting setting, `Some(false)`
    /// explicitly enables it, and `Some(true)` explicitly disables it.
    pub contextual_formatting_override: Option<bool>,
    /// `NULL` when unpinned. Pinned contexts sort newest-pin-first in the
    /// sidebar; Everywhere is pinned implicitly by the UI and never sets this.
    pub pinned_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextAssignment {
    pub id: i64,
    pub name: String,
    pub is_everywhere: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextTarget {
    pub id: i64,
    pub context_id: i64,
    pub executable: String,
    pub app_name: Option<String>,
    pub developer: Option<String>,
    pub platform: Option<String>,
    pub created_at: String,
}

type ContextTargetRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub fn current_platform_tag() -> Option<&'static str> {
    if cfg!(windows) {
        Some("windows")
    } else if cfg!(target_os = "macos") {
        Some("macos")
    } else if cfg!(target_os = "linux") {
        Some("linux")
    } else {
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextWebsiteTarget {
    pub id: i64,
    pub context_id: i64,
    pub domain: String,
    pub created_at: String,
}

fn context_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Context> {
    Ok(Context {
        id: row.get(0)?,
        name: row.get(1)?,
        is_everywhere: row.get::<_, i64>(2)? != 0,
        icon: row.get(3)?,
        tone: row.get(4)?,
        cleanup_intensity: row.get(5)?,
        color: row.get(6)?,
        custom_instructions: row.get(7)?,
        contextual_formatting_disabled: row.get::<_, i64>(8)? != 0,
        paste_in_chunks: row.get::<_, i64>(12)? != 0,
        t3_skill_mentions_disabled: row.get::<_, i64>(13)? != 0,
        pinned_at: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        contextual_formatting_override: row.get::<_, Option<i64>>(14)?.map(|value| value != 0),
    })
}

fn normalize_context_name(name: &str) -> Result<String> {
    let normalized = require_nonempty_trimmed("Context name", name)?;
    validate_char_limit("Context name", &normalized, CONTEXT_NAME_CHAR_LIMIT)?;
    Ok(normalized)
}

fn normalize_custom_instructions(custom_instructions: Option<&str>) -> Result<Option<String>> {
    let Some(normalized) = normalize_optional_trimmed(custom_instructions) else {
        return Ok(None);
    };
    validate_char_limit(
        "Custom instructions",
        &normalized,
        CONTEXT_CUSTOM_INSTRUCTIONS_CHAR_LIMIT,
    )?;
    Ok(Some(normalized))
}

fn normalize_executable(executable: &str) -> Result<String> {
    let normalized = require_nonempty_trimmed("Executable", executable)?.to_lowercase();
    validate_char_limit("Executable", &normalized, CONTEXT_EXECUTABLE_CHAR_LIMIT)?;
    Ok(normalized)
}

fn normalize_domain(domain: &str) -> Result<String> {
    let trimmed = require_nonempty_trimmed("Website", domain)?.to_lowercase();
    let without_scheme = trimmed.split("://").last().unwrap_or(&trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let normalized = require_nonempty_trimmed("Website", host)?;
    validate_char_limit("Website", &normalized, CONTEXT_DOMAIN_CHAR_LIMIT)?;
    Ok(normalized)
}

pub(crate) fn ensure_everywhere_context_conn(conn: &rusqlite::Connection) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO contexts (id, name, is_everywhere) VALUES (?1, 'Everywhere', 1)",
        params![EVERYWHERE_CONTEXT_ID],
    )?;
    conn.query_row(
        "SELECT id FROM contexts WHERE is_everywhere = 1",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn everywhere_context_id(db: &Db) -> Result<i64> {
    let conn = lock_conn(db)?;
    ensure_everywhere_context_conn(&conn)
}

pub fn query_contexts(db: &Db) -> Result<Vec<Context>> {
    let conn = lock_conn(db)?;
    query_all(
        &conn,
        "SELECT id, name, is_everywhere, icon, tone, cleanup_intensity, color, custom_instructions, contextual_formatting_disabled, pinned_at, created_at, updated_at, paste_in_chunks, t3_skill_mentions_disabled, contextual_formatting_override
         FROM contexts
         ORDER BY id ASC",
        [], context_from_row,
    )
}

pub fn count_user_contexts(db: &Db) -> Result<i64> {
    let conn = lock_conn(db)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM contexts WHERE is_everywhere = 0",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn query_context(db: &Db, context_id: i64) -> Result<Context> {
    let conn = lock_conn(db)?;
    query_context_conn(&conn, context_id)
}

/// Returns instruction layers in application order: Everywhere first, then
/// the selected group. Empty layers are omitted so callers can join this with
/// snippet and protected instructions without introducing blank prompt gaps.
pub fn query_effective_context_instructions(db: &Db, context_id: i64) -> Result<Option<String>> {
    let conn = lock_conn(db)?;
    let context = query_context_conn(&conn, context_id)?;
    let everywhere_id = ensure_everywhere_context_conn(&conn)?;
    let everywhere_instructions: Option<String> = conn.query_row(
        "SELECT custom_instructions FROM contexts WHERE id = ?1",
        params![everywhere_id],
        |row| row.get(0),
    )?;
    let mut layers = Vec::with_capacity(2);
    if let Some(instructions) = everywhere_instructions
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        layers.push(instructions);
    }
    if context_id != everywhere_id {
        if let Some(instructions) = context
            .custom_instructions
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layers.push(instructions);
        }
    }
    Ok((!layers.is_empty()).then(|| layers.join("\n\n")))
}

pub fn query_dictionary_entry_contexts(db: &Db, term: &str) -> Result<Vec<ContextAssignment>> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    let conn = lock_conn(db)?;
    query_all(
        &conn,
        "SELECT c.id, c.name, c.is_everywhere
         FROM contexts c
         INNER JOIN dictionary_contexts dc ON dc.context_id = c.id
         INNER JOIN dictionary d ON d.id = dc.dictionary_id
         WHERE d.term = ?1
         ORDER BY c.is_everywhere DESC, c.name COLLATE NOCASE ASC",
        params![normalized_term],
        |row| {
            Ok(ContextAssignment {
                id: row.get(0)?,
                name: row.get(1)?,
                is_everywhere: row.get::<_, i64>(2)? != 0,
            })
        },
    )
}

pub fn query_snippet_entry_contexts(db: &Db, trigger: &str) -> Result<Vec<ContextAssignment>> {
    let normalized_trigger = require_nonempty_trimmed("Trigger", trigger)?;
    let conn = lock_conn(db)?;
    query_all(
        &conn,
        "SELECT c.id, c.name, c.is_everywhere
         FROM contexts c
         INNER JOIN snippet_contexts sc ON sc.context_id = c.id
         INNER JOIN snippets s ON s.id = sc.snippet_id
         WHERE s.trigger = ?1
         ORDER BY c.is_everywhere DESC, c.name COLLATE NOCASE ASC",
        params![normalized_trigger],
        |row| {
            Ok(ContextAssignment {
                id: row.get(0)?,
                name: row.get(1)?,
                is_everywhere: row.get::<_, i64>(2)? != 0,
            })
        },
    )
}

pub(super) fn query_context_conn(conn: &rusqlite::Connection, context_id: i64) -> Result<Context> {
    conn.query_row(
        "SELECT id, name, is_everywhere, icon, tone, cleanup_intensity, color, custom_instructions, contextual_formatting_disabled, pinned_at, created_at, updated_at, paste_in_chunks, t3_skill_mentions_disabled, contextual_formatting_override
         FROM contexts WHERE id = ?1",
        params![context_id],
        context_from_row,
    )
    .map_err(Into::into)
}

#[cfg(test)]
pub fn insert_context_returning(
    db: &Db,
    name: &str,
    icon: Option<&str>,
    tone: Option<&str>,
    cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>,
    contextual_formatting_disabled: bool,
) -> Result<Context> {
    insert_context_with_delivery(db, name, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_disabled, false)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn insert_context_with_delivery(
    db: &Db, name: &str, icon: Option<&str>, tone: Option<&str>, cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>, contextual_formatting_disabled: bool, paste_in_chunks: bool,
) -> Result<Context> {
    insert_context_with_override_and_delivery(db, name, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_disabled.then_some(true), paste_in_chunks)
}

#[allow(clippy::too_many_arguments)]
pub fn insert_context_with_override_and_delivery(
    db: &Db, name: &str, icon: Option<&str>, tone: Option<&str>, cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>, contextual_formatting_override: Option<bool>, paste_in_chunks: bool,
) -> Result<Context> {
    let normalized_name = normalize_context_name(name)?;
    if normalized_name.eq_ignore_ascii_case("Everywhere") {
        anyhow::bail!("The Everywhere context already exists");
    }
    let normalized_custom_instructions = normalize_custom_instructions(custom_instructions)?;

    let conn = lock_conn(db)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM contexts WHERE is_everywhere = 0",
        [],
        |row| row.get(0),
    )?;
    if count >= MAX_USER_CONTEXTS {
        anyhow::bail!("You've reached the limit of {MAX_USER_CONTEXTS} context groups");
    }
    conn.execute(
        "INSERT INTO contexts (name, is_everywhere, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_disabled, paste_in_chunks, contextual_formatting_override) VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            normalized_name,
            normalize_optional_trimmed(icon),
            normalize_optional_trimmed(tone),
            normalize_optional_trimmed(cleanup_intensity),
            normalized_custom_instructions,
            contextual_formatting_override == Some(true),
            paste_in_chunks,
            contextual_formatting_override,
        ],
    )?;
    let id = conn.last_insert_rowid();
    query_context_conn(&conn, id)
}

fn context_name_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE name = ?1)",
        params![name],
        |row| row.get(0),
    )?)
}

fn duplicate_context_name(conn: &Connection, source_name: &str) -> Result<String> {
    for copy_number in 1..10_000 {
        let suffix = if copy_number == 1 {
            " copy".to_string()
        } else {
            format!(" copy {copy_number}")
        };
        let base_len = CONTEXT_NAME_CHAR_LIMIT.saturating_sub(suffix.chars().count());
        let base: String = source_name
            .chars()
            .take(base_len)
            .collect::<String>()
            .trim_end()
            .to_string();
        let candidate = format!("{base}{suffix}");
        if !context_name_exists(conn, &candidate)? {
            return Ok(candidate);
        }
    }
    anyhow::bail!("Could not find an available name for the duplicated context")
}

pub fn duplicate_context(db: &Db, context_id: i64) -> Result<Context> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let source = query_context_conn(&tx, context_id)?;
    if source.is_everywhere {
        anyhow::bail!("The Everywhere context cannot be duplicated");
    }

    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM contexts WHERE is_everywhere = 0",
        [],
        |row| row.get(0),
    )?;
    if count >= MAX_USER_CONTEXTS {
        anyhow::bail!("You've reached the limit of {MAX_USER_CONTEXTS} context groups");
    }

    let name = duplicate_context_name(&tx, &source.name)?;
    tx.execute(
        "INSERT INTO contexts (
           name, is_everywhere, icon, tone, cleanup_intensity, color,
           custom_instructions, contextual_formatting_disabled, paste_in_chunks, t3_skill_mentions_disabled, contextual_formatting_override
         ) VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            name,
            source.icon,
            source.tone,
            source.cleanup_intensity,
            source.color,
            source.custom_instructions,
            source.contextual_formatting_disabled,
            source.paste_in_chunks,
            source.t3_skill_mentions_disabled,
            source.contextual_formatting_override,
        ],
    )?;
    let duplicate_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO dictionary_contexts (context_id, dictionary_id)
         SELECT ?1, dictionary_id FROM dictionary_contexts WHERE context_id = ?2",
        params![duplicate_id, context_id],
    )?;
    tx.execute(
        "INSERT INTO snippet_contexts (context_id, snippet_id)
         SELECT ?1, snippet_id FROM snippet_contexts WHERE context_id = ?2",
        params![duplicate_id, context_id],
    )?;
    tx.commit()?;
    query_context_conn(&conn, duplicate_id)
}

pub fn update_context(db: &Db, context_id: i64, name: &str) -> Result<()> {
    let normalized_name = normalize_context_name(name)?;
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET name = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![context_id, normalized_name],
    )?;
    require_row_changed(changed, "Context", context_id)
}

#[cfg(test)]
pub fn update_context_settings(
    db: &Db,
    context_id: i64,
    icon: Option<&str>,
    tone: Option<&str>,
    cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>,
    contextual_formatting_disabled: bool,
) -> Result<()> {
    update_context_settings_with_delivery(db, context_id, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_disabled, None)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn update_context_settings_with_delivery(
    db: &Db, context_id: i64, icon: Option<&str>, tone: Option<&str>, cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>, contextual_formatting_disabled: bool, paste_in_chunks: Option<bool>,
) -> Result<()> {
    update_context_settings_with_override_and_delivery(db, context_id, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_disabled.then_some(true), paste_in_chunks)
}

#[cfg(test)]
pub fn update_context_settings_with_override(
    db: &Db, context_id: i64, icon: Option<&str>, tone: Option<&str>, cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>, contextual_formatting_override: Option<bool>,
) -> Result<()> {
    update_context_settings_with_override_and_delivery(db, context_id, icon, tone, cleanup_intensity, custom_instructions, contextual_formatting_override, None)
}

#[allow(clippy::too_many_arguments)]
pub fn update_context_settings_with_override_and_delivery(
    db: &Db, context_id: i64, icon: Option<&str>, tone: Option<&str>, cleanup_intensity: Option<&str>,
    custom_instructions: Option<&str>, contextual_formatting_override: Option<bool>, paste_in_chunks: Option<bool>,
) -> Result<()> {
    let normalized_custom_instructions = normalize_custom_instructions(custom_instructions)?;
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET icon = ?2, tone = ?3, cleanup_intensity = ?4, custom_instructions = ?5, contextual_formatting_disabled = ?6, paste_in_chunks = COALESCE(?7, paste_in_chunks), contextual_formatting_override = ?8, updated_at = datetime('now') WHERE id = ?1",
        params![
            context_id,
            normalize_optional_trimmed(icon),
            normalize_optional_trimmed(tone),
            normalize_optional_trimmed(cleanup_intensity),
            normalized_custom_instructions,
            contextual_formatting_override == Some(true),
            paste_in_chunks,
            contextual_formatting_override,
        ],
    )?;
    require_row_changed(changed, "Context", context_id)
}

pub fn update_context_t3_skill_mentions(db: &Db, context_id: i64, enabled: bool) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET t3_skill_mentions_disabled = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![context_id, !enabled],
    )?;
    require_row_changed(changed, "Context", context_id)
}

#[cfg(test)]
pub fn update_context_paste_in_chunks(db: &Db, context_id: i64, enabled: bool) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET paste_in_chunks = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![context_id, enabled],
    )?;
    require_row_changed(changed, "Context", context_id)
}

pub fn update_context_color(db: &Db, context_id: i64, color: Option<&str>) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET color = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![context_id, normalize_optional_trimmed(color)],
    )?;
    require_row_changed(changed, "Context", context_id)
}

pub fn set_context_pinned(db: &Db, context_id: i64, pinned: bool) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE contexts SET pinned_at = CASE WHEN ?2 THEN datetime('now') ELSE NULL END WHERE id = ?1",
        params![context_id, pinned],
    )?;
    require_row_changed(changed, "Context", context_id)
}

pub fn delete_context(db: &Db, context_id: i64) -> Result<()> {
    let conn = lock_conn(db)?;
    let context = query_context_conn(&conn, context_id)?;
    if context.is_everywhere {
        anyhow::bail!("The Everywhere context cannot be deleted");
    }
    delete_context_conn(&conn, context_id)
}

pub fn delete_context_conn(conn: &Connection, context_id: i64) -> Result<()> {
    conn.execute_batch("SAVEPOINT delete_context")?;
    let result = delete_context_contents(conn, context_id);
    match result {
        Ok(()) => conn.execute_batch("RELEASE delete_context")?,
        Err(error) => {
            conn.execute_batch("ROLLBACK TO delete_context; RELEASE delete_context")?;
            return Err(error);
        }
    }
    Ok(())
}

fn delete_context_contents(tx: &Connection, context_id: i64) -> Result<()> {
    let everywhere_id = ensure_everywhere_context_conn(tx)?;
    move_dictionary_corrections_conn(tx, context_id, everywhere_id)?;
    tx.execute(
        "DELETE FROM pending_corrections WHERE context_id = ?1",
        params![context_id],
    )?;
    tx.execute(
        "DELETE FROM auto_learn_candidates WHERE context_id = ?1",
        params![context_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
         SELECT ?1, dictionary_id FROM dictionary_contexts WHERE context_id = ?2",
        params![everywhere_id, context_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id)
         SELECT ?1, snippet_id FROM snippet_contexts WHERE context_id = ?2",
        params![everywhere_id, context_id],
    )?;
    tx.execute(
        "UPDATE transcriptions SET context_id = ?1 WHERE context_id = ?2",
        params![everywhere_id, context_id],
    )?;
    let everywhere_uuid: String = tx.query_row(
        "SELECT uuid FROM contexts WHERE id = ?1",
        params![everywhere_id],
        |r| r.get(0),
    )?;
    let deleted_uuid: String = tx.query_row(
        "SELECT uuid FROM contexts WHERE id = ?1",
        params![context_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO transcription_context_daily_stats
           (day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
         SELECT day, ?1, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm
           FROM transcription_context_daily_stats
          WHERE context_uuid = ?2
         ON CONFLICT(day, context_uuid) DO UPDATE SET
           total_words = total_words + excluded.total_words,
           total_transcriptions = total_transcriptions + excluded.total_transcriptions,
           speaking_ms = speaking_ms + excluded.speaking_ms,
           wpm_sum = wpm_sum + excluded.wpm_sum,
           wpm_count = wpm_count + excluded.wpm_count,
           best_wpm = MAX(best_wpm, excluded.best_wpm)",
        params![everywhere_uuid, deleted_uuid],
    )?;
    tx.execute(
        "DELETE FROM transcription_context_daily_stats WHERE context_uuid = ?1",
        params![deleted_uuid],
    )?;
    tx.execute(
        "INSERT INTO transcription_hourly_stats (day, hour, context_id, total_words)
         SELECT day, hour, ?1, SUM(total_words)
           FROM transcription_hourly_stats
          WHERE context_id = ?2
          GROUP BY day, hour
         ON CONFLICT(day, hour, context_id) DO UPDATE
           SET total_words = total_words + excluded.total_words",
        params![everywhere_id, context_id],
    )?;
    tx.execute(
        "INSERT INTO provider_daily_stats
           (day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars)
         SELECT day, ?1, model, provider, task, SUM(calls), SUM(audio_ms), SUM(input_chars), SUM(output_chars)
           FROM provider_daily_stats
          WHERE context_id = ?2
          GROUP BY day, model, provider, task
         ON CONFLICT(day, context_id, model, provider, task) DO UPDATE SET
           calls = calls + excluded.calls,
           audio_ms = audio_ms + excluded.audio_ms,
           input_chars = input_chars + excluded.input_chars,
           output_chars = output_chars + excluded.output_chars",
        params![everywhere_id, context_id],
    )?;
    for sql in [
        "DELETE FROM transcription_hourly_stats WHERE context_id = ?1",
        "DELETE FROM provider_daily_stats WHERE context_id = ?1",
        "DELETE FROM context_targets WHERE context_id = ?1",
        "DELETE FROM context_website_targets WHERE context_id = ?1",
        "DELETE FROM dictionary_contexts WHERE context_id = ?1",
        "DELETE FROM snippet_contexts WHERE context_id = ?1",
    ] {
        tx.execute(sql, params![context_id])?;
    }
    let changed = tx.execute("DELETE FROM contexts WHERE id = ?1", params![context_id])?;
    require_row_changed(changed, "Context", context_id)?;
    Ok(())
}

fn context_target_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextTarget> {
    Ok(ContextTarget {
        id: row.get(0)?,
        context_id: row.get(1)?,
        executable: row.get(2)?,
        app_name: row.get(3)?,
        developer: row.get(4)?,
        platform: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub fn query_context_targets(db: &Db, context_id: Option<i64>) -> Result<Vec<ContextTarget>> {
    let conn = lock_conn(db)?;
    query_all(
        &conn,
        "SELECT id, context_id, executable, app_name, developer, platform, created_at
         FROM context_targets
         WHERE (?1 IS NULL OR context_id = ?1)
           AND (?2 IS NULL OR platform IS NULL OR platform = ?2)
           AND executable NOT LIKE '?::%'
         ORDER BY executable COLLATE NOCASE ASC",
        params![context_id, current_platform_tag()],
        context_target_from_row,
    )
}

#[cfg(test)]
pub fn assign_context_target(db: &Db, context_id: i64, executable: &str) -> Result<ContextTarget> {
    assign_context_target_with_metadata(db, context_id, executable, None, None)
}

pub fn assign_context_target_with_metadata(
    db: &Db,
    context_id: i64,
    executable: &str,
    app_name: Option<&str>,
    developer: Option<&str>,
) -> Result<ContextTarget> {
    let normalized_executable = normalize_executable(executable)?;
    let normalized_app_name = normalize_optional_trimmed(app_name);
    let normalized_developer = normalize_optional_trimmed(developer);
    let mut conn = lock_conn(db)?;
    let context = query_context_conn(&conn, context_id)?;
    if context.is_everywhere {
        anyhow::bail!("The Everywhere context cannot have executable targets");
    }

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO context_targets (context_id, executable, app_name, developer, platform) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(executable) DO UPDATE SET context_id = excluded.context_id, app_name = excluded.app_name, developer = excluded.developer, platform = excluded.platform",
        params![
            context_id,
            normalized_executable,
            normalized_app_name,
            normalized_developer,
            current_platform_tag()
        ],
    )?;
    let target = tx.query_row(
        "SELECT id, context_id, executable, app_name, developer, platform, created_at
         FROM context_targets WHERE executable = ?1",
        params![normalized_executable],
        context_target_from_row,
    )?;
    tx.commit()?;
    Ok(target)
}

pub fn reconcile_context_targets(
    db: &Db,
    installed_apps: &[crate::system::apps::InstalledApp],
) -> Result<bool> {
    let conn = lock_conn(db)?;
    let current_platform = current_platform_tag();
    let rows: Vec<ContextTargetRow> = conn
        .prepare(
            "SELECT id, context_id, executable, app_name, developer, platform
             FROM context_targets
             WHERE platform IS NULL OR platform = ?1",
        )?
        .query_map(params![current_platform], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = false;

    for (id, _context_id, executable, app_name, developer, platform) in rows {
        if executable.starts_with("?::") {
            continue;
        }
        let exact = installed_apps
            .iter()
            .find(|app| app.exe.trim().eq_ignore_ascii_case(executable.trim()));
        let candidate = exact.or_else(|| {
            crate::system::apps::closest_installed_app(
                &executable,
                app_name.as_deref(),
                developer.as_deref(),
                installed_apps,
            )
        });
        let Some(candidate) = candidate else { continue };
        if exact.is_none() && candidate.exe.eq_ignore_ascii_case(&executable) {
            continue;
        }
        let next_executable = if exact.is_some() {
            executable.clone()
        } else {
            candidate.exe.trim().to_lowercase()
        };
        let already_owned: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM context_targets WHERE executable = ?1 AND id <> ?2)",
            params![next_executable, id],
            |row| row.get(0),
        )?;
        if already_owned {
            continue;
        }
        let next_app_name = normalize_optional_trimmed(Some(candidate.name.as_str()));
        let next_developer = normalize_optional_trimmed(candidate.developer.as_deref());
        let next_platform = platform.clone().or(current_platform.map(str::to_string));
        if next_executable == executable
            && next_app_name == app_name
            && next_developer == developer
            && next_platform == platform
        {
            continue;
        }
        let updated = conn.execute(
            "UPDATE context_targets
             SET executable = ?1, app_name = ?2, developer = ?3, platform = ?4
             WHERE id = ?5",
            params![
                next_executable,
                next_app_name,
                next_developer,
                next_platform,
                id
            ],
        )?;
        changed |= updated > 0;
    }
    Ok(changed)
}

pub fn remove_context_target(db: &Db, context_id: i64, executable: &str) -> Result<()> {
    let normalized_executable = normalize_executable(executable)?;
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "DELETE FROM context_targets WHERE context_id = ?1 AND executable = ?2",
        params![context_id, normalized_executable],
    )?;
    require_row_changed(changed, "Context target", context_id)
}

pub fn query_context_website_targets(
    db: &Db,
    context_id: Option<i64>,
) -> Result<Vec<ContextWebsiteTarget>> {
    let conn = lock_conn(db)?;
    query_all(
        &conn,
        "SELECT id, context_id, domain, created_at
         FROM context_website_targets
         WHERE (?1 IS NULL OR context_id = ?1)
         ORDER BY domain COLLATE NOCASE ASC",
        params![context_id],
        |row| {
            Ok(ContextWebsiteTarget {
                id: row.get(0)?,
                context_id: row.get(1)?,
                domain: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
}

pub fn assign_context_website(
    db: &Db,
    context_id: i64,
    domain: &str,
) -> Result<ContextWebsiteTarget> {
    let normalized_domain = normalize_domain(domain)?;
    let mut conn = lock_conn(db)?;
    let context = query_context_conn(&conn, context_id)?;
    if context.is_everywhere {
        anyhow::bail!("The Everywhere context cannot have website targets");
    }

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO context_website_targets (context_id, domain) VALUES (?1, ?2)
         ON CONFLICT(domain) DO UPDATE SET context_id = excluded.context_id",
        params![context_id, normalized_domain],
    )?;
    let target = tx.query_row(
        "SELECT id, context_id, domain, created_at
         FROM context_website_targets WHERE domain = ?1",
        params![normalized_domain],
        |row| {
            Ok(ContextWebsiteTarget {
                id: row.get(0)?,
                context_id: row.get(1)?,
                domain: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )?;
    tx.commit()?;
    Ok(target)
}

pub fn remove_context_website(db: &Db, context_id: i64, domain: &str) -> Result<()> {
    let normalized_domain = normalize_domain(domain)?;
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "DELETE FROM context_website_targets WHERE context_id = ?1 AND domain = ?2",
        params![context_id, normalized_domain],
    )?;
    require_row_changed(changed, "Context website", context_id)
}

#[cfg(any(feature = "dev-session", test))]
pub fn resolve_context_for_target(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
) -> Result<Context> {
    resolve_context_with_sub_app(db, executable, domain, None).map(|(context, _)| context)
}

pub fn resolve_context_with_sub_app(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
    window_title: Option<&str>,
) -> Result<(Context, Option<ContextSubApp>)> {
    let conn = lock_conn(db)?;
    if let Some(sub_app) = window_title
        .map(|title| resolve_sub_app_conn(&conn, executable, title))
        .transpose()?
        .flatten()
    {
        if let Some(context_id) = sub_app.context_id {
            let context = query_context_conn(&conn, context_id)?;
            return Ok((context, Some(sub_app)));
        }
    }
    resolve_context_without_sub_app_conn(&conn, executable, domain).map(|context| (context, None))
}

fn resolve_context_without_sub_app_conn(
    conn: &rusqlite::Connection,
    executable: &str,
    domain: Option<&str>,
) -> Result<Context> {
    let everywhere_id = ensure_everywhere_context_conn(conn)?;
    let normalized_executable = executable.trim().to_lowercase();
    let normalized_domain = domain
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_lowercase);

    if let Some(domain) = &normalized_domain {
        let context_id: Option<i64> = conn
            .query_row(
                "SELECT context_id FROM context_website_targets WHERE domain = ?1",
                params![domain],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(context_id) = context_id {
            return query_context_conn(conn, context_id);
        }
    }

    if normalized_executable.is_empty() {
        return query_context_conn(conn, everywhere_id);
    }

    let context_id: Option<i64> = conn
        .query_row(
            "SELECT context_id FROM context_targets WHERE executable = ?1",
            params![normalized_executable],
            |row| row.get(0),
        )
        .optional()?;
    query_context_conn(conn, context_id.unwrap_or(everywhere_id))
}

pub fn set_dictionary_context_assignment(
    db: &Db,
    context_id: i64,
    dictionary_id: i64,
    assigned: bool,
) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    query_context_conn(&tx, context_id)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM dictionary WHERE id = ?1)",
        params![dictionary_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Dictionary entry {dictionary_id} was not found");
    }
    if assigned {
        tx.execute(
            "UPDATE dictionary SET mistake = NULL WHERE id = ?1 AND mistake IS NOT NULL",
            params![dictionary_id],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
             VALUES (?1, ?2)",
            params![context_id, dictionary_id],
        )?;
    } else {
        remove_dictionary_corrections_for_context_conn(&tx, context_id, dictionary_id)?;
        tx.execute(
            "DELETE FROM dictionary_contexts WHERE context_id = ?1 AND dictionary_id = ?2",
            params![context_id, dictionary_id],
        )?;
        cleanup_orphaned_auto_dictionary_conn(&tx, context_id, dictionary_id)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set_snippet_context_assignment(
    db: &Db,
    context_id: i64,
    snippet_id: i64,
    assigned: bool,
) -> Result<()> {
    let conn = lock_conn(db)?;
    query_context_conn(&conn, context_id)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM snippets WHERE id = ?1)",
        params![snippet_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Snippet {snippet_id} was not found");
    }
    if assigned {
        conn.execute(
            "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id)
             VALUES (?1, ?2)",
            params![context_id, snippet_id],
        )?;
    } else {
        conn.execute(
            "DELETE FROM snippet_contexts WHERE context_id = ?1 AND snippet_id = ?2",
            params![context_id, snippet_id],
        )?;
    }
    invalidate_snippet_cache();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_assigns_existing_content_to_everywhere() {
        let db = open(":memory:").expect("db");
        insert_dictionary_entry(&db, "Verenu", Some("Varinu")).expect("dictionary");
        insert_snippet(&db, "sig", "signature", "").expect("snippet");

        let everywhere = query_context(&db, EVERYWHERE_CONTEXT_ID).expect("Everywhere");
        assert!(everywhere.is_everywhere);
        assert_eq!(
            query_dictionary_for_context(&db, everywhere.id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            query_snippets_for_context(&db, everywhere.id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn version_11_content_is_assigned_during_context_migration() {
        let path = std::env::temp_dir().join(format!(
            "verenu_context_migration_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        {
            let conn = rusqlite::Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                "CREATE TABLE dictionary (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   term TEXT NOT NULL UNIQUE,
                   mistake TEXT,
                   auto_learned INTEGER NOT NULL DEFAULT 0,
                   correction_count INTEGER NOT NULL DEFAULT 0,
                   confidence_tier TEXT NOT NULL DEFAULT 'low',
                   last_seen_at DATETIME,
                   created_at DATETIME NOT NULL DEFAULT (datetime('now'))
                 );
                 CREATE TABLE snippets (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   trigger TEXT NOT NULL UNIQUE,
                   expansion TEXT NOT NULL,
                   instructions TEXT NOT NULL DEFAULT '',
                   use_count INTEGER NOT NULL DEFAULT 0,
                   created_at DATETIME NOT NULL DEFAULT (datetime('now'))
                 );
                 INSERT INTO dictionary (term, mistake) VALUES ('Verenu', 'Varinu');
                 INSERT INTO snippets (trigger, expansion) VALUES ('sig', 'signature');
                 PRAGMA user_version = 11;",
            )
            .expect("seed legacy content");
        }

        let db = open(&path).expect("migrate legacy db");
        assert_eq!(
            query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            query_snippets_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn paste_chunks_context_defaults_updates_and_duplicates() {
        let db = open(":memory:").unwrap();
        assert!(!query_context(&db, EVERYWHERE_CONTEXT_ID).unwrap().paste_in_chunks);
        let context = insert_context_with_delivery(&db, "Terminal chunks", None, None, None, None, false, true).unwrap();
        assert!(context.paste_in_chunks);
        update_context_settings_with_delivery(&db, context.id, None, None, None, None, false, None).unwrap();
        assert!(query_context(&db, context.id).unwrap().paste_in_chunks);
        assert!(duplicate_context(&db, context.id).unwrap().paste_in_chunks);
        update_context_settings_with_delivery(&db, context.id, None, None, None, None, false, Some(false)).unwrap();
        assert!(!query_context(&db, context.id).unwrap().paste_in_chunks);
    }

    #[test]
    fn version_32_migration_preserves_context_data_and_translates_formatting_overrides() {
        let path = std::env::temp_dir().join(format!(
            "verenu_context_v33_migration_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let (enabled_id, disabled_id) = {
            let db = open(&path).expect("create current db");
            let enabled = insert_context_returning(
                &db,
                "Legacy follow",
                None,
                Some("formal"),
                Some("high"),
                Some("Global rule"),
                false,
            )
            .expect("context following global formatting");
            let disabled = insert_context_returning(
                &db,
                "Legacy disabled",
                None,
                Some("casual"),
                Some("low"),
                Some("Local rule"),
                true,
            )
            .expect("context with formatting disabled");
            update_context_paste_in_chunks(&db, disabled.id, true).unwrap();
            update_context_t3_skill_mentions(&db, disabled.id, false).unwrap();
            insert_dictionary_entry(&db, "Verenu", Some("Varinu")).expect("dictionary row");
            insert_snippet(&db, "sig", "signature", "").expect("snippet row");

            let conn = lock_conn(&db).expect("lock current db");
            // Recreate the v32 state: the nullable column did not exist, and
            // the legacy boolean carried only an explicit disable.
            conn.execute_batch(
                "ALTER TABLE contexts DROP COLUMN contextual_formatting_override;
                 PRAGMA user_version = 32;",
            )
            .expect("prepare v32 state");
            drop(conn);
            drop(db);
            (enabled.id, disabled.id)
        };

        let migrated = open(&path).expect("run v33 migration");
        let follows_global = query_context(&migrated, enabled_id).expect("follow context");
        let disabled = query_context(&migrated, disabled_id).expect("disabled context");
        assert_eq!(follows_global.contextual_formatting_override, None);
        assert_eq!(disabled.contextual_formatting_override, Some(true));
        assert!(disabled.paste_in_chunks);
        assert!(disabled.t3_skill_mentions_disabled);
        assert_eq!(follows_global.tone.as_deref(), Some("formal"));
        assert_eq!(follows_global.cleanup_intensity.as_deref(), Some("high"));
        assert_eq!(
            follows_global.custom_instructions.as_deref(),
            Some("Global rule")
        );
        assert_eq!(disabled.tone.as_deref(), Some("casual"));
        assert_eq!(disabled.cleanup_intensity.as_deref(), Some("low"));
        assert_eq!(disabled.custom_instructions.as_deref(), Some("Local rule"));
        assert_eq!(
            query_dictionary(&migrated)
                .expect("dictionary preserved")
                .len(),
            1
        );
        assert_eq!(
            query_snippets(&migrated).expect("snippets preserved").len(),
            1
        );

        drop(migrated);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn custom_instructions_round_trip_and_enforce_char_limit() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(
            &db,
            "Support",
            None,
            None,
            None,
            Some("  Reply in a friendly tone.  "),
            false,
        )
        .expect("context");
        assert_eq!(
            context.custom_instructions.as_deref(),
            Some("Reply in a friendly tone.")
        );

        update_context_settings(
            &db,
            context.id,
            None,
            None,
            None,
            Some("Keep it brief."),
            false,
        )
        .expect("update");
        assert_eq!(
            query_context(&db, context.id)
                .unwrap()
                .custom_instructions
                .as_deref(),
            Some("Keep it brief.")
        );

        update_context_settings(&db, context.id, None, None, None, None, false).expect("clear");
        assert_eq!(
            query_context(&db, context.id).unwrap().custom_instructions,
            None
        );

        let too_long = "x".repeat(CONTEXT_CUSTOM_INSTRUCTIONS_CHAR_LIMIT + 1);
        assert!(
            update_context_settings(&db, context.id, None, None, None, Some(&too_long), false)
                .is_err()
        );
    }

    #[test]
    fn contextual_formatting_override_defaults_off_and_round_trips() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Terminal", None, None, None, None, false)
            .expect("context");
        assert!(!context.contextual_formatting_disabled);

        update_context_settings(&db, context.id, None, None, None, None, true)
            .expect("disable formatting");
        assert!(
            query_context(&db, context.id)
                .expect("updated context")
                .contextual_formatting_disabled
        );
    }

    #[test]
    fn content_assignment_is_scoped_and_reversible() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("context");
        insert_dictionary_entry(&db, "Verenu", Some("Varinu")).expect("dictionary");
        insert_snippet(&db, "sig", "signature", "").expect("snippet");
        let dictionary_id = query_dictionary(&db).unwrap()[0].id;
        let snippet_id = query_snippets(&db).unwrap()[0].id;

        assert!(query_dictionary_for_context(&db, context.id)
            .unwrap()
            .is_empty());
        assert!(query_snippets_for_context(&db, context.id)
            .unwrap()
            .is_empty());

        set_dictionary_context_assignment(&db, context.id, dictionary_id, true).unwrap();
        set_snippet_context_assignment(&db, context.id, snippet_id, true).unwrap();
        assert_eq!(
            query_dictionary_for_context(&db, context.id).unwrap().len(),
            1
        );
        let context_entry = query_dictionary_for_context(&db, context.id)
            .expect("context dictionary")
            .into_iter()
            .next()
            .expect("shared canonical entry");
        assert!(
            context_entry.corrections.is_empty(),
            "sharing a canonical term must not copy another Context's correction mapping"
        );
        assert!(context_entry.mistake.is_none());
        assert_eq!(
            query_snippets_for_context(&db, context.id).unwrap().len(),
            1
        );

        set_dictionary_context_assignment(&db, context.id, dictionary_id, false).unwrap();
        set_snippet_context_assignment(&db, context.id, snippet_id, false).unwrap();
        assert!(query_dictionary_for_context(&db, context.id)
            .unwrap()
            .is_empty());
        assert!(query_snippets_for_context(&db, context.id)
            .unwrap()
            .is_empty());
        assert_eq!(
            query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            query_snippets_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn effective_content_inherits_everywhere_and_clears_local_assignments() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("context");
        insert_dictionary_entry(&db, "Verenu", Some("Varinu")).expect("global vocabulary");
        let dictionary_id = query_dictionary(&db).unwrap()[0].id;

        let inherited =
            query_effective_dictionary_for_context(&db, context.id).expect("effective dictionary");
        assert_eq!(inherited.len(), 1);
        assert!(inherited[0].inherited_from_everywhere);
        assert!(!inherited[0].assigned_to_context);
        assert_eq!(inherited[0].mistake.as_deref(), Some("Varinu"));

        assert!(update_dictionary_entry_for_context(
            &db,
            context.id,
            dictionary_id,
            "Renamed globally",
            Some("Vreenu"),
        )
        .is_err());
        assert!(
            !query_effective_dictionary_for_context(&db, context.id).unwrap()[0]
                .assigned_to_context
        );
        assert_eq!(query_dictionary(&db).unwrap()[0].term, "Verenu");

        update_dictionary_entry_for_context(
            &db,
            context.id,
            dictionary_id,
            "Verenu",
            Some("VARINU, Vreenu"),
        )
        .expect("customize inherited correction locally");
        let customized = query_effective_dictionary_for_context(&db, context.id)
            .expect("effective customized dictionary");
        let corrections = &customized[0].corrections;
        assert!(customized[0].inherited_from_everywhere);
        assert!(customized[0].assigned_to_context);
        assert_eq!(corrections.len(), 2);
        assert!(corrections
            .iter()
            .any(|item| item.mistake == "VARINU" && item.context_id == context.id));
        assert!(corrections
            .iter()
            .any(|item| item.mistake == "Vreenu" && item.context_id == context.id));
        assert_eq!(
            query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID).unwrap()[0]
                .mistake
                .as_deref(),
            Some("Varinu")
        );

        set_dictionary_context_assignment(&db, context.id, dictionary_id, false)
            .expect("clear local vocabulary override");
        let cleared = query_effective_dictionary_for_context(&db, context.id)
            .expect("effective dictionary after clearing local override");
        assert_eq!(cleared.len(), 1);
        assert!(cleared[0].inherited_from_everywhere);
        assert!(!cleared[0].assigned_to_context);
        assert_eq!(cleared[0].mistake.as_deref(), Some("Varinu"));

        insert_snippet(&db, "thanks", "Thank you.", "Use global style.").expect("global snippet");
        let global_snippet = query_snippets(&db).unwrap()[0].clone();
        let local_snippet = insert_snippet_returning(
            &db,
            "status",
            "In progress.",
            "Use local style.",
            Some(context.id),
        )
        .expect("local snippet");
        let effective =
            query_effective_snippets_for_context(&db, context.id).expect("effective snippets");
        assert_eq!(effective.len(), 2);
        assert_eq!(
            crate::data::snippets::collect_snippet_instructions_from("thanks status", &effective),
            "Use global style.\nUse local style."
        );
        let inherited = effective
            .iter()
            .find(|item| item.id == global_snippet.id)
            .unwrap();
        assert!(inherited.inherited_from_everywhere);
        assert!(!inherited.assigned_to_context);
        let local = effective
            .iter()
            .find(|item| item.id == local_snippet.id)
            .unwrap();
        assert!(!local.inherited_from_everywhere);
        assert!(local.assigned_to_context);

        set_snippet_context_assignment(&db, context.id, global_snippet.id, true).unwrap();
        let assigned = query_effective_snippets_for_context(&db, context.id)
            .expect("explicitly assigned inherited snippet");
        let global_in_context = assigned
            .iter()
            .find(|item| item.id == global_snippet.id)
            .unwrap();
        assert!(global_in_context.inherited_from_everywhere);
        assert!(global_in_context.assigned_to_context);
        set_snippet_context_assignment(&db, context.id, global_snippet.id, false).unwrap();
        let cleared = query_effective_snippets_for_context(&db, context.id)
            .expect("inherited snippet after clearing local assignment");
        let global_in_context = cleared
            .iter()
            .find(|item| item.id == global_snippet.id)
            .unwrap();
        assert!(global_in_context.inherited_from_everywhere);
        assert!(!global_in_context.assigned_to_context);
    }

    #[test]
    fn local_spoken_variant_overrides_a_different_global_term_until_cleared() {
        let db = open(":memory:").expect("db");
        let group =
            insert_context_returning(&db, "Development", None, None, None, None, false).unwrap();
        insert_dictionary_entry(&db, "GlobalTool", Some("tool-x")).unwrap();
        insert_dictionary_entry_returning(&db, "LocalTool", Some("TOOL-X"), Some(group.id))
            .unwrap();
        let effective = query_effective_dictionary_for_context(&db, group.id).unwrap();
        let global = effective
            .iter()
            .find(|entry| entry.term == "GlobalTool")
            .unwrap();
        assert!(global.corrections.is_empty());
        assert!(
            global.mistake.is_none(),
            "shadowed mappings must not survive through the legacy fallback"
        );
        let (text, _) = crate::data::dictionary::apply_substitutions_from("Use tool-x", &effective);
        assert_eq!(text, "Use LocalTool");
        let local_id = effective
            .iter()
            .find(|entry| entry.term == "LocalTool")
            .unwrap()
            .id;
        set_dictionary_context_assignment(&db, group.id, local_id, false).unwrap();
        let inherited = query_effective_dictionary_for_context(&db, group.id).unwrap();
        let (text, _) = crate::data::dictionary::apply_substitutions_from("Use tool-x", &inherited);
        assert_eq!(text, "Use GlobalTool");
        let everywhere =
            query_effective_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID).unwrap();
        assert_eq!(everywhere[0].mistake.as_deref(), Some("tool-x"));
    }

    #[test]
    fn context_instructions_run_global_then_local_and_can_be_cleared() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(
            &db,
            "Support",
            None,
            None,
            None,
            Some("Use the team's style."),
            false,
        )
        .expect("context");
        update_context_settings_with_override(
            &db,
            EVERYWHERE_CONTEXT_ID,
            None,
            None,
            None,
            Some("Be concise."),
            None,
        )
        .expect("global instructions");
        assert_eq!(
            query_effective_context_instructions(&db, context.id)
                .unwrap()
                .as_deref(),
            Some("Be concise.\n\nUse the team's style.")
        );
        update_context_settings_with_override(&db, context.id, None, None, None, None, None)
            .expect("clear local instructions");
        assert_eq!(
            query_effective_context_instructions(&db, context.id)
                .unwrap()
                .as_deref(),
            Some("Be concise.")
        );
    }

    #[test]
    fn removing_a_dictionary_assignment_purges_only_that_context_mapping_and_evidence() {
        let db = open(":memory:").expect("db");
        let first = insert_context_returning(&db, "Development", None, None, None, None, false)
            .expect("first context");
        let second = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("second context");

        insert_dictionary_entry_auto_learned_for_context(
            &db,
            first.id,
            "Kubernetes",
            Some("kubernetez"),
            "high",
        )
        .expect("first mapping");
        let dictionary_id = query_dictionary(&db)
            .expect("dictionary")
            .into_iter()
            .find(|entry| entry.term == "Kubernetes")
            .expect("canonical entry")
            .id;
        insert_dictionary_entry_auto_learned_for_context(
            &db,
            second.id,
            "Kubernetes",
            Some("kubernetez"),
            "high",
        )
        .expect("second mapping");

        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
                 VALUES (?1, 'kubernetez', 'Kubernetes'), (?2, 'kubernetez', 'Kubernetes')",
                params![first.id, second.id],
            )
            .expect("pending evidence");
            conn.execute(
                "INSERT INTO auto_learn_candidates
                   (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
                 VALUES (?1, 'kubernetez', 'Kubernetes', 0.9, 0.9, 1),
                        (?2, 'kubernetez', 'Kubernetes', 0.9, 0.9, 1)",
                params![first.id, second.id],
            )
            .expect("candidate evidence");
        }

        set_dictionary_context_assignment(&db, first.id, dictionary_id, false)
            .expect("remove first assignment");

        assert!(query_dictionary_for_context(&db, first.id)
            .expect("first dictionary")
            .is_empty());
        let second_entry = query_dictionary_for_context(&db, second.id)
            .expect("second dictionary")
            .into_iter()
            .find(|entry| entry.id == dictionary_id)
            .expect("second mapping remains");
        assert_eq!(second_entry.corrections.len(), 1);

        let conn = lock_conn(&db).expect("lock");
        let first_pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_corrections WHERE context_id = ?1",
                params![first.id],
                |row| row.get(0),
            )
            .expect("first pending count");
        let second_pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_corrections WHERE context_id = ?1",
                params![second.id],
                |row| row.get(0),
            )
            .expect("second pending count");
        assert_eq!(first_pending, 0);
        assert_eq!(second_pending, 1);
    }

    #[test]
    fn rejecting_a_mapping_is_scoped_to_its_context_and_mapping_id() {
        let db = open(":memory:").expect("db");
        let first = insert_context_returning(&db, "Development", None, None, None, None, false)
            .expect("first context");
        let second = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("second context");

        insert_dictionary_entry_auto_learned_for_context(
            &db,
            first.id,
            "Groq",
            Some("grockx"),
            "high",
        )
        .expect("first mapping");
        insert_dictionary_entry_auto_learned_for_context(
            &db,
            second.id,
            "Groq",
            Some("rockz"),
            "high",
        )
        .expect("second mapping");
        let dictionary_id = query_dictionary(&db)
            .expect("dictionary")
            .into_iter()
            .find(|entry| entry.term == "Groq")
            .expect("canonical entry")
            .id;
        let first_mapping_id = query_dictionary_for_context(&db, first.id)
            .expect("first dictionary")
            .into_iter()
            .find(|entry| entry.id == dictionary_id)
            .and_then(|entry| entry.corrections.into_iter().next())
            .expect("first correction")
            .id;
        let second_mapping_id = query_dictionary_for_context(&db, second.id)
            .expect("second dictionary")
            .into_iter()
            .find(|entry| entry.id == dictionary_id)
            .and_then(|entry| entry.corrections.into_iter().next())
            .expect("second correction")
            .id;

        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
                 VALUES (?1, 'grockx', 'Groq'), (?2, 'rockz', 'Groq')",
                params![first.id, second.id],
            )
            .expect("pending evidence");
            conn.execute(
                "INSERT INTO auto_learn_candidates
                   (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
                 VALUES (?1, 'grockx', 'Groq', 0.9, 0.9, 1),
                        (?2, 'rockz', 'Groq', 0.9, 0.9, 1)",
                params![first.id, second.id],
            )
            .expect("candidate evidence");
        }

        assert_eq!(
            delete_auto_learned_corrections_by_ids(&db, first.id, &[first_mapping_id])
                .expect("reject first mapping"),
            1
        );
        assert_eq!(
            delete_auto_learned_corrections_by_ids(&db, first.id, &[second_mapping_id])
                .expect("wrong-context rejection is ignored"),
            0
        );

        let first_entry = query_dictionary_for_context(&db, first.id)
            .expect("first dictionary after rejection")
            .into_iter()
            .find(|entry| entry.id == dictionary_id)
            .expect("canonical row remains assigned");
        assert!(first_entry.corrections.is_empty());
        assert!(first_entry.mistake.is_none());

        let second_entry = query_dictionary_for_context(&db, second.id)
            .expect("second dictionary after rejection")
            .into_iter()
            .find(|entry| entry.id == dictionary_id)
            .expect("second mapping remains");
        assert_eq!(second_entry.corrections.len(), 1);
        assert_eq!(second_entry.corrections[0].id, second_mapping_id);
        assert_eq!(second_entry.corrections[0].mistake, "rockz");

        let conn = lock_conn(&db).expect("lock");
        let first_candidates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM auto_learn_candidates WHERE context_id = ?1",
                params![first.id],
                |row| row.get(0),
            )
            .expect("first candidates");
        let second_candidates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM auto_learn_candidates WHERE context_id = ?1",
                params![second.id],
                |row| row.get(0),
            )
            .expect("second candidates");
        assert_eq!(first_candidates, 0);
        assert_eq!(second_candidates, 1);
    }

    #[test]
    fn rejecting_the_last_auto_mapping_removes_only_its_orphaned_canonical() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Development", None, None, None, None, false)
            .expect("context");
        insert_dictionary_entry_auto_learned_for_context(
            &db,
            context.id,
            "Kubernetes",
            Some("kubernetez"),
            "high",
        )
        .expect("auto mapping");
        let entry = query_dictionary_for_context(&db, context.id)
            .expect("context dictionary")
            .into_iter()
            .next()
            .expect("entry");
        let correction_id = entry.corrections[0].id;

        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
                 VALUES (?1, 'kubernetez', 'Kubernetes')",
                params![context.id],
            )
            .expect("pending evidence");
            conn.execute(
                "INSERT INTO auto_learn_candidates
                   (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
                 VALUES (?1, 'kubernetez', 'Kubernetes', 0.9, 0.9, 1)",
                params![context.id],
            )
            .expect("candidate evidence");
        }

        assert_eq!(
            delete_auto_learned_corrections_by_ids(&db, context.id, &[correction_id])
                .expect("reject mapping"),
            1
        );
        assert!(query_dictionary(&db)
            .expect("global dictionary")
            .into_iter()
            .all(|entry| entry.term != "Kubernetes"));

        let conn = lock_conn(&db).expect("lock");
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_corrections WHERE context_id = ?1",
                params![context.id],
                |row| row.get(0),
            )
            .expect("pending count");
        let candidates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM auto_learn_candidates WHERE context_id = ?1",
                params![context.id],
                |row| row.get(0),
            )
            .expect("candidate count");
        assert_eq!(pending, 0);
        assert_eq!(candidates, 0);
    }

    #[test]
    fn manual_mapping_is_context_local_and_protected_from_auto_learn() {
        let db = open(":memory:").expect("db");
        let manual_context =
            insert_context_returning(&db, "Writing", None, None, None, None, false)
                .expect("manual context");
        let auto_context =
            insert_context_returning(&db, "Development", None, None, None, None, false)
                .expect("auto context");

        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
                 VALUES (?1, 'user typo', 'Kubernetes')",
                params![manual_context.id],
            )
            .expect("pending evidence");
            conn.execute(
                "INSERT INTO auto_learn_candidates
                   (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
                 VALUES (?1, 'user typo', 'Kubernetes', 0.9, 0.9, 1)",
                params![manual_context.id],
            )
            .expect("candidate evidence");
        }

        let manual = insert_dictionary_entry_returning(
            &db,
            "Kubernetes",
            Some("user typo"),
            Some(manual_context.id),
        )
        .expect("manual mapping");
        let conn = lock_conn(&db).expect("lock");
        let stale_evidence: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM pending_corrections WHERE context_id = ?1)
                        + (SELECT COUNT(*) FROM auto_learn_candidates WHERE context_id = ?1)",
                params![manual_context.id],
                |row| row.get(0),
            )
            .expect("stale evidence count");
        assert_eq!(stale_evidence, 0);
        drop(conn);
        assert!(!insert_dictionary_entry_auto_learned_for_context(
            &db,
            manual_context.id,
            "Kubernetes",
            Some("user typo"),
            "high",
        )
        .expect("same-context auto attempt"));
        insert_dictionary_entry_auto_learned_for_context(
            &db,
            auto_context.id,
            "Kubernetes",
            Some("koobernetes"),
            "high",
        )
        .expect("other-context auto mapping");

        let manual_entry = query_dictionary_for_context(&db, manual_context.id)
            .expect("manual dictionary")
            .into_iter()
            .find(|entry| entry.id == manual.id)
            .expect("manual entry");
        assert_eq!(manual_entry.corrections.len(), 1);
        assert!(!manual_entry.corrections[0].auto_learned);
        assert_eq!(manual_entry.corrections[0].mistake, "user typo");
        assert_eq!(
            delete_auto_learned_corrections_by_ids(
                &db,
                manual_context.id,
                &[manual_entry.corrections[0].id],
            )
            .expect("manual rejection is ignored"),
            0
        );

        let auto_entry = query_dictionary_for_context(&db, auto_context.id)
            .expect("auto dictionary")
            .into_iter()
            .find(|entry| entry.id == manual.id)
            .expect("shared canonical entry");
        assert_eq!(auto_entry.corrections.len(), 1);
        assert!(auto_entry.corrections[0].auto_learned);
        assert_eq!(auto_entry.corrections[0].mistake, "koobernetes");
    }

    #[test]
    fn duplicate_comma_variants_materialize_as_one_mapping() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Development", None, None, None, None, false)
            .expect("context");
        let entry = insert_dictionary_entry_returning(
            &db,
            "Kubernetes",
            Some("Kubernetez, kubernetez,  Kubernetez "),
            Some(context.id),
        )
        .expect("dictionary");

        let materialized = query_dictionary_for_context(&db, context.id)
            .expect("context dictionary")
            .into_iter()
            .find(|row| row.id == entry.id)
            .expect("entry");
        assert_eq!(materialized.corrections.len(), 1);
        assert_eq!(materialized.corrections[0].mistake, "Kubernetez");
    }

    #[test]
    fn target_resolution_returns_one_context_and_falls_back_to_everywhere() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("context");
        assign_context_target(&db, context.id, "Code.EXE").expect("target");

        assert_eq!(
            resolve_context_for_target(&db, "code.exe", None)
                .expect("resolve")
                .id,
            context.id
        );
        assert_eq!(
            resolve_context_for_target(&db, "unknown.exe", None)
                .expect("fallback")
                .id,
            EVERYWHERE_CONTEXT_ID
        );
    }

    #[test]
    fn stale_target_rebinds_to_close_replacement_with_same_developer() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Coding", None, None, None, None, false)
            .expect("context");
        assign_context_target_with_metadata(
            &db,
            context.id,
            "t3-code-nightly-20260830.exe",
            Some("T3 Code (nightly) 0.0.37-nightly.20260830"),
            Some("T3 Tools"),
        )
        .expect("target");

        let apps = vec![crate::system::apps::InstalledApp {
            name: "T3 Code (nightly) 0.0.38-nightly.20260901".to_string(),
            exe: "t3-code-nightly-20260901.exe".to_string(),
            developer: Some("T3 Tools".to_string()),
        }];
        assert!(reconcile_context_targets(&db, &apps).expect("reconcile"));
        let targets = query_context_targets(&db, Some(context.id)).expect("targets");
        assert_eq!(targets[0].executable, "t3-code-nightly-20260901.exe");
        assert_eq!(targets[0].developer.as_deref(), Some("T3 Tools"));
    }

    #[test]
    fn stale_target_rejects_close_name_from_different_developer() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Coding", None, None, None, None, false)
            .expect("context");
        assign_context_target_with_metadata(
            &db,
            context.id,
            "t3-code-nightly-old.exe",
            Some("T3 Code nightly"),
            Some("T3 Tools"),
        )
        .expect("target");

        let apps = vec![crate::system::apps::InstalledApp {
            name: "T3 Code nightly".to_string(),
            exe: "t3-code-nightly-new.exe".to_string(),
            developer: Some("Unrelated Tools".to_string()),
        }];
        assert!(!reconcile_context_targets(&db, &apps).expect("reconcile"));
        let targets = query_context_targets(&db, Some(context.id)).expect("targets");
        assert_eq!(targets[0].executable, "t3-code-nightly-old.exe");
    }

    #[test]
    fn website_domain_match_takes_priority_over_executable_match() {
        let db = open(":memory:").expect("db");
        let exe_context = insert_context_returning(&db, "Browsing", None, None, None, None, false)
            .expect("exe context");
        let site_context =
            insert_context_returning(&db, "Work Email", None, None, None, None, false)
                .expect("site context");
        assign_context_target(&db, exe_context.id, "chrome.exe").expect("exe target");
        assign_context_website(&db, site_context.id, "mail.google.com").expect("website target");

        assert_eq!(
            resolve_context_for_target(&db, "chrome.exe", Some("mail.google.com"))
                .expect("resolve")
                .id,
            site_context.id
        );
        assert_eq!(
            resolve_context_for_target(&db, "chrome.exe", Some("docs.google.com"))
                .expect("fallback to exe")
                .id,
            exe_context.id
        );
        assert_eq!(
            resolve_context_for_target(&db, "chrome.exe", None)
                .expect("no domain")
                .id,
            exe_context.id
        );
    }

    #[test]
    fn sub_app_then_website_then_app_then_everywhere_selects_one_group() {
        let db = open(":memory:").expect("db");
        let app = insert_context_returning(&db, "Browser", None, None, None, None, false).unwrap();
        let site = insert_context_returning(&db, "Mail", None, None, None, None, false).unwrap();
        let sub_app_context =
            insert_context_returning(&db, "Project", None, None, None, None, false).unwrap();
        assign_context_target(&db, app.id, "chrome.exe").unwrap();
        assign_context_website(&db, site.id, "mail.example").unwrap();
        let sub_app = create_sub_app(
            &db,
            NewSubApp {
                executable: "chrome.exe",
                app_name: Some("Chrome"),
                label: "Project",
                icon: None,
                title_pattern: "#engineering",
                match_mode: TitleMatchMode::Contains,
            },
        )
        .unwrap();
        assign_sub_app(&db, sub_app.id, Some(sub_app_context.id)).unwrap();

        assert_eq!(
            resolve_context_with_sub_app(
                &db,
                "chrome.exe",
                Some("mail.example"),
                Some("#engineering | Mail")
            )
            .unwrap()
            .0
            .id,
            sub_app_context.id
        );
        assert_eq!(
            resolve_context_with_sub_app(
                &db,
                "chrome.exe",
                Some("mail.example"),
                Some("#general | Mail")
            )
            .unwrap()
            .0
            .id,
            site.id
        );
        assert_eq!(
            resolve_context_with_sub_app(&db, "chrome.exe", Some("docs.example"), None)
                .unwrap()
                .0
                .id,
            app.id
        );
        assert_eq!(
            resolve_context_with_sub_app(&db, "unknown.exe", None, None)
                .unwrap()
                .0
                .id,
            EVERYWHERE_CONTEXT_ID
        );

        assign_sub_app(&db, sub_app.id, None).unwrap();
        assert_eq!(
            resolve_context_with_sub_app(
                &db,
                "chrome.exe",
                Some("mail.example"),
                Some("#engineering | Mail")
            )
            .unwrap()
            .0
            .id,
            site.id
        );
    }

    #[test]
    fn website_domain_normalizes_pasted_urls() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Work Email", None, None, None, None, false)
            .expect("context");
        let target =
            assign_context_website(&db, context.id, "https://Mail.Google.com/mail/u/0?tab=rm")
                .expect("assign");
        assert_eq!(target.domain, "mail.google.com");
    }

    #[test]
    fn assigning_a_target_replaces_its_previous_context() {
        let db = open(":memory:").expect("db");
        let first =
            insert_context_returning(&db, "First", None, None, None, None, false).expect("first");
        let second =
            insert_context_returning(&db, "Second", None, None, None, None, false).expect("second");
        assign_context_target(&db, first.id, "editor.exe").expect("first target");
        assign_context_target(&db, second.id, "EDITOR.EXE").expect("replacement target");

        let targets = query_context_targets(&db, None).expect("targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].context_id, second.id);
    }

    #[test]
    fn query_context_targets_hides_unresolved_sync_rows() {
        let db = open(":memory:").expect("db");
        let context =
            insert_context_returning(&db, "Cross Platform", None, None, None, None, false)
                .expect("context");
        assign_context_target(&db, context.id, "editor.exe").expect("resolved target");
        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO context_targets (context_id, executable, platform) VALUES (?1, ?2, NULL)",
                params![context.id, "?::some mac only app.app"],
            )
            .expect("insert unresolved row");
        }

        let targets = query_context_targets(&db, Some(context.id)).expect("targets");
        assert_eq!(
            targets.len(),
            1,
            "only the resolved target should be visible"
        );
        assert_eq!(targets[0].executable, "editor.exe");
    }

    #[test]
    fn deleting_a_context_returns_items_to_everywhere() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Temporary", None, None, None, None, false)
            .expect("context");
        insert_dictionary_entry(&db, "Tauri", Some("Tari")).expect("dictionary");
        insert_snippet(&db, "sig", "signature", "").expect("snippet");
        let dictionary_id = query_dictionary(&db).unwrap()[0].id;
        let snippet_id = query_snippets(&db).unwrap()[0].id;

        set_dictionary_context_assignment(&db, context.id, dictionary_id, true).unwrap();
        set_snippet_context_assignment(&db, context.id, snippet_id, true).unwrap();
        assign_context_website(&db, context.id, "mail.google.com").unwrap();
        set_dictionary_context_assignment(&db, EVERYWHERE_CONTEXT_ID, dictionary_id, false)
            .unwrap();
        set_snippet_context_assignment(&db, EVERYWHERE_CONTEXT_ID, snippet_id, false).unwrap();

        delete_context(&db, context.id).expect("delete context");

        assert!(query_context_website_targets(&db, None).unwrap().is_empty());

        assert_eq!(
            query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            query_snippets_for_context(&db, EVERYWHERE_CONTEXT_ID)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn deleting_context_absorbs_pruned_daily_analytics() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Temporary", None, None, None, None, false)
            .expect("context");
        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "INSERT INTO transcriptions
                   (uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id, created_at)
                 VALUES ('old-context-row', 'old', 'old', 8, 8, 1000, 0, 'test', ?1, datetime('now', '-40 days'))",
                params![context.id],
            )
            .expect("old transcription");
        }
        prune_transcriptions_older_than(&db, 7).expect("prune");
        let deleted_uuid: String = {
            let conn = lock_conn(&db).expect("lock");
            conn.query_row(
                "SELECT uuid FROM contexts WHERE id = ?1",
                params![context.id],
                |r| r.get(0),
            )
            .expect("context uuid")
        };

        delete_context(&db, context.id).expect("delete context");

        let conn = lock_conn(&db).expect("lock");
        let everywhere_words: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(total_words), 0)
                   FROM transcription_context_daily_stats
                  WHERE context_uuid = (SELECT uuid FROM contexts WHERE id = ?1)",
                params![EVERYWHERE_CONTEXT_ID],
                |r| r.get(0),
            )
            .expect("Everywhere daily analytics");
        assert_eq!(everywhere_words, 8);
        let stranded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_context_daily_stats WHERE context_uuid = ?1",
                params![deleted_uuid],
                |r| r.get(0),
            )
            .expect("stranded analytics count");
        assert_eq!(stranded, 0);
    }

    #[test]
    fn deleting_last_context_transcription_removes_zero_daily_bucket() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Temporary", None, None, None, None, false)
            .expect("context");
        let transcription = insert_transcription_returning(
            &db,
            "temporary",
            "temporary",
            1,
            1_000,
            "test",
            None,
            Some(context.id),
        )
        .expect("transcription");
        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "DELETE FROM transcriptions WHERE id = ?1",
                params![transcription.id],
            )
            .expect("delete transcription");
            let buckets: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transcription_context_daily_stats
                      WHERE context_uuid = (SELECT uuid FROM contexts WHERE id = ?1)
                        AND total_transcriptions <= 0",
                    params![context.id],
                    |r| r.get(0),
                )
                .expect("context daily buckets");
            assert_eq!(buckets, 0);
        }
    }

    #[test]
    fn deleting_a_context_moves_its_correction_mapping_to_everywhere() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Temporary", None, None, None, None, false)
            .expect("context");
        let dictionary = insert_dictionary_entry_returning(
            &db,
            "Kubernetes",
            Some("kubernetez"),
            Some(context.id),
        )
        .expect("dictionary");

        let before = query_dictionary_for_context(&db, context.id)
            .expect("context dictionary")
            .into_iter()
            .find(|entry| entry.id == dictionary.id)
            .expect("context entry");
        assert_eq!(before.corrections.len(), 1);
        assert_eq!(before.corrections[0].context_id, context.id);

        delete_context(&db, context.id).expect("delete context");

        let after = query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID)
            .expect("Everywhere dictionary")
            .into_iter()
            .find(|entry| entry.id == dictionary.id)
            .expect("moved entry");
        assert_eq!(after.mistake.as_deref(), Some("kubernetez"));
        assert_eq!(after.corrections.len(), 1);
        assert_eq!(after.corrections[0].context_id, EVERYWHERE_CONTEXT_ID);
        assert_eq!(after.corrections[0].mistake, "kubernetez");
    }

    #[test]
    fn conflicting_content_reports_and_moves_from_everywhere() {
        let db = open(":memory:").expect("db");
        let context =
            insert_context_returning(&db, "Work", None, None, None, None, false).expect("context");
        let dictionary = insert_dictionary_entry_returning(&db, "Grok", Some("Glock"), None)
            .expect("dictionary");
        let snippet =
            insert_snippet_returning(&db, "sig", "Best regards", "", None).expect("snippet");

        let dictionary_locations = query_dictionary_entry_contexts(&db, "Grok").expect("locations");
        assert_eq!(dictionary_locations.len(), 1);
        assert_eq!(dictionary_locations[0].id, EVERYWHERE_CONTEXT_ID);
        assert!(dictionary_locations[0].is_everywhere);

        let snippet_locations = query_snippet_entry_contexts(&db, "sig").expect("locations");
        assert_eq!(snippet_locations.len(), 1);
        assert_eq!(snippet_locations[0].id, EVERYWHERE_CONTEXT_ID);
        assert!(snippet_locations[0].is_everywhere);

        let moved_dictionary = move_dictionary_entry_by_term_to_context(&db, "Grok", context.id)
            .expect("move dictionary");
        assert_eq!(moved_dictionary.id, dictionary.id);
        assert_eq!(moved_dictionary.mistake.as_deref(), Some("Glock"));
        assert!(query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID)
            .unwrap()
            .iter()
            .all(|entry| entry.id != dictionary.id));
        assert!(query_dictionary_for_context(&db, context.id)
            .unwrap()
            .iter()
            .any(|entry| entry.id == dictionary.id));

        let moved_snippet =
            move_snippet_entry_to_context(&db, "sig", context.id).expect("move snippet");
        assert_eq!(moved_snippet.id, snippet.id);
        assert_eq!(moved_snippet.expansion, "Best regards");
        assert!(query_snippets_for_context(&db, EVERYWHERE_CONTEXT_ID)
            .unwrap()
            .iter()
            .all(|entry| entry.id != snippet.id));
        assert!(query_snippets_for_context(&db, context.id)
            .unwrap()
            .iter()
            .any(|entry| entry.id == snippet.id));
    }

    #[test]
    fn duplicating_context_copies_settings_and_content_without_targets() {
        let db = open(":memory:").expect("db");
        let source = insert_context_returning(
            &db,
            "Development",
            Some("code"),
            Some("casual"),
            Some("high"),
            Some("Use concise technical language."),
            true,
        )
        .expect("source context");
        update_context_color(&db, source.id, Some("oklch(0.65 0.09 250)")).expect("source color");
        let dictionary = insert_dictionary_entry_returning(&db, "Tauri", Some("Tory"), None)
            .expect("dictionary");
        let snippet =
            insert_snippet_returning(&db, "sig", "Best regards", "", None).expect("snippet");
        set_dictionary_context_assignment(&db, source.id, dictionary.id, true)
            .expect("dictionary assignment");
        set_snippet_context_assignment(&db, source.id, snippet.id, true)
            .expect("snippet assignment");
        assign_context_target(&db, source.id, "code.exe").expect("app target");
        assign_context_website(&db, source.id, "docs.example.com").expect("website target");

        let duplicate = duplicate_context(&db, source.id).expect("duplicate context");

        assert_eq!(duplicate.name, "Development copy");
        assert_eq!(duplicate.icon.as_deref(), Some("code"));
        assert_eq!(duplicate.tone.as_deref(), Some("casual"));
        assert_eq!(duplicate.cleanup_intensity.as_deref(), Some("high"));
        assert_eq!(
            duplicate.custom_instructions.as_deref(),
            Some("Use concise technical language.")
        );
        assert_eq!(duplicate.color.as_deref(), Some("oklch(0.65 0.09 250)"));
        assert!(query_dictionary_for_context(&db, duplicate.id)
            .unwrap()
            .iter()
            .any(|entry| entry.id == dictionary.id));
        assert!(query_snippets_for_context(&db, duplicate.id)
            .unwrap()
            .iter()
            .any(|entry| entry.id == snippet.id));
        assert!(query_context_targets(&db, Some(duplicate.id))
            .unwrap()
            .is_empty());
        assert!(query_context_website_targets(&db, Some(duplicate.id))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn duplicated_context_names_are_unique() {
        let db = open(":memory:").expect("db");
        let source = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("source");
        insert_context_returning(&db, "Writing copy", None, None, None, None, false).expect("copy");

        let duplicate = duplicate_context(&db, source.id).expect("numbered copy");
        assert_eq!(duplicate.name, "Writing copy 2");
    }

    #[test]
    fn everywhere_context_cannot_be_duplicated() {
        let db = open(":memory:").expect("db");
        assert!(duplicate_context(&db, EVERYWHERE_CONTEXT_ID).is_err());
    }

    #[test]
    fn adding_existing_content_to_a_context_does_not_overwrite_everywhere() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "Writing", None, None, None, None, false)
            .expect("context");
        insert_dictionary_entry_returning(&db, "Verenu", Some("Vernu"), None).expect("dictionary");
        insert_snippet_returning(&db, "sig", "signature", "", None).expect("snippet");

        insert_dictionary_entry_returning(&db, "Verenu", Some("Verano"), Some(context.id))
            .expect("assign existing canonical with a Context-specific correction");
        assert!(insert_snippet_returning(&db, "sig", "different", "", Some(context.id)).is_err());

        let dictionary = query_dictionary(&db).expect("dictionary");
        assert_eq!(dictionary[0].mistake.as_deref(), Some("Vernu, Verano"));
        let snippets = query_snippets(&db).expect("snippets");
        assert_eq!(snippets[0].expansion, "signature");
        let context_dictionary =
            query_dictionary_for_context(&db, context.id).expect("context dictionary");
        assert_eq!(context_dictionary.len(), 1);
        assert_eq!(context_dictionary[0].mistake.as_deref(), Some("Verano"));
        assert!(query_snippets_for_context(&db, context.id)
            .unwrap()
            .is_empty());

        assert!(
            insert_dictionary_entry_returning(&db, "Verenu", Some("Verano"), Some(context.id))
                .is_err(),
            "the same Context cannot add a duplicate canonical mapping"
        );
        insert_snippet_returning(&db, "sig", "signature", "", Some(context.id))
            .expect("assign existing snippet");
        assert_eq!(
            query_dictionary_for_context(&db, context.id).unwrap().len(),
            1
        );
        assert_eq!(
            query_snippets_for_context(&db, context.id).unwrap().len(),
            1
        );
    }

    #[test]
    fn legacy_insert_reports_duplicate_canonical_term() {
        let db = open(":memory:").expect("db");
        insert_dictionary_entry_returning(&db, "Verenu", None, None).expect("dictionary");

        let error = insert_dictionary_entry_returning(&db, "Verenu", None, None)
            .expect_err("duplicate legacy term should fail cleanly");

        assert_eq!(error.to_string(), "\"Verenu\" is already in the dictionary");
        assert_eq!(query_dictionary(&db).expect("dictionary").len(), 1);
    }

    #[test]
    fn context_rejects_duplicate_mistranscription_variants() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "AI tools", None, None, None, None, false)
            .expect("context");

        insert_dictionary_entry_returning(&db, "@bot", Some("grok bot"), Some(context.id))
            .expect("first dictionary entry");
        let error =
            insert_dictionary_entry_returning(&db, "Boot", Some("Grok Bot, bot"), Some(context.id))
                .expect_err("duplicate variant should be rejected");

        let message = error.to_string();
        assert!(message.contains("Often mistranscribed as"));
        assert!(message.contains("@bot"));
        assert_eq!(
            query_dictionary_for_context(&db, context.id).unwrap().len(),
            1
        );
    }

    #[test]
    fn editing_an_entry_checks_all_contexts_for_duplicate_variants() {
        let db = open(":memory:").expect("db");
        let context = insert_context_returning(&db, "AI tools", None, None, None, None, false)
            .expect("context");
        let first =
            insert_dictionary_entry_returning(&db, "@bot", Some("grok bot"), Some(context.id))
                .expect("first dictionary entry");
        let second =
            insert_dictionary_entry_returning(&db, "Boot", Some("boot bot"), Some(context.id))
                .expect("second dictionary entry");

        let error = update_dictionary_entry(&db, second.id, "Boot", Some("GROK BOT"))
            .expect_err("edit should reject duplicate variant");
        assert!(error.to_string().contains("@bot"));
        assert_eq!(
            query_dictionary(&db)
                .unwrap()
                .into_iter()
                .find(|entry| entry.id == first.id)
                .and_then(|entry| entry.mistake),
            Some("grok bot".to_string())
        );
    }

    #[test]
    fn legacy_edit_reports_missing_dictionary_entry() {
        let db = open(":memory:").expect("db");

        let error = update_dictionary_entry(&db, 999, "Missing", None)
            .expect_err("editing a missing entry should fail");

        assert_eq!(error.to_string(), "Dictionary entry 999 was not found");
    }

    #[test]
    fn assigning_an_existing_entry_shares_only_canonical_identity() {
        let db = open(":memory:").expect("db");
        let source =
            insert_context_returning(&db, "Source", None, None, None, None, false).expect("source");
        let target =
            insert_context_returning(&db, "Target", None, None, None, None, false).expect("target");
        insert_dictionary_entry_returning(&db, "@bot", Some("grok bot"), Some(target.id))
            .expect("target dictionary entry");
        let second =
            insert_dictionary_entry_returning(&db, "Boot", Some("grok bot"), Some(source.id))
                .expect("source dictionary entry");

        set_dictionary_context_assignment(&db, target.id, second.id, true)
            .expect("canonical assignment should not copy source mapping");
        assert_eq!(
            query_dictionary_for_context(&db, target.id).unwrap().len(),
            2
        );
        assert_eq!(
            query_dictionary_for_context(&db, source.id).unwrap().len(),
            1
        );
        let target_entry = query_dictionary_for_context(&db, target.id)
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == second.id)
            .expect("shared canonical entry");
        assert!(target_entry.corrections.is_empty());
        assert!(target_entry.mistake.is_none());
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ContextStats {
    pub dictations: i64,
    pub words: i64,
    pub last_used_at: Option<String>,
}

pub fn query_context_stats(db: &Db, context_id: i64) -> Result<ContextStats> {
    let conn = lock_conn(db)?;
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(words), 0), MAX(created_at)
         FROM transcriptions WHERE context_id = ?1",
        params![context_id],
        |row| {
            Ok(ContextStats {
                dictations: row.get(0)?,
                words: row.get(1)?,
                last_used_at: row.get(2)?,
            })
        },
    )
    .map_err(Into::into)
}
