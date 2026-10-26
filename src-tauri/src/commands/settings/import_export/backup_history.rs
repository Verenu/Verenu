
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::ExportContext;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupHistory {
    transcriptions: Vec<Transcription>,
    calls: Vec<Call>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Transcription {
    uuid: String,
    raw_text: String,
    clean_text: String,
    words: i64,
    spoken_words: Option<i64>,
    duration_ms: i64,
    api_used: String,
    app_name: Option<String>,
    context_uuid: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Call {
    uuid: String,
    transcription_uuid: String,
    model: String,
    provider: String,
    task: String,
    audio_ms: i64,
    input_chars: i64,
    output_chars: i64,
    created_at: String,
}

#[derive(Default)]
pub(super) struct HistoryImportStats {
    pub inserted: usize,
    pub existing: usize,
    pub conflicts: usize,
    pub context_unmatched: usize,
}

pub(super) fn export_history_conn(conn: &Connection) -> Result<BackupHistory> {
    let mut stmt = conn.prepare("SELECT t.uuid, t.raw_text, t.clean_text, t.words, t.spoken_words, t.duration_ms, t.api_used, t.app_name, c.uuid, t.created_at FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id ORDER BY t.id")?;
    let transcriptions = stmt
        .query_map([], |r| {
            Ok(Transcription {
                uuid: r.get(0)?,
                raw_text: r.get(1)?,
                clean_text: r.get(2)?,
                words: r.get(3)?,
                spoken_words: r.get(4)?,
                duration_ms: r.get(5)?,
                api_used: r.get(6)?,
                app_name: r.get(7)?,
                context_uuid: r.get(8)?,
                created_at: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT a.uuid, t.uuid, a.model, a.provider, a.task, a.audio_ms, a.input_chars, a.output_chars, a.created_at FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id ORDER BY a.id")?;
    let calls = stmt
        .query_map([], |r| {
            Ok(Call {
                uuid: r.get(0)?,
                transcription_uuid: r.get(1)?,
                model: r.get(2)?,
                provider: r.get(3)?,
                task: r.get(4)?,
                audio_ms: r.get(5)?,
                input_chars: r.get(6)?,
                output_chars: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(BackupHistory {
        transcriptions,
        calls,
    })
}

pub(super) fn context_id(
    conn: &Connection,
    uuid: &str,
    contexts: &[ExportContext],
) -> Result<Option<i64>> {
    let Some(source) = contexts
        .iter()
        .find(|context| context.uuid.as_deref() == Some(uuid))
    else {
        return Ok(None);
    };
    if source.is_everywhere {
        Ok(conn
            .query_row("SELECT id FROM contexts WHERE is_everywhere = 1", [], |r| {
                r.get(0)
            })
            .optional()?)
    } else {
        Ok(conn.query_row("SELECT id FROM contexts WHERE uuid = ?1 OR name = ?2 COLLATE NOCASE ORDER BY (uuid = ?1) DESC LIMIT 1", params![uuid, source.name.trim()], |r| r.get(0)).optional()?)
    }
}

pub(super) fn import_history(
    conn: &Connection,
    history: &BackupHistory,
    contexts: &[ExportContext],
) -> Result<HistoryImportStats> {
    let mut stats = HistoryImportStats::default();
    let mut conflicts = HashSet::new();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS backup_imported_history (uuid TEXT PRIMARY KEY)",
    )?;
    let pruned_days: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT s.day FROM transcription_daily_stats s LEFT JOIN (SELECT date(created_at, 'localtime') AS day, COUNT(*) AS retained FROM transcriptions GROUP BY 1) t ON t.day = s.day WHERE s.total_transcriptions > COALESCE(t.retained, 0)")?;
        let days = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        days
    };
    let mut ids = HashSet::new();
    for row in &history.transcriptions {
        ensure!(
            uuid::Uuid::parse_str(&row.uuid).is_ok()
                && ids.insert(&row.uuid)
                && row.words >= 0
                && row.duration_ms >= 0
                && row.spoken_words.is_none_or(|v| v >= 0),
            "Invalid transcription in backup"
        );
        let valid_time: bool =
            conn.query_row("SELECT datetime(?1) IS NOT NULL", [&row.created_at], |r| {
                r.get(0)
            })?;
        ensure!(valid_time, "Invalid transcription time in backup");
    }
    let mut call_ids = HashSet::new();
    for call in &history.calls {
        ensure!(
            uuid::Uuid::parse_str(&call.uuid).is_ok()
                && call_ids.insert(&call.uuid)
                && ids.contains(&call.transcription_uuid)
                && call.audio_ms >= 0
                && call.input_chars >= 0
                && call.output_chars >= 0,
            "Invalid provider call in backup"
        );
        let valid_time: bool =
            conn.query_row("SELECT datetime(?1) IS NOT NULL", [&call.created_at], |r| {
                r.get(0)
            })?;
        ensure!(valid_time, "Invalid provider call time in backup");
    }
    for row in &history.transcriptions {
        let existing: Option<(String, String, i64, i64, String)> = conn.query_row("SELECT raw_text, clean_text, words, duration_ms, created_at FROM transcriptions WHERE uuid = ?1", [&row.uuid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
        if let Some(existing) = existing {
            if existing
                != (
                    row.raw_text.clone(),
                    row.clean_text.clone(),
                    row.words,
                    row.duration_ms,
                    row.created_at.clone(),
                )
            {
                stats.conflicts += 1;
                conflicts.insert(row.uuid.as_str());
            } else {
                stats.existing += 1;
                conn.execute(
                    "INSERT OR IGNORE INTO backup_imported_history(uuid) VALUES (?1)",
                    [&row.uuid],
                )?;
            }
            continue;
        }
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM backup_imported_history WHERE uuid = ?1)",
            [&row.uuid],
            |r| r.get(0),
        )?;
        let day: String = conn.query_row("SELECT date(?1, 'localtime')", [&row.created_at], |r| r.get(0))?;
        let pruned_overlap = pruned_days.contains(&day);
        if seen || pruned_overlap {
            stats.conflicts += 1;
            conflicts.insert(row.uuid.as_str());
            continue;
        }
        let context = match row.context_uuid.as_deref() {
            Some(uuid) => {
                let id = context_id(conn, uuid, contexts)?;
                if id.is_none() { stats.context_unmatched += 1; }
                id
            },
            None => None,
        };
        conn.execute("INSERT INTO transcriptions (uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)", params![row.uuid, row.raw_text, row.clean_text, row.words, row.spoken_words, row.duration_ms, row.api_used, row.app_name, context, row.created_at])?;
        conn.execute("INSERT INTO lifetime_stats (id, total_words) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET total_words = total_words + excluded.total_words", [row.words])?;
        conn.execute(
            "INSERT INTO backup_imported_history(uuid) VALUES (?1)",
            [&row.uuid],
        )?;
        stats.inserted += 1;
    }
    for call in &history.calls {
        if conflicts.contains(call.transcription_uuid.as_str()) {
            continue;
        }
        let existing: Option<bool> = conn.query_row("SELECT t.uuid = ?2 AND a.model = ?3 AND a.provider = ?4 AND a.task = ?5 AND a.audio_ms = ?6 AND a.input_chars = ?7 AND a.output_chars = ?8 AND a.created_at = ?9 FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id WHERE a.uuid = ?1", params![call.uuid, call.transcription_uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.created_at], |r| r.get(0)).optional()?;
        if let Some(matches) = existing {
            if !matches { stats.conflicts += 1; }
            continue;
        }
        conn.execute("INSERT OR IGNORE INTO api_calls (uuid, transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at) SELECT ?1, id, ?2, ?3, ?4, ?5, ?6, ?7, ?8 FROM transcriptions WHERE uuid = ?9", params![call.uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.created_at, call.transcription_uuid])?;
    }
    Ok(stats)
}
