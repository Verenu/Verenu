
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
pub(super) struct Transcription {
    pub(super) uuid: String,
    raw_text: String,
    clean_text: String,
    pub(super) words: i64,
    pub(super) spoken_words: Option<i64>,
    pub(super) duration_ms: i64,
    api_used: String,
    app_name: Option<String>,
    pub(super) context_uuid: Option<String>,
    pub(super) created_at: String,
    #[serde(default)]
    pub(super) activity_day: Option<String>,
    #[serde(default)]
    pub(super) activity_hour: Option<i64>,
    #[serde(default)]
    pub(super) held: bool,
    #[serde(default)]
    pub(super) hourly_held: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Call {
    pub(super) uuid: String,
    pub(super) transcription_uuid: String,
    pub(super) model: String,
    pub(super) provider: String,
    pub(super) task: String,
    pub(super) audio_ms: i64,
    pub(super) input_chars: i64,
    pub(super) output_chars: i64,
    pub(super) created_at: String,
    #[serde(default)]
    pub(super) activity_day: Option<String>,
    #[serde(default)]
    pub(super) held: bool,
}

#[derive(Default)]
pub(super) struct HistoryImportStats {
    pub inserted: usize,
    pub existing: usize,
    pub conflicts: usize,
    pub context_unmatched: usize,
    pub expired: usize,
}

pub(super) fn export_history_conn(conn: &Connection) -> Result<BackupHistory> {
    let mut stmt = conn.prepare("SELECT t.uuid, t.raw_text, t.clean_text, t.words, t.spoken_words, t.duration_ms, t.api_used, t.app_name, COALESCE(c.uuid, p.context_uuid), t.created_at, t.activity_day, t.activity_hour, EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid = t.uuid) OR EXISTS(SELECT 1 FROM backup_preupgrade_history p WHERE p.uuid = t.uuid), EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=t.uuid) FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id LEFT JOIN pending_transcription_contexts p ON p.transcription_uuid=t.uuid ORDER BY t.id")?;
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
                activity_day: r.get(10)?,
                activity_hour: r.get(11)?,
                held: r.get(12)?,
                hourly_held: r.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT a.uuid, t.uuid, a.model, a.provider, a.task, a.audio_ms, a.input_chars, a.output_chars, a.created_at, a.activity_day, EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid=a.uuid) OR EXISTS(SELECT 1 FROM backup_preupgrade_history p WHERE p.uuid=t.uuid) FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id ORDER BY a.id")?;
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
                activity_day: r.get(9)?,
                held: r.get(10)?,
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
    if let Some(id) = conn.query_row("SELECT id FROM contexts WHERE uuid=?1", [uuid], |r| r.get(0)).optional()? {
        return Ok(Some(id));
    }
    if let Some(id) = conn.query_row("SELECT c.id FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?1", [uuid], |r| r.get(0)).optional()? {
        return Ok(Some(id));
    }
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

pub(super) fn record_mapped_event(conn: &Connection, event: &str, source: Option<&str>, local: Option<&str>) -> Result<()> {
    if let (Some(source), Some(local)) = (source, local) {
        conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) SELECT ?1,source_uuid,local_uuid,source_name FROM backup_context_aliases WHERE source_uuid=?2 AND local_uuid=?3", params![event,source,local])?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn import_history(
    conn: &Connection,
    history: &BackupHistory,
    contexts: &[ExportContext],
) -> Result<HistoryImportStats> {
    import_history_with_retention(conn, history, contexts, None)
}

pub(super) fn import_history_with_retention(
    conn: &Connection,
    history: &BackupHistory,
    contexts: &[ExportContext],
    cutoff: Option<&str>,
) -> Result<HistoryImportStats> {
    let mut stats = HistoryImportStats::default();
    let mut conflicts = HashSet::new();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS backup_imported_history (uuid TEXT PRIMARY KEY)",
    )?;
    let pruned_days: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT day FROM legacy_pruned_days")?;
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
                && row.spoken_words.is_none_or(|v| v >= 0)
                && row.activity_day.as_ref().is_none_or(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok())
                && row.activity_hour.is_none_or(|hour| (0..24).contains(&hour)),
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
        ensure!(call.activity_day.as_ref().is_none_or(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok()), "Invalid provider call day in backup");
    }
    for row in &history.transcriptions {
        let pending: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [&row.uuid], |r| r.get(0)).optional()?;
        if pending.as_deref().is_some_and(|uuid| row.context_uuid.as_deref() != Some(uuid)) {
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
        let local_uuid: Option<String> = context.map(|id| conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))).transpose()?;
        let expected = pending.as_deref().or(local_uuid.as_deref()).or(row.context_uuid.as_deref());
        if crate::data::db::has_conflicting_call_context(conn, &row.uuid, expected)? {
            stats.conflicts += 1;
            conflicts.insert(row.uuid.as_str());
            continue;
        }
        if let Some(id) = context {
            let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p JOIN transcriptions t ON t.uuid=p.transcription_uuid WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND t.raw_text=?3 AND t.clean_text=?4 AND t.words=?5 AND t.spoken_words IS ?6 AND t.duration_ms=?7 AND t.api_used=?8 AND t.app_name IS ?9 AND t.created_at=?10 AND (?11 IS NULL OR t.activity_day=?11) AND (?12 IS NULL OR t.activity_hour=?12))", params![row.uuid,row.context_uuid,row.raw_text,row.clean_text,row.words,row.spoken_words,row.duration_ms,row.api_used,row.app_name,row.created_at,row.activity_day,row.activity_hour], |r| r.get(0))?;
            let pending_receipt: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p JOIN transcription_receipts r ON r.uuid=p.transcription_uuid WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND r.words=?3 AND r.spoken_words IS ?4 AND r.duration_ms=?5 AND (?6 IS NULL OR r.day=?6) AND (?7 IS NULL OR r.hour=?7))", params![row.uuid,row.context_uuid,row.words,row.spoken_words,row.duration_ms,row.activity_day,row.activity_hour], |r| r.get(0))?;
            let call_only: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND NOT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1) AND NOT EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1))", params![row.uuid,row.context_uuid], |r| r.get(0))?;
            if pending || pending_receipt || call_only {
                let local_uuid: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))?;
                crate::data::db::resolve_pending_context(conn, &row.uuid, id, &local_uuid)?;
            }
        }
        let existing: Option<bool> = conn.query_row("SELECT raw_text = ?2 AND clean_text = ?3 AND words = ?4 AND spoken_words IS ?5 AND duration_ms = ?6 AND api_used = ?7 AND app_name IS ?8 AND context_id IS ?9 AND created_at = ?10 AND (?11 IS NULL OR activity_day = ?11) AND (?12 IS NULL OR activity_hour = ?12) FROM transcriptions WHERE uuid = ?1", params![row.uuid, row.raw_text, row.clean_text, row.words, row.spoken_words, row.duration_ms, row.api_used, row.app_name, context, row.created_at, row.activity_day, row.activity_hour], |r| r.get(0)).optional()?;
        if let Some(matches) = existing {
            let pending_matches: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1 AND context_uuid IS ?2)", params![row.uuid,row.context_uuid], |r| r.get(0))?;
            if !matches || (row.context_uuid.is_some() && context.is_none() && !pending_matches) {
                stats.conflicts += 1;
                conflicts.insert(row.uuid.as_str());
            } else {
                stats.existing += 1;
                record_mapped_event(conn, &row.uuid, row.context_uuid.as_deref(), local_uuid.as_deref())?;
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
        let receipt: Option<bool> = conn.query_row("SELECT words = ?2 AND spoken_words IS ?3 AND duration_ms = ?4 AND context_uuid IS COALESCE((SELECT uuid FROM contexts WHERE id = ?5), ?8) AND (?6 IS NULL OR day = ?6) AND (?7 IS NULL OR hour = ?7) FROM transcription_receipts WHERE uuid = ?1", params![row.uuid, row.words, row.spoken_words, row.duration_ms, context, row.activity_day, row.activity_hour, row.context_uuid], |r| r.get(0)).optional()?;
        if let Some(matches) = receipt {
            if matches { stats.existing += 1; record_mapped_event(conn, &row.uuid, row.context_uuid.as_deref(), local_uuid.as_deref())?; } else { stats.conflicts += 1; conflicts.insert(row.uuid.as_str()); }
            continue;
        }
        let expired: bool = match cutoff {
            Some(cutoff) => conn.query_row("SELECT datetime(?1) < datetime(?2)", params![row.created_at, cutoff], |r| r.get(0))?,
            None => false,
        };
        if expired {
            stats.conflicts += super::backup_stats::import_expired_transcription(conn, row, contexts)?;
            stats.expired += 1;
            continue;
        }
        let day: String = conn.query_row("SELECT COALESCE(?1, date(?2, 'localtime'))", params![row.activity_day, row.created_at], |r| r.get(0))?;
        let pruned_overlap = pruned_days.contains(&day);
        if seen {
            stats.conflicts += 1;
            conflicts.insert(row.uuid.as_str());
            continue;
        }
        if pruned_overlap || row.held {
            // The text is still useful, but the old summary may already count it.
            conn.execute("INSERT OR IGNORE INTO backup_held_history(uuid) VALUES (?1)", [&row.uuid])?;
            stats.conflicts += usize::from(pruned_overlap);
        }
        conn.execute("INSERT INTO transcriptions (uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id, created_at, activity_day, activity_hour) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)", params![row.uuid, row.raw_text, row.clean_text, row.words, row.spoken_words, row.duration_ms, row.api_used, row.app_name, context, row.created_at, row.activity_day, row.activity_hour])?;
        record_mapped_event(conn, &row.uuid, row.context_uuid.as_deref(), local_uuid.as_deref())?;
        if context.is_none() && row.context_uuid.is_some() {
            conn.execute("INSERT OR IGNORE INTO pending_transcription_contexts VALUES (?1,?2)", params![row.uuid,row.context_uuid])?;
            if row.hourly_held { conn.execute("INSERT OR IGNORE INTO backup_held_hourly_receipts VALUES (?1)", [&row.uuid])?; }
        } else if !pruned_overlap && !row.held { stats.conflicts += usize::from(crate::data::db::hold_ambiguous_retained_hour(conn, &row.uuid, row.hourly_held)?); }
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
        let existing: Option<bool> = conn.query_row("SELECT t.uuid = ?2 AND a.model = ?3 AND a.provider = ?4 AND a.task = ?5 AND a.audio_ms = ?6 AND a.input_chars = ?7 AND a.output_chars = ?8 AND a.created_at = ?9 AND (?10 IS NULL OR a.activity_day = ?10) FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id WHERE a.uuid = ?1", params![call.uuid, call.transcription_uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.created_at, call.activity_day], |r| r.get(0)).optional()?;
        if let Some(matches) = existing {
            if !matches { stats.conflicts += 1; }
            continue;
        }
        let receipt: Option<bool> = conn.query_row("SELECT transcription_uuid IS ?2 AND model = ?3 AND provider = ?4 AND task = ?5 AND audio_ms = ?6 AND input_chars = ?7 AND output_chars = ?8 AND (?9 IS NULL OR day = ?9) FROM call_receipts WHERE uuid = ?1", params![call.uuid, call.transcription_uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.activity_day], |r| r.get(0)).optional()?;
        if let Some(matches) = receipt {
            if !matches { stats.conflicts += 1; }
            continue;
        }
        let inserted = conn.execute("INSERT OR IGNORE INTO api_calls (uuid, transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at, activity_day) SELECT ?1, id, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?10 FROM transcriptions WHERE uuid = ?9", params![call.uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.created_at, call.transcription_uuid, call.activity_day])?;
        if inserted == 0 {
            stats.conflicts += super::backup_stats::import_call_with_pruned_parent(conn, call, contexts)?;
        } else {
            let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1)", [&call.transcription_uuid], |r| r.get(0))?;
            if pending {
                if call.held { conn.execute("INSERT OR IGNORE INTO backup_held_calls VALUES (?1)", [&call.uuid])?; }
            } else {
                stats.conflicts += usize::from(crate::data::db::hold_ambiguous_retained_call(conn, &call.uuid, call.held)?);
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db;

    #[test]
    fn retained_parent_rejects_orphan_call_context_conflict() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Second')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at) SELECT 'dddddddd-dddd-4ddd-8ddd-dddddddddddd','a','a',5,1000,'test',id,'2020-01-01 10:00:00' FROM contexts WHERE name='Second'", []).unwrap();
        }
        let mut history = export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','First'),('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Second')", []).unwrap();
        let call = serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        super::super::backup_stats::import_sync_receipt(&conn, "call_receipts", call).unwrap();
        assert!(import_history(&conn, &history, &[]).unwrap().conflicts > 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        history.transcriptions[0].context_uuid = Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into());
        assert_eq!(import_history(&conn, &history, &[]).unwrap().inserted, 1);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn retained_pending_context_survives_backup_and_resolves_on_reimport() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00')", []).unwrap();
            conn.execute("INSERT INTO pending_transcription_contexts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 10:00:00' FROM transcriptions", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        import_history(&conn, &history, &[]).unwrap();
        assert_eq!(export_history_conn(&conn).unwrap().transcriptions[0].context_uuid.as_deref(), Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"));
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Recovered')", []).unwrap();
        import_history(&conn, &history, &[]).unwrap();
        assert_eq!(conn.query_row("SELECT context_id FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), conn.query_row("SELECT id FROM contexts WHERE name='Recovered'", [], |r| r.get::<_, i64>(0)).unwrap());
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn importing_history_does_not_resurrect_a_compacted_call() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'one', 'one', 1, 1000, 'test')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid, transcription_id, model, provider, task, audio_ms, created_at) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', id, 'synthetic', 'test', 'stt', 1000, '2020-01-01 12:00:00' FROM transcriptions", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        import_history(&dest.lock().unwrap(), &history, &[]).unwrap();
        db::compact_api_calls_older_than(&dest, 90).unwrap();
        let conn = dest.lock().unwrap();
        assert_eq!(import_history(&conn, &history, &[]).unwrap().conflicts, 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn history_and_receipt_calendar_disagreement_is_reported() {
        let source = db::open(":memory:").unwrap();
        source.lock().unwrap().execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at, activity_day, activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'a', 'a', 1, 1000, 'test', '2020-01-01 01:00:00', '2019-12-31', 17)", []).unwrap();
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        conn.execute("INSERT INTO transcription_receipts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', '2020-01-01', 1, NULL, 1, NULL, 1000, 1)", []).unwrap();
        assert_eq!(import_history(&conn, &history, &[]).unwrap().conflicts, 1);
    }

    #[test]
    fn retained_call_overlapping_legacy_provider_usage_is_held() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',1,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01',0,'model','provider','stt',1,0,0,0)", []).unwrap();
        assert_eq!(import_history(&conn, &history, &[]).unwrap().conflicts, 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls a WHERE NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid=a.uuid)", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn retained_history_overlapping_legacy_hour_keeps_daily_total_once() {
        let source = db::open(":memory:").unwrap();
        source.lock().unwrap().execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01',10,0,5)", []).unwrap();
            assert_eq!(import_history(&conn, &history, &[]).unwrap().conflicts, 1);
            assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_hourly_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        assert_eq!(dest.lock().unwrap().query_row("SELECT total_words FROM transcription_hourly_stats WHERE day='2020-01-01' AND hour=10", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn expired_held_history_keeps_provider_usage_uncounted() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO backup_held_history VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',1,1000,'test','2020-01-01 10:00:00')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'model','provider','stt','2020-01-01 10:00:00' FROM transcriptions", []).unwrap();
            conn.execute("INSERT INTO backup_held_calls VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        import_history_with_retention(&conn, &history, &[], Some("2021-01-01 00:00:00")).unwrap();
        assert_eq!(conn.query_row("SELECT counted FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn held_dictation_does_not_suppress_independent_provider_usage() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO backup_held_history VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',1,1000,'test','2020-01-01 10:00:00')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'model','provider','stt','2020-01-01 10:00:00' FROM transcriptions", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        import_history_with_retention(&conn, &history, &[], Some("2021-01-01 00:00:00")).unwrap();
        assert_eq!(conn.query_row("SELECT counted FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn matching_uuid_with_changed_metadata_is_a_conflict() {
        let source = db::open(":memory:").unwrap();
        db::insert_transcription_returning(&source, "one", "one", 1, 1000, "test", None, None).unwrap();
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        assert_eq!(import_history(&conn, &history, &[]).unwrap().inserted, 1);
        conn.execute("UPDATE transcriptions SET api_used = 'different', spoken_words = 99, app_name = 'other'", []).unwrap();
        let result = import_history(&conn, &history, &[]).unwrap();
        assert_eq!(result.conflicts, 1);
        assert_eq!(result.existing, 0);
    }

    #[test]
    fn distinct_history_survives_existing_pruned_activity_on_same_day() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'source', 'source', 5, 1000, 'test', '2020-01-01 12:00:00')", []).unwrap();
        }
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'destination', 'destination', 7, 1000, 'test', '2020-01-01 12:00:00')", []).unwrap();
        }
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        let result = import_history(&conn, &history, &[]).unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
    }

    #[test]
    fn unknown_legacy_overlap_keeps_history_without_inflating_statistics() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at, activity_day, activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'source', 'source', 5, 1000, 'test', '2020-01-01 12:00:00', '2020-01-01', 12)", []).unwrap();
        }
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions) VALUES ('2020-01-01', 7, 1)", []).unwrap();
            conn.execute("INSERT INTO legacy_pruned_days(day) VALUES ('2020-01-01')", []).unwrap();
        }
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        let result = import_history(&conn, &history, &[]).unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.conflicts, 1);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn new_provider_call_survives_when_its_parent_was_pruned() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let entry = db::insert_transcription_returning(&source, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
        source.lock().unwrap().execute("UPDATE transcriptions SET created_at = '2020-01-01 12:00:00' WHERE id = ?1", [entry.id]).unwrap();
        let history = export_history_conn(&source.lock().unwrap()).unwrap();
        import_history(&dest.lock().unwrap(), &history, &[]).unwrap();
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        db::insert_api_calls(&source, &[db::ApiCall { transcription_id: entry.id, model: "synthetic".into(), provider: "test".into(), task: "stt".into(), audio_ms: 1000, input_chars: 0, output_chars: 1, created_at: "2020-01-01 12:00:00".into() }]).unwrap();
        let updated = export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        let result = import_history(&conn, &updated, &[]).unwrap();
        assert_eq!(result.conflicts, 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
}
