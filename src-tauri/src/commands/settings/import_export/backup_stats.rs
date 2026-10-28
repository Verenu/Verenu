
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[cfg(test)]
use crate::data::db;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupActivity {
    lifetime: Lifetime,
    daily: Vec<Daily>,
    #[serde(default)]
    contexts: Vec<ContextDay>,
    #[serde(default)]
    hourly: Vec<Hourly>,
    #[serde(default)]
    providers: Vec<Provider>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    remote_counters: Vec<RemoteCounter>,
    #[serde(default)]
    transcription_receipts: Vec<TranscriptionReceipt>,
    #[serde(default)]
    call_receipts: Vec<CallReceipt>,
    #[serde(default)]
    legacy_wpm: Vec<LegacyWpm>,
    #[serde(default)]
    context_provenance: Vec<ContextProvenance>,
    #[serde(default)]
    context_aliases: Option<Vec<ContextAlias>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ContextAlias { source_uuid: String, context_uuid: String, source_name: Option<String> }
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ContextProvenance { source_uuid: String, context_uuid: String, transcription_uuid: String, #[serde(default)] source_name: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RemoteCounter { device_id: String, total_words: i64, dictionary_fixes: i64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LegacyWpm { source_id: String, sum: f64, count: i64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TranscriptionReceipt {
    uuid: String,
    day: String,
    hour: i64,
    context_uuid: Option<String>,
    words: i64,
    spoken_words: Option<i64>,
    duration_ms: i64,
    #[serde(default = "receipt_counted")]
    counted: bool,
    #[serde(default = "receipt_counted")]
    hourly_counted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CallReceipt {
    uuid: String,
    transcription_uuid: Option<String>,
    day: String,
    context_uuid: Option<String>,
    model: String,
    provider: String,
    task: String,
    audio_ms: i64,
    input_chars: i64,
    output_chars: i64,
    #[serde(default = "receipt_counted")]
    counted: bool,
}

fn receipt_counted() -> bool { true }

pub(crate) fn import_sync_receipt(conn: &Connection, table: &str, value: serde_json::Value) -> Result<usize> {
    let mut activity = BackupActivity {
        lifetime: Lifetime::default(), daily: vec![], contexts: vec![], hourly: vec![], providers: vec![],
        source_id: None, remote_counters: vec![], transcription_receipts: vec![], call_receipts: vec![], legacy_wpm: vec![], context_provenance: vec![], context_aliases: None,
    };
    match table {
        "transcription_receipts" => {
            let row: TranscriptionReceipt = serde_json::from_value(value)?;
            ensure!(uuid::Uuid::parse_str(&row.uuid).is_ok() && chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d").is_ok() && (0..24).contains(&row.hour) && row.words >= 0 && row.duration_ms >= 0 && row.spoken_words.is_none_or(|n| n >= 0), "Invalid transcription receipt");
            activity.transcription_receipts.push(row);
        }
        "call_receipts" => {
            let row: CallReceipt = serde_json::from_value(value)?;
            ensure!(uuid::Uuid::parse_str(&row.uuid).is_ok() && row.transcription_uuid.as_ref().is_none_or(|id| uuid::Uuid::parse_str(id).is_ok()) && chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d").is_ok() && row.audio_ms >= 0 && row.input_chars >= 0 && row.output_chars >= 0, "Invalid provider receipt");
            activity.call_receipts.push(row);
        }
        _ => anyhow::bail!("Unknown receipt type"),
    }
    let conflicts = import_receipts(conn, &activity, &[])?;
    validate_totals(conn)?;
    Ok(conflicts)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Hourly {
    day: String,
    hour: i64,
    context_uuid: Option<String>,
    words: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Provider {
    day: String,
    context_uuid: Option<String>,
    model: String,
    provider: String,
    task: String,
    calls: i64,
    audio_ms: i64,
    input_chars: i64,
    output_chars: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ContextDay {
    context_uuid: String,
    #[serde(flatten)]
    daily: Daily,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct Lifetime {
    total_words: i64,
    dictionary_fixes: i64,
    wpm_sum: f64,
    wpm_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Daily {
    day: String,
    total_words: i64,
    total_transcriptions: i64,
    speaking_ms: i64,
    wpm_sum: f64,
    wpm_count: i64,
    best_wpm: f64,
}

#[cfg(test)]
pub(super) fn export_activity(db: &db::Db) -> Result<BackupActivity> {
    let conn = db
        .lock()
        .map_err(|_| anyhow::anyhow!("Database lock was poisoned"))?;
    export_activity_conn(&conn)
}

pub(super) fn export_activity_conn(conn: &Connection) -> Result<BackupActivity> {
    let lifetime = conn.query_row(
        "SELECT total_words, dictionary_fixes, wpm_sum, wpm_count FROM lifetime_stats WHERE id = 1",
        [], |r| Ok(Lifetime { total_words: r.get(0)?, dictionary_fixes: r.get(1)?, wpm_sum: r.get(2)?, wpm_count: r.get(3)? }),
    ).optional()?.unwrap_or_default();
    let mut stmt = conn.prepare("SELECT day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm FROM transcription_daily_stats ORDER BY day")?;
    let mut daily = stmt
        .query_map([], |r| {
            Ok(Daily {
                day: r.get(0)?,
                total_words: r.get(1)?,
                total_transcriptions: r.get(2)?,
                speaking_ms: r.get(3)?,
                wpm_sum: r.get(4)?,
                wpm_count: r.get(5)?,
                best_wpm: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT context_uuid, day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm FROM transcription_context_daily_stats ORDER BY context_uuid, day")?;
    let mut contexts = stmt
        .query_map([], |r| {
            Ok(ContextDay {
                context_uuid: r.get(0)?,
                daily: Daily {
                    day: r.get(1)?,
                    total_words: r.get(2)?,
                    total_transcriptions: r.get(3)?,
                    speaking_ms: r.get(4)?,
                    wpm_sum: r.get(5)?,
                    wpm_count: r.get(6)?,
                    best_wpm: r.get(7)?,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT uuid, day, hour, context_uuid, words, spoken_words, duration_ms, counted, NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=transcription_receipts.uuid) FROM transcription_receipts ORDER BY uuid")?;
    let transcription_receipts = stmt.query_map([], |r| Ok(TranscriptionReceipt {
        uuid: r.get(0)?, day: r.get(1)?, hour: r.get(2)?, context_uuid: r.get(3)?,
        words: r.get(4)?, spoken_words: r.get(5)?, duration_ms: r.get(6)?, counted: r.get(7)?, hourly_counted: r.get(8)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT uuid, transcription_uuid, day, context_uuid, model, provider, task, audio_ms, input_chars, output_chars, counted FROM call_receipts ORDER BY uuid")?;
    let call_receipts = stmt.query_map([], |r| Ok(CallReceipt {
        uuid: r.get(0)?, transcription_uuid: r.get(1)?, day: r.get(2)?, context_uuid: r.get(3)?,
        model: r.get(4)?, provider: r.get(5)?, task: r.get(6)?, audio_ms: r.get(7)?,
        input_chars: r.get(8)?, output_chars: r.get(9)?, counted: r.get(10)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let pending: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT transcription_uuid,context_uuid FROM pending_transcription_contexts")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?,r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let (mut retained_daily, mut retained_contexts) = retained_summaries(conn)?;
    for receipt in transcription_receipts.iter().filter(|receipt| receipt.counted) {
        let spoken = receipt.spoken_words.unwrap_or(receipt.words);
        let count = i64::from(receipt.duration_ms > 0 && spoken > 0);
        let summary = Daily { day: receipt.day.clone(), total_words: receipt.words, total_transcriptions: 1,
            speaking_ms: receipt.duration_ms, wpm_sum: if count > 0 { spoken as f64 * 60000.0 / receipt.duration_ms as f64 } else { 0.0 },
            wpm_count: count, best_wpm: 0.0 };
        add_summary(retained_daily.entry(receipt.day.clone()).or_insert_with(|| empty_day(&receipt.day)), &summary);
        let context = if pending.contains_key(&receipt.uuid) { None } else { receipt.context_uuid.clone() };
        let key = (receipt.day.clone(), context.unwrap_or_else(|| "unscoped".to_string()));
        add_summary(retained_contexts.entry(key).or_insert_with(|| empty_day(&receipt.day)), &summary);
    }
    for day in &mut daily {
        if let Some(retained) = retained_daily.get(&day.day) { subtract_retained(day, retained); }
    }
    for entry in &mut contexts {
        if let Some(retained) = retained_contexts.get(&(entry.daily.day.clone(), entry.context_uuid.clone())) {
            subtract_retained(&mut entry.daily, retained);
        }
    }
    let mut stmt = conn.prepare("SELECT s.day, s.hour, c.uuid, s.total_words FROM transcription_hourly_stats s LEFT JOIN contexts c ON c.id = s.context_id ORDER BY s.day, s.hour, s.context_id")?;
    let mut hourly = stmt
        .query_map([], |r| {
            Ok(Hourly {
                day: r.get(0)?,
                hour: r.get(1)?,
                context_uuid: r.get(2)?,
                words: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT s.day, c.uuid, s.model, s.provider, s.task, s.calls, s.audio_ms, s.input_chars, s.output_chars FROM provider_daily_stats s LEFT JOIN contexts c ON c.id = s.context_id ORDER BY s.day, s.context_id, s.model, s.provider, s.task")?;
    let mut providers = stmt
        .query_map([], |r| {
            Ok(Provider {
                day: r.get(0)?,
                context_uuid: r.get(1)?,
                model: r.get(2)?,
                provider: r.get(3)?,
                task: r.get(4)?,
                calls: r.get(5)?,
                audio_ms: r.get(6)?,
                input_chars: r.get(7)?,
                output_chars: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut hourly_indexes: HashMap<_, _> = hourly.iter().enumerate().map(|(i, row)| ((row.day.clone(), row.hour, row.context_uuid.clone()), i)).collect();
    let mut stmt = conn.prepare("SELECT t.activity_day, t.activity_hour, c.uuid, SUM(t.words) FROM transcriptions t LEFT JOIN contexts c ON c.id=t.context_id WHERE EXISTS(SELECT 1 FROM backup_preupgrade_history p WHERE p.uuid=t.uuid) AND NOT EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid=t.uuid) GROUP BY 1,2,3")?;
    for row in stmt.query_map([], |r| Ok(Hourly { day:r.get(0)?, hour:r.get(1)?, context_uuid:r.get(2)?, words:r.get(3)? }))? {
        let row = row?;
        let key = (row.day.clone(), row.hour, row.context_uuid.clone());
        if let Some(&i) = hourly_indexes.get(&key) { hourly[i].words += row.words; }
        else { hourly_indexes.insert(key, hourly.len()); hourly.push(row); }
    }
    for receipt in transcription_receipts.iter().filter(|receipt| receipt.counted && receipt.hourly_counted) {
        let context = if pending.contains_key(&receipt.uuid) { None } else { receipt.context_uuid.clone() };
        if let Some(&i) = hourly_indexes.get(&(receipt.day.clone(), receipt.hour, context)) {
            hourly[i].words = hourly[i].words.saturating_sub(receipt.words).max(0);
        }
    }
    hourly.retain(|row| row.words > 0);
    let mut provider_indexes: HashMap<_, _> = providers.iter().enumerate().map(|(i, row)| ((row.day.clone(), row.context_uuid.clone(), row.model.clone(), row.provider.clone(), row.task.clone()), i)).collect();
    let mut stmt = conn.prepare("SELECT a.activity_day, c.uuid, a.model, a.provider, a.task, COUNT(*), SUM(a.audio_ms), SUM(a.input_chars), SUM(a.output_chars) FROM api_calls a JOIN transcriptions t ON t.id=a.transcription_id LEFT JOIN contexts c ON c.id=t.context_id WHERE EXISTS(SELECT 1 FROM backup_preupgrade_history p WHERE p.uuid=t.uuid) AND NOT EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid=t.uuid) AND NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid=a.uuid) GROUP BY 1,2,3,4,5")?;
    for row in stmt.query_map([], |r| Ok(Provider { day:r.get(0)?, context_uuid:r.get(1)?, model:r.get(2)?, provider:r.get(3)?, task:r.get(4)?, calls:r.get(5)?, audio_ms:r.get(6)?, input_chars:r.get(7)?, output_chars:r.get(8)? }))? {
        let row = row?;
        let key = (row.day.clone(), row.context_uuid.clone(), row.model.clone(), row.provider.clone(), row.task.clone());
        if let Some(&i) = provider_indexes.get(&key) {
            providers[i].calls += row.calls; providers[i].audio_ms += row.audio_ms;
            providers[i].input_chars += row.input_chars; providers[i].output_chars += row.output_chars;
        } else { provider_indexes.insert(key, providers.len()); providers.push(row); }
    }
    for receipt in call_receipts.iter().filter(|receipt| receipt.counted) {
        let context = if receipt.transcription_uuid.as_ref().and_then(|uuid| pending.get(uuid)) == receipt.context_uuid.as_ref() && receipt.context_uuid.is_some() { None } else { receipt.context_uuid.clone() };
        if let Some(&i) = provider_indexes.get(&(receipt.day.clone(), context, receipt.model.clone(), receipt.provider.clone(), receipt.task.clone())) {
            let row = &mut providers[i];
            row.calls = row.calls.saturating_sub(1).max(0);
            row.audio_ms = row.audio_ms.saturating_sub(receipt.audio_ms).max(0);
            row.input_chars = row.input_chars.saturating_sub(receipt.input_chars).max(0);
            row.output_chars = row.output_chars.saturating_sub(receipt.output_chars).max(0);
        }
    }
    providers.retain(|row| row.calls > 0);
    let source_id: Option<String> = conn.query_row("SELECT uuid FROM sync_identity LIMIT 1", [], |r| r.get(0)).optional()?;
    let mut stmt = conn.prepare("SELECT device_id, total_words, dictionary_fixes FROM combined_device_counters ORDER BY device_id")?;
    let remote_counters = stmt.query_map([], |r| Ok(RemoteCounter { device_id: r.get(0)?, total_words: r.get(1)?, dictionary_fixes: r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let (daily_wpm, daily_count): (f64, i64) = conn.query_row("SELECT COALESCE(SUM(wpm_sum),0), COALESCE(SUM(wpm_count),0) FROM transcription_daily_stats", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut stmt = conn.prepare("SELECT source_id, wpm_sum, wpm_count FROM backup_lifetime_wpm_contributions ORDER BY source_id")?;
    let mut legacy_wpm = stmt.query_map([], |r| Ok(LegacyWpm { source_id:r.get(0)?, sum:r.get(1)?, count:r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let imported_sum: f64 = legacy_wpm.iter().map(|entry| entry.sum).sum();
    let imported_count: i64 = legacy_wpm.iter().map(|entry| entry.count).sum();
    let extra_sum = (lifetime.wpm_sum - daily_wpm - imported_sum).max(0.0);
    let extra_count = lifetime.wpm_count.saturating_sub(daily_count).saturating_sub(imported_count).max(0);
    if extra_count > 0 {
        if let Some(id) = &source_id { legacy_wpm.push(LegacyWpm { source_id:id.clone(), sum:extra_sum, count:extra_count }); }
    }
    let mut stmt = conn.prepare("SELECT e.source_uuid,e.local_uuid,e.transcription_uuid,e.source_name FROM backup_context_event_sources e JOIN contexts c ON c.uuid=e.local_uuid ORDER BY e.source_uuid,e.transcription_uuid")?;
    let context_provenance = stmt.query_map([], |r| Ok(ContextProvenance { source_uuid:r.get(0)?, context_uuid:r.get(1)?, transcription_uuid:r.get(2)?, source_name:r.get(3)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT a.source_uuid,a.local_uuid,a.source_name FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid ORDER BY a.source_uuid")?;
    let context_aliases = stmt.query_map([], |r| Ok(ContextAlias { source_uuid:r.get(0)?, context_uuid:r.get(1)?, source_name:r.get(2)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(BackupActivity {
        lifetime,
        daily,
        contexts,
        hourly,
        providers,
        source_id,
        remote_counters,
        transcription_receipts,
        call_receipts,
        legacy_wpm,
        context_provenance,
        context_aliases: Some(context_aliases),
    })
}

fn empty_day(day: &str) -> Daily {
    Daily { day: day.to_string(), total_words: 0, total_transcriptions: 0, speaking_ms: 0, wpm_sum: 0.0, wpm_count: 0, best_wpm: 0.0 }
}

fn add_summary(total: &mut Daily, contribution: &Daily) {
    total.total_words += contribution.total_words;
    total.total_transcriptions += contribution.total_transcriptions;
    total.speaking_ms += contribution.speaking_ms;
    total.wpm_sum += contribution.wpm_sum;
    total.wpm_count += contribution.wpm_count;
}

type RetainedSummaries = (HashMap<String, Daily>, HashMap<(String, String), Daily>);

fn retained_summaries(conn: &Connection) -> Result<RetainedSummaries> {
    let mut global = HashMap::<String, Daily>::new();
    let mut by_context = HashMap::new();
    let mut stmt = conn.prepare("SELECT COALESCE(t.activity_day, date(t.created_at, 'localtime')), COALESCE(c.uuid, 'unscoped'), SUM(t.words), COUNT(*), SUM(t.duration_ms), SUM(CASE WHEN t.duration_ms > 0 AND COALESCE(t.spoken_words, t.words) > 0 THEN CAST(COALESCE(t.spoken_words, t.words) AS REAL) * 60000.0 / t.duration_ms ELSE 0 END), SUM(CASE WHEN t.duration_ms > 0 AND COALESCE(t.spoken_words, t.words) > 0 THEN 1 ELSE 0 END) FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id WHERE NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = t.uuid) AND NOT EXISTS (SELECT 1 FROM backup_preupgrade_history p WHERE p.uuid = t.uuid) GROUP BY 1, 2")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, Daily { day: r.get(0)?, total_words: r.get(2)?, total_transcriptions: r.get(3)?, speaking_ms: r.get(4)?, wpm_sum: r.get(5)?, wpm_count: r.get(6)?, best_wpm: 0.0 })))?;
    for row in rows {
        let (context, retained) = row?;
        let total = global.entry(retained.day.clone()).or_insert(Daily { day: retained.day.clone(), total_words: 0, total_transcriptions: 0, speaking_ms: 0, wpm_sum: 0.0, wpm_count: 0, best_wpm: 0.0 });
        total.total_words += retained.total_words;
        total.total_transcriptions += retained.total_transcriptions;
        total.speaking_ms += retained.speaking_ms;
        total.wpm_sum += retained.wpm_sum;
        total.wpm_count += retained.wpm_count;
        by_context.insert((retained.day.clone(), context), retained);
    }
    Ok((global, by_context))
}

fn subtract_retained(total: &mut Daily, retained: &Daily) {
    total.total_words = total.total_words.saturating_sub(retained.total_words).max(0);
    total.total_transcriptions = total.total_transcriptions.saturating_sub(retained.total_transcriptions).max(0);
    total.speaking_ms = total.speaking_ms.saturating_sub(retained.speaking_ms).max(0);
    total.wpm_sum = (total.wpm_sum - retained.wpm_sum).max(0.0);
    total.wpm_count = total.wpm_count.saturating_sub(retained.wpm_count).max(0);
}

pub(super) fn existing_days(conn: &Connection) -> Result<HashSet<String>> {
    // Capture bucket membership before restoring history or receipts. Prefixes
    // keep hour and provider identities distinct from ISO calendar days.
    let mut stmt = conn.prepare("SELECT day FROM transcription_daily_stats WHERE total_transcriptions > 0 UNION SELECT day FROM transcription_hourly_stats UNION SELECT day FROM provider_daily_stats UNION SELECT COALESCE(activity_day, date(created_at, 'localtime')) FROM api_calls WHERE COALESCE(activity_day, date(created_at, 'localtime')) IS NOT NULL")?;
    let mut days = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    let mut stmt = conn.prepare("SELECT day, hour, context_id FROM transcription_hourly_stats UNION SELECT activity_day, activity_hour, COALESCE(context_id,0) FROM transcriptions WHERE activity_day IS NOT NULL AND activity_hour IS NOT NULL")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
        let (day, hour, context) = row?;
        days.insert(hour_bucket(&day, hour, context)?);
    }
    let mut stmt = conn.prepare("SELECT day, context_id, model, provider, task FROM provider_daily_stats UNION SELECT a.activity_day, COALESCE(t.context_id,0), a.model, a.provider, a.task FROM api_calls a LEFT JOIN transcriptions t ON t.id=a.transcription_id WHERE a.activity_day IS NOT NULL")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))? {
        let (day, context, model, provider, task) = row?;
        days.insert(provider_bucket(&day, context, &model, &provider, &task)?);
    }
    Ok(days)
}

fn hour_bucket(day: &str, hour: i64, context: i64) -> Result<String> {
    Ok(format!("hour:{}", serde_json::to_string(&(day, hour, context))?))
}

fn provider_bucket(day: &str, context: i64, model: &str, provider: &str, task: &str) -> Result<String> {
    Ok(format!("provider:{}", serde_json::to_string(&(day, context, model, provider, task))?))
}

fn valid_context(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok() || value == "everywhere-0000-0000-0000-000000000001"
}

pub(super) fn import_context_aliases(conn: &Connection, activity: &BackupActivity, source_contexts: &[super::ExportContext]) -> Result<usize> {
    let mut conflicts = 0;
    let mut aliases = activity.context_aliases.clone().unwrap_or_default();
    if activity.context_aliases.is_none() {
        let mut candidates: HashMap<&str, HashSet<(&str, Option<&str>)>> = HashMap::new();
        for entry in &activity.context_provenance { candidates.entry(&entry.source_uuid).or_default().insert((&entry.context_uuid, entry.source_name.as_deref())); }
        for (source, values) in candidates {
            if values.len() == 1 {
                let (context, name) = values.into_iter().next().expect("one candidate");
                aliases.push(ContextAlias { source_uuid:source.to_string(), context_uuid:context.to_string(), source_name:name.map(str::to_string) });
            } else { conflicts += 1; }
        }
    }
    let mut seen_aliases = HashSet::new();
    for entry in &aliases {
        ensure!(valid_context(&entry.source_uuid) && valid_context(&entry.context_uuid) && seen_aliases.insert(&entry.source_uuid), "Invalid Context alias");
        let Some(id) = super::backup_history::context_id(conn, &entry.context_uuid, source_contexts)? else { conflicts += 1; continue; };
        let local: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))?;
        let alias: Option<String> = conn.query_row("SELECT local_uuid FROM backup_context_aliases WHERE source_uuid=?1", [&entry.source_uuid], |r| r.get(0)).optional()?;
        if alias.as_deref().is_some_and(|old| old != local) { conflicts += 1; continue; }
        if !conn.query_row("SELECT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1)", [&entry.source_uuid], |r| r.get::<_, bool>(0))? {
            conn.execute("INSERT OR IGNORE INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES (?1,?2,?3)", params![entry.source_uuid,local,entry.source_name])?;
            crate::data::db::resolve_pending_contexts(conn, &entry.source_uuid, id, &local)?;
        }
    }
    Ok(conflicts)
}

// Identified events merge by UUID. Legacy aggregate snapshots require a
// conservative merge because their individual events cannot be identified.
pub(super) fn import_activity(
    conn: &Connection,
    activity: &BackupActivity,
    source_contexts: &[super::ExportContext],
    existing_days: &HashSet<String>,
) -> Result<usize> {
    let life = &activity.lifetime;
    ensure!(
        life.total_words >= 0
            && life.dictionary_fixes >= 0
            && life.wpm_count >= 0
            && life.wpm_sum.is_finite()
            && life.wpm_sum >= 0.0,
        "Invalid lifetime activity in backup"
    );
    let mut legacy_wpm_sources = HashSet::new();
    for entry in &activity.legacy_wpm {
        ensure!(uuid::Uuid::parse_str(&entry.source_id).is_ok() && legacy_wpm_sources.insert(&entry.source_id) && entry.count > 0 && entry.sum.is_finite() && entry.sum >= 0.0, "Invalid legacy WPM activity");
    }
    for day in activity
        .daily
        .iter()
        .chain(activity.contexts.iter().map(|context| &context.daily))
    {
        ensure!(
            chrono::NaiveDate::parse_from_str(&day.day, "%Y-%m-%d").is_ok()
                && day.total_words >= 0
                && day.total_transcriptions >= 0
                && day.speaking_ms >= 0
                && day.wpm_count >= 0
                && day.wpm_count <= day.total_transcriptions
                && day.wpm_sum.is_finite()
                && day.wpm_sum >= 0.0
                && day.best_wpm.is_finite()
                && day.best_wpm >= 0.0,
            "Invalid daily activity in backup"
        );
    }
    for row in &activity.hourly {
        ensure!(
            chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d").is_ok()
                && (0..24).contains(&row.hour)
                && row.words >= 0,
            "Invalid hourly activity in backup"
        );
    }
    for row in &activity.providers {
        ensure!(
            chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d").is_ok()
                && row.calls >= 0
                && row.audio_ms >= 0
                && row.input_chars >= 0
                && row.output_chars >= 0,
            "Invalid provider activity in backup"
        );
    }
    let mut receipt_ids = HashSet::new();
    for receipt in &activity.transcription_receipts {
        ensure!(uuid::Uuid::parse_str(&receipt.uuid).is_ok() && receipt_ids.insert(&receipt.uuid)
            && chrono::NaiveDate::parse_from_str(&receipt.day, "%Y-%m-%d").is_ok()
            && (0..24).contains(&receipt.hour) && receipt.words >= 0 && receipt.duration_ms >= 0
            && receipt.spoken_words.is_none_or(|words| words >= 0), "Invalid transcription receipt");
    }
    let mut call_ids = HashSet::new();
    for receipt in &activity.call_receipts {
        ensure!(uuid::Uuid::parse_str(&receipt.uuid).is_ok() && call_ids.insert(&receipt.uuid)
            && receipt.transcription_uuid.as_ref().is_none_or(|id| uuid::Uuid::parse_str(id).is_ok())
            && chrono::NaiveDate::parse_from_str(&receipt.day, "%Y-%m-%d").is_ok()
            && receipt.audio_ms >= 0 && receipt.input_chars >= 0 && receipt.output_chars >= 0,
            "Invalid provider receipt");
    }
    let self_id: Option<String> = conn.query_row("SELECT uuid FROM sync_identity LIMIT 1", [], |r| r.get(0)).optional()?;
    let mut conflicts = 0;
    for counter in &activity.remote_counters {
        ensure!(uuid::Uuid::parse_str(&counter.device_id).is_ok() && counter.total_words >= 0 && counter.dictionary_fixes >= 0, "Invalid device counter in backup");
        if self_id.as_deref() == Some(&counter.device_id) {
            conn.execute("UPDATE lifetime_stats SET total_words = MAX(total_words, ?1), dictionary_fixes = MAX(dictionary_fixes, ?2) WHERE id = 1", params![counter.total_words, counter.dictionary_fixes])?;
        } else {
            conn.execute("INSERT INTO backup_device_counters(device_id, total_words, dictionary_fixes) VALUES (?1, ?2, ?3) ON CONFLICT(device_id) DO UPDATE SET total_words = MAX(total_words, excluded.total_words), dictionary_fixes = MAX(dictionary_fixes, excluded.dictionary_fixes)", params![counter.device_id, counter.total_words, counter.dictionary_fixes])?;
        }
    }
    if let Some(source_id) = activity.source_id.as_deref() {
        ensure!(uuid::Uuid::parse_str(source_id).is_ok(), "Invalid source device in backup");
        if self_id.as_deref() == Some(source_id) {
            conn.execute("INSERT INTO lifetime_stats(id, total_words, dictionary_fixes) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET total_words = MAX(total_words, excluded.total_words), dictionary_fixes = MAX(dictionary_fixes, excluded.dictionary_fixes)", params![life.total_words, life.dictionary_fixes])?;
        } else {
            conn.execute("INSERT INTO backup_device_counters(device_id, total_words, dictionary_fixes) VALUES (?1, ?2, ?3) ON CONFLICT(device_id) DO UPDATE SET total_words = MAX(total_words, excluded.total_words), dictionary_fixes = MAX(dictionary_fixes, excluded.dictionary_fixes)", params![source_id, life.total_words, life.dictionary_fixes])?;
        }
    } else if life.total_words > 0 || life.dictionary_fixes > 0 {
        conflicts += 1;
    }
    let before_resolution = self::existing_days(conn)?;
    let mut overlap_days = existing_days.clone();
    conflicts += import_context_aliases(conn, activity, source_contexts)?;
    overlap_days.extend(self::existing_days(conn)?.difference(&before_resolution).cloned());
    conflicts += import_receipts(conn, activity, source_contexts)?;
    for entry in &activity.context_provenance {
        ensure!(valid_context(&entry.source_uuid) && valid_context(&entry.context_uuid) && uuid::Uuid::parse_str(&entry.transcription_uuid).is_ok(), "Invalid Context provenance");
        let Some(id) = super::backup_history::context_id(conn, &entry.context_uuid, source_contexts)? else { conflicts += 1; continue; };
        let local: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))?;
        let matching: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 AND context_id=?2 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND context_uuid=?3 UNION ALL SELECT 1 FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid=?3)", params![entry.transcription_uuid,id,local], |r| r.get(0))?;
        if !matching { conflicts += 1; continue; }
        conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) VALUES (?1,?2,?3,?4)", params![entry.transcription_uuid,entry.source_uuid,local,entry.source_name])?;
    }
    for entry in &activity.legacy_wpm {
        if self_id.as_deref() == Some(&entry.source_id) {
            let (sum, count): (f64, i64) = conn.query_row("SELECT wpm_sum, wpm_count FROM lifetime_stats WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let (daily_sum, daily_count): (f64, i64) = conn.query_row("SELECT COALESCE(SUM(wpm_sum),0), COALESCE(SUM(wpm_count),0) FROM transcription_daily_stats", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let (imported_sum, imported_count): (f64, i64) = conn.query_row("SELECT COALESCE(SUM(wpm_sum),0), COALESCE(SUM(wpm_count),0) FROM backup_lifetime_wpm_contributions", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let extra_sum = (sum - daily_sum - imported_sum).max(0.0);
            let extra_count = count.saturating_sub(daily_count).saturating_sub(imported_count).max(0);
            if entry.count > extra_count && entry.sum >= extra_sum {
                conn.execute("UPDATE lifetime_stats SET wpm_sum=wpm_sum+?1, wpm_count=wpm_count+?2 WHERE id=1", params![entry.sum - extra_sum, entry.count - extra_count])?;
            } else if entry.count > extra_count || (entry.count == extra_count && entry.sum != extra_sum) {
                conflicts += 1;
            }
            continue;
        }
        let prior: Option<(f64, i64)> = conn.query_row("SELECT wpm_sum, wpm_count FROM backup_lifetime_wpm_contributions WHERE source_id=?1", [&entry.source_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        match prior {
            Some((sum, count)) => conflicts += usize::from(sum != entry.sum || count != entry.count),
            None if overlap_days.is_empty() => {
                let prior_extra: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM backup_lifetime_wpm_contributions)", [], |r| r.get(0))?;
                if prior_extra { conflicts += 1; continue; }
                conn.execute("UPDATE lifetime_stats SET wpm_sum=wpm_sum+?1, wpm_count=wpm_count+?2 WHERE id=1", params![entry.sum, entry.count])?;
                conn.execute("INSERT INTO backup_lifetime_wpm_contributions VALUES (?1, ?2, ?3)", params![entry.source_id, entry.sum, entry.count])?;
            }
            None => conflicts += 1,
        }
    }
    let (local_words, local_fixes): (i64, i64) = conn.query_row(
        "SELECT total_words, dictionary_fixes FROM lifetime_stats WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut totals = (i128::from(local_words), i128::from(local_fixes));
    let mut counters = conn.prepare("SELECT total_words, dictionary_fixes FROM combined_device_counters")?;
    for counter in counters.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (words, fixes) = counter?;
        totals.0 += i128::from(words);
        totals.1 += i128::from(fixes);
    }
    ensure!(totals.0 <= i128::from(i64::MAX) && totals.1 <= i128::from(i64::MAX), "Device counters exceed the supported total");
    conn.execute_batch("CREATE TABLE IF NOT EXISTS backup_daily_contributions (source_id TEXT NOT NULL, day TEXT NOT NULL, context_uuid TEXT NOT NULL, total_words INTEGER NOT NULL, total_transcriptions INTEGER NOT NULL, speaking_ms INTEGER NOT NULL, wpm_sum REAL NOT NULL, wpm_count INTEGER NOT NULL, PRIMARY KEY(source_id, day, context_uuid))")?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS backup_insights_contributions (source_id TEXT NOT NULL, kind TEXT NOT NULL, bucket TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(source_id, kind, bucket))")?;
    for day in &activity.daily {
        conflicts += merge_daily(conn, activity.source_id.as_deref(), day, "", &overlap_days)?;
    }
    for entry in &activity.contexts {
        let local_uuid: Option<String> = if entry.context_uuid == "unscoped" {
            Some("unscoped".to_string())
        } else {
            match super::backup_history::context_id(conn, &entry.context_uuid, source_contexts)? {
                Some(id) => conn
                    .query_row("SELECT uuid FROM contexts WHERE id = ?1", [id], |r| {
                        r.get(0)
                    })
                    .optional()?,
                None => None,
            }
        };
        let Some(local_uuid) = local_uuid else {
            conflicts += 1;
            continue;
        };
        let day = &entry.daily;
        conflicts += merge_daily(conn, activity.source_id.as_deref(), day, &local_uuid, &overlap_days)?;
    }
    for row in &activity.hourly {
        let id = match row.context_uuid.as_deref() {
            Some(uuid) => match super::backup_history::context_id(conn, uuid, source_contexts)? {
                Some(id) => id,
                None => {
                    conflicts += 1;
                    continue;
                }
            },
            None => 0,
        };
        let bucket = serde_json::to_string(&(&row.day, row.hour, &row.context_uuid))?;
        let value = serde_json::to_string(row)?;
        let Some(source) = activity.source_id.as_deref() else { conflicts += 1; continue; };
        let prior: Option<String> = conn.query_row("SELECT value FROM backup_insights_contributions WHERE source_id=?1 AND kind='hourly' AND bucket=?2", params![source, bucket], |r| r.get(0)).optional()?;
        if let Some(prior) = prior {
            if prior != value { conflicts += 1; }
            continue;
        }
        if overlap_days.contains(&hour_bucket(&row.day, row.hour, id)?) { conflicts += 1; continue; }
        conn.execute("INSERT INTO transcription_hourly_stats(day, hour, context_id, total_words) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(day, hour, context_id) DO UPDATE SET total_words = total_words + excluded.total_words", params![row.day, row.hour, id, row.words])?;
        conn.execute("INSERT INTO backup_insights_contributions VALUES (?1, 'hourly', ?2, ?3)", params![source, bucket, value])?;
    }
    for row in &activity.providers {
        let id = match row.context_uuid.as_deref() {
            Some(uuid) => match super::backup_history::context_id(conn, uuid, source_contexts)? {
                Some(id) => id,
                None => {
                    conflicts += 1;
                    continue;
                }
            },
            None => 0,
        };
        let bucket = serde_json::to_string(&(&row.day, &row.context_uuid, &row.model, &row.provider, &row.task))?;
        let value = serde_json::to_string(row)?;
        let Some(source) = activity.source_id.as_deref() else { conflicts += 1; continue; };
        let prior: Option<String> = conn.query_row("SELECT value FROM backup_insights_contributions WHERE source_id=?1 AND kind='provider' AND bucket=?2", params![source, bucket], |r| r.get(0)).optional()?;
        if let Some(prior) = prior {
            if prior != value { conflicts += 1; }
            continue;
        }
        if overlap_days.contains(&provider_bucket(&row.day, id, &row.model, &row.provider, &row.task)?) { conflicts += 1; continue; }
        conn.execute("INSERT INTO provider_daily_stats(day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(day, context_id, model, provider, task) DO UPDATE SET calls = calls + excluded.calls, audio_ms = audio_ms + excluded.audio_ms, input_chars = input_chars + excluded.input_chars, output_chars = output_chars + excluded.output_chars", params![row.day, id, row.model, row.provider, row.task, row.calls, row.audio_ms, row.input_chars, row.output_chars])?;
        conn.execute("INSERT INTO backup_insights_contributions VALUES (?1, 'provider', ?2, ?3)", params![source, bucket, value])?;
    }
    validate_totals(conn)?;
    Ok(conflicts)
}

pub(crate) fn validate_totals(conn: &Connection) -> Result<()> {
    for sql in [
        "SELECT total_words, total_transcriptions, speaking_ms, wpm_count FROM transcription_daily_stats",
        "SELECT total_words, total_transcriptions, speaking_ms, wpm_count FROM transcription_context_daily_stats",
        "SELECT total_words, 0, 0, 0 FROM transcription_hourly_stats UNION ALL SELECT t.words, 0, 0, 0 FROM transcriptions t WHERE NOT EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid=t.uuid) AND NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=t.uuid)",
        "SELECT calls, audio_ms, input_chars, output_chars FROM provider_daily_stats UNION ALL SELECT 1, a.audio_ms, a.input_chars, a.output_chars FROM api_calls a WHERE NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid=a.uuid)",
    ] {
        let mut statement = conn.prepare(sql)?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)))?;
        let mut sums = [0i128; 4];
        for row in rows {
            let (a, b, c, d) = row?;
            for (total, value) in sums.iter_mut().zip([a, b, c, d]) { *total += i128::from(value); }
        }
        ensure!(sums.iter().all(|sum| *sum <= i128::from(i64::MAX) && *sum >= 0), "Activity totals exceed supported limits");
    }
    let invalid_wpm: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_daily_stats WHERE wpm_sum < 0 OR best_wpm < 0 OR wpm_sum > 1.7976931348623157e308 OR best_wpm > 1.7976931348623157e308 UNION ALL SELECT 1 FROM transcription_context_daily_stats WHERE wpm_sum < 0 OR best_wpm < 0 OR wpm_sum > 1.7976931348623157e308 OR best_wpm > 1.7976931348623157e308)", [], |row| row.get(0))?;
    ensure!(!invalid_wpm, "Invalid WPM total");
    let (sum, count): (f64, i64) = conn.query_row("SELECT wpm_sum, wpm_count FROM lifetime_stats WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    ensure!(sum.is_finite() && sum >= 0.0 && count >= 0, "Invalid lifetime WPM total");
    Ok(())
}

fn receipt_context(conn: &Connection, uuid: Option<&str>, contexts: &[super::ExportContext]) -> Result<(Option<i64>, Option<String>, bool)> {
    let Some(uuid) = uuid else { return Ok((None, None, false)); };
    let id: Option<i64> = conn.query_row("SELECT id FROM contexts WHERE uuid = ?1", [uuid], |r| r.get(0)).optional()?;
    let id = match id { Some(id) => Some(id), None => super::backup_history::context_id(conn, uuid, contexts)? };
    let local_uuid = id.map(|id| conn.query_row("SELECT uuid FROM contexts WHERE id = ?1", [id], |r| r.get(0))).transpose()?;
    Ok((id, local_uuid, id.is_none()))
}

pub(super) fn import_call_with_pruned_parent(conn: &Connection, call: &super::backup_history::Call, contexts: &[super::ExportContext]) -> Result<usize> {
    let parent: Option<(Option<String>, bool)> = conn.query_row("SELECT context_uuid, counted FROM transcription_receipts WHERE uuid = ?1", [&call.transcription_uuid], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((context_uuid, _)) = parent else { return Ok(1); };
    let day: String = conn.query_row("SELECT COALESCE(?1, date(?2, 'localtime'))", params![call.activity_day, call.created_at], |r| r.get(0))?;
    let activity = BackupActivity {
        lifetime: Lifetime::default(), daily: vec![], contexts: vec![], hourly: vec![], providers: vec![],
        source_id: None, remote_counters: vec![], transcription_receipts: vec![], legacy_wpm: vec![], context_provenance: vec![], context_aliases: None,
        call_receipts: vec![CallReceipt { uuid: call.uuid.clone(), transcription_uuid: Some(call.transcription_uuid.clone()), day,
            context_uuid, model: call.model.clone(), provider: call.provider.clone(), task: call.task.clone(),
            audio_ms: call.audio_ms, input_chars: call.input_chars, output_chars: call.output_chars, counted: !call.held }],
    };
    import_receipts(conn, &activity, contexts)
}

pub(super) fn import_expired_transcription(conn: &Connection, row: &super::backup_history::Transcription, contexts: &[super::ExportContext]) -> Result<usize> {
    let day: String = conn.query_row("SELECT COALESCE(?1, date(?2, 'localtime'))", params![row.activity_day, row.created_at], |r| r.get(0))?;
    let hour: i64 = conn.query_row("SELECT COALESCE(?1, CAST(strftime('%H', ?2, 'localtime') AS INTEGER))", params![row.activity_hour, row.created_at], |r| r.get(0))?;
    let activity = BackupActivity {
        lifetime: Lifetime::default(), daily: vec![], contexts: vec![], hourly: vec![], providers: vec![],
        source_id: None, remote_counters: vec![], call_receipts: vec![], legacy_wpm: vec![], context_provenance: vec![], context_aliases: None,
        transcription_receipts: vec![TranscriptionReceipt { uuid: row.uuid.clone(), day, hour, context_uuid: row.context_uuid.clone(), words: row.words, spoken_words: row.spoken_words, duration_ms: row.duration_ms, counted: !row.held, hourly_counted: !row.held && !row.hourly_held }],
    };
    import_receipts(conn, &activity, contexts)
}

fn import_receipts(conn: &Connection, activity: &BackupActivity, contexts: &[super::ExportContext]) -> Result<usize> {
    let mut conflicts = 0;
    for receipt in &activity.transcription_receipts {
        let pending: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [&receipt.uuid], |r| r.get(0)).optional()?;
        if pending.as_deref().is_some_and(|uuid| receipt.context_uuid.as_deref() != Some(uuid)) {
            conflicts += 1;
            continue;
        }
        let (context_id, context_uuid, unmatched) = receipt_context(conn, receipt.context_uuid.as_deref(), contexts)?;
        conflicts += usize::from(unmatched);
        let expected = pending.as_deref().or(context_uuid.as_deref()).or(receipt.context_uuid.as_deref());
        if crate::data::db::has_conflicting_call_context(conn, &receipt.uuid, expected)? {
            conflicts += 1;
            continue;
        }
        if let (Some(id), Some(local_uuid)) = (context_id, context_uuid.as_deref()) {
            let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p JOIN transcription_receipts r ON r.uuid=p.transcription_uuid WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND r.day=?3 AND r.hour=?4 AND r.words=?5 AND r.spoken_words IS ?6 AND r.duration_ms=?7)", params![receipt.uuid,receipt.context_uuid,receipt.day,receipt.hour,receipt.words,receipt.spoken_words,receipt.duration_ms], |r| r.get(0))?;
            let pending_retained: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p JOIN transcriptions t ON t.uuid=p.transcription_uuid WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND t.activity_day=?3 AND t.activity_hour=?4 AND t.words=?5 AND t.spoken_words IS ?6 AND t.duration_ms=?7)", params![receipt.uuid,receipt.context_uuid,receipt.day,receipt.hour,receipt.words,receipt.spoken_words,receipt.duration_ms], |r| r.get(0))?;
            let call_only: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND NOT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1) AND NOT EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1))", params![receipt.uuid,receipt.context_uuid], |r| r.get(0))?;
            if pending || pending_retained || call_only { crate::data::db::resolve_pending_context(conn, &receipt.uuid, id, local_uuid)?; }
        }
        let stored_context = context_uuid.as_deref().or(if unmatched { receipt.context_uuid.as_deref() } else { None });
        let retained: Option<bool> = conn.query_row("SELECT words = ?2 AND spoken_words IS ?3 AND duration_ms = ?4 AND context_id IS ?5 AND activity_day = ?6 AND activity_hour = ?7 FROM transcriptions WHERE uuid = ?1", params![receipt.uuid, receipt.words, receipt.spoken_words, receipt.duration_ms, context_id, receipt.day, receipt.hour], |r| r.get(0)).optional()?;
        if let Some(matches) = retained {
            conflicts += usize::from(!matches);
            if matches { super::backup_history::record_mapped_event(conn, &receipt.uuid, receipt.context_uuid.as_deref(), context_uuid.as_deref())?; }
            continue;
        }
        let existing: Option<bool> = conn.query_row("SELECT day = ?2 AND hour = ?3 AND context_uuid IS ?4 AND words = ?5 AND spoken_words IS ?6 AND duration_ms = ?7 FROM transcription_receipts WHERE uuid = ?1", params![receipt.uuid, receipt.day, receipt.hour, stored_context, receipt.words, receipt.spoken_words, receipt.duration_ms], |r| r.get(0)).optional()?;
        if let Some(matches) = existing {
            conflicts += usize::from(!matches);
            if matches { super::backup_history::record_mapped_event(conn, &receipt.uuid, receipt.context_uuid.as_deref(), context_uuid.as_deref())?; }
            if matches && receipt.counted {
                let (counted, hourly): (bool, bool) = conn.query_row("SELECT counted, NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=transcription_receipts.uuid) FROM transcription_receipts WHERE uuid=?1", [&receipt.uuid], |r| Ok((r.get(0)?, r.get(1)?)))?;
                conflicts += usize::from(!counted || (receipt.hourly_counted && !hourly));
            }
            continue;
        }
        let legacy: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM legacy_pruned_days WHERE day = ?1)", [&receipt.day], |r| r.get(0))?;
        let counted = receipt.counted && !legacy;
        conflicts += usize::from(!counted);
        let legacy_hour = crate::data::db::has_legacy_hour(conn, &receipt.day, receipt.hour, context_id, context_uuid.as_deref())?;
        let hourly_counted = counted && receipt.hourly_counted && !legacy_hour;
        if counted && !hourly_counted { conflicts += 1; }
        if !hourly_counted { conn.execute("INSERT OR IGNORE INTO backup_held_hourly_receipts(uuid) VALUES (?1)", [&receipt.uuid])?; }
        conn.execute("INSERT INTO transcription_receipts(uuid, day, hour, context_uuid, words, spoken_words, duration_ms, counted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![receipt.uuid, receipt.day, receipt.hour, stored_context, receipt.words, receipt.spoken_words, receipt.duration_ms, counted])?;
        super::backup_history::record_mapped_event(conn, &receipt.uuid, receipt.context_uuid.as_deref(), context_uuid.as_deref())?;
        if unmatched { conn.execute("INSERT OR IGNORE INTO pending_transcription_contexts(transcription_uuid,context_uuid) VALUES (?1,?2)", params![receipt.uuid,stored_context])?; }
        if !counted { continue; }
        let spoken = receipt.spoken_words.unwrap_or(receipt.words);
        let wpm_count = i64::from(receipt.duration_ms > 0 && spoken > 0);
        let wpm = if wpm_count > 0 { spoken as f64 * 60000.0 / receipt.duration_ms as f64 } else { 0.0 };
        conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?4) ON CONFLICT(day) DO UPDATE SET total_words=total_words+excluded.total_words, total_transcriptions=total_transcriptions+1, speaking_ms=speaking_ms+excluded.speaking_ms, wpm_sum=wpm_sum+excluded.wpm_sum, wpm_count=wpm_count+excluded.wpm_count, best_wpm=MAX(best_wpm,excluded.best_wpm)", params![receipt.day, receipt.words, receipt.duration_ms, wpm, wpm_count])?;
        conn.execute("INSERT INTO transcription_context_daily_stats(day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?5) ON CONFLICT(day, context_uuid) DO UPDATE SET total_words=total_words+excluded.total_words, total_transcriptions=total_transcriptions+1, speaking_ms=speaking_ms+excluded.speaking_ms, wpm_sum=wpm_sum+excluded.wpm_sum, wpm_count=wpm_count+excluded.wpm_count, best_wpm=MAX(best_wpm,excluded.best_wpm)", params![receipt.day, context_uuid.as_deref().unwrap_or("unscoped"), receipt.words, receipt.duration_ms, wpm, wpm_count])?;
        if hourly_counted { conn.execute("INSERT INTO transcription_hourly_stats(day, hour, context_id, total_words) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(day, hour, context_id) DO UPDATE SET total_words=total_words+excluded.total_words", params![receipt.day, receipt.hour, context_id.unwrap_or(0), receipt.words])?; }
        conn.execute("UPDATE lifetime_stats SET wpm_sum=wpm_sum+?1, wpm_count=wpm_count+?2 WHERE id=1", params![wpm, wpm_count])?;
    }
    for receipt in &activity.call_receipts {
        if let Some(parent) = receipt.transcription_uuid.as_deref() {
            let pending: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [parent], |r| r.get(0)).optional()?;
            if pending.as_deref().is_some_and(|uuid| receipt.context_uuid.as_deref() != Some(uuid)) {
                conflicts += 1;
                continue;
            }
        }
        let (context_id, context_uuid, unmatched) = receipt_context(conn, receipt.context_uuid.as_deref(), contexts)?;
        conflicts += usize::from(unmatched);
        if let Some(parent) = receipt.transcription_uuid.as_deref() {
            let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1)", [parent], |r| r.get(0))?;
            let parent_context: Option<Option<String>> = conn.query_row("SELECT c.uuid FROM transcriptions t LEFT JOIN contexts c ON c.id=t.context_id WHERE t.uuid=?1 UNION ALL SELECT context_uuid FROM transcription_receipts WHERE uuid=?1 LIMIT 1", [parent], |r| r.get(0)).optional()?;
            if !pending && parent_context.as_ref().is_some_and(|parent_context| parent_context != &context_uuid || unmatched) {
                conflicts += usize::from(!unmatched);
                continue;
            }
            let sibling_context: Option<Option<String>> = conn.query_row("SELECT context_uuid FROM call_receipts WHERE transcription_uuid=?1 LIMIT 1", [parent], |r| r.get(0)).optional()?;
            if !pending && sibling_context.as_ref().is_some_and(|sibling| sibling != &context_uuid || unmatched) {
                conflicts += usize::from(!unmatched);
                continue;
            }
        }
        if let (Some(id), Some(local_uuid), Some(parent)) = (context_id, context_uuid.as_deref(), receipt.transcription_uuid.as_deref()) {
            let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=?1 AND p.context_uuid IS ?2 AND NOT EXISTS(SELECT 1 FROM api_calls WHERE uuid=?3) AND (NOT EXISTS(SELECT 1 FROM call_receipts WHERE uuid=?3) OR EXISTS(SELECT 1 FROM call_receipts r WHERE r.uuid=?3 AND r.transcription_uuid IS ?1 AND r.day=?4 AND r.model=?5 AND r.provider=?6 AND r.task=?7 AND r.audio_ms=?8 AND r.input_chars=?9 AND r.output_chars=?10)))", params![parent,receipt.context_uuid,receipt.uuid,receipt.day,receipt.model,receipt.provider,receipt.task,receipt.audio_ms,receipt.input_chars,receipt.output_chars], |r| r.get(0))?;
            if pending { crate::data::db::resolve_pending_context(conn, parent, id, local_uuid)?; }
        }
        let retained: Option<bool> = conn.query_row("SELECT t.uuid IS ?2 AND a.model = ?3 AND a.provider = ?4 AND a.task = ?5 AND a.audio_ms = ?6 AND a.input_chars = ?7 AND a.output_chars = ?8 AND a.activity_day = ?9 AND t.context_id IS ?10 FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id WHERE a.uuid = ?1", params![receipt.uuid, receipt.transcription_uuid, receipt.model, receipt.provider, receipt.task, receipt.audio_ms, receipt.input_chars, receipt.output_chars, receipt.day, context_id], |r| r.get(0)).optional()?;
        if let Some(matches) = retained {
            conflicts += usize::from(!matches);
            if matches { if let Some(parent) = receipt.transcription_uuid.as_deref() { super::backup_history::record_mapped_event(conn, parent, receipt.context_uuid.as_deref(), context_uuid.as_deref())?; } }
            continue;
        }
        let mut call = crate::data::db::CompactedCall { uuid: &receipt.uuid, transcription_uuid: receipt.transcription_uuid.as_deref(), day: &receipt.day, context_uuid: context_uuid.as_deref(), context_id, model: &receipt.model, provider: &receipt.provider, task: &receipt.task, audio_ms: receipt.audio_ms, input_chars: receipt.input_chars, output_chars: receipt.output_chars, counted: receipt.counted };
        call.counted &= !crate::data::db::has_legacy_provider(conn, &call)?;
        let mut pending_to_insert = None;
        if unmatched {
            if let Some(parent) = receipt.transcription_uuid.as_deref() {
                let parent_exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [parent], |r| r.get(0))?;
                let pending_matches: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1 AND context_uuid IS ?2)", params![parent,receipt.context_uuid], |r| r.get(0))?;
                if pending_matches || !parent_exists { call.context_uuid = receipt.context_uuid.as_deref(); }
                if !parent_exists && !pending_matches { pending_to_insert = Some(parent); }
            }
        }
        match crate::data::db::insert_compacted_call(conn, &call)? {
            crate::data::db::ReceiptOutcome::Inserted => {
                if let Some(parent) = receipt.transcription_uuid.as_deref() { super::backup_history::record_mapped_event(conn, parent, receipt.context_uuid.as_deref(), context_uuid.as_deref())?; }
                if let Some(parent) = pending_to_insert { conn.execute("INSERT OR IGNORE INTO pending_transcription_contexts(transcription_uuid,context_uuid) VALUES (?1,?2)", params![parent,receipt.context_uuid])?; }
                conflicts += usize::from(!call.counted);
            },
            crate::data::db::ReceiptOutcome::Conflict => conflicts += 1,
            crate::data::db::ReceiptOutcome::Existing => {
                if let Some(parent) = receipt.transcription_uuid.as_deref() { super::backup_history::record_mapped_event(conn, parent, receipt.context_uuid.as_deref(), context_uuid.as_deref())?; }
                let counted: bool = conn.query_row("SELECT counted FROM call_receipts WHERE uuid=?1", [&receipt.uuid], |r| r.get(0))?;
                conflicts += usize::from(receipt.counted && !counted);
            }
        }
    }
    Ok(conflicts)
}

fn merge_daily(conn: &Connection, source_id: Option<&str>, day: &Daily, context: &str, existing_days: &HashSet<String>) -> Result<usize> {
    if day.total_transcriptions == 0 { return Ok(0); }
    let Some(source_id) = source_id else { return Ok(1); };
    let prior: Option<(i64, i64, i64, f64, i64)> = conn.query_row("SELECT total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count FROM backup_daily_contributions WHERE source_id = ?1 AND day = ?2 AND context_uuid = ?3", params![source_id, day.day, context], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
    if prior.is_none() && existing_days.contains(&day.day) { return Ok(1); }
    let prior = prior.unwrap_or((0, 0, 0, 0.0, 0));
    if day.total_transcriptions <= prior.1 {
        return Ok(usize::from(day.total_transcriptions == prior.1 && (day.total_words != prior.0 || day.speaking_ms != prior.2 || day.wpm_count != prior.4)));
    }
    // A later snapshot can include records that this destination already
    // restored while they were retained on the source. Without event-level
    // provenance, treating that growth as new activity would count them twice.
    if prior.1 > 0 { return Ok(1); }
    if day.total_words < prior.0 || day.speaking_ms < prior.2 || day.wpm_sum < prior.3 || day.wpm_count < prior.4 { return Ok(1); }
    let (words, count, speaking, sum, wpm_count) = (day.total_words - prior.0, day.total_transcriptions - prior.1, day.speaking_ms - prior.2, day.wpm_sum - prior.3, day.wpm_count - prior.4);
    if context.is_empty() {
        conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(day) DO UPDATE SET total_words = total_words + excluded.total_words, total_transcriptions = total_transcriptions + excluded.total_transcriptions, speaking_ms = speaking_ms + excluded.speaking_ms, wpm_sum = wpm_sum + excluded.wpm_sum, wpm_count = wpm_count + excluded.wpm_count, best_wpm = MAX(best_wpm, excluded.best_wpm)", params![day.day, words, count, speaking, sum, wpm_count, day.best_wpm])?;
        conn.execute("INSERT INTO lifetime_stats(id, wpm_sum, wpm_count) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET wpm_sum = wpm_sum + excluded.wpm_sum, wpm_count = wpm_count + excluded.wpm_count", params![sum, wpm_count])?;
    } else {
        conn.execute("INSERT INTO transcription_context_daily_stats(day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(day, context_uuid) DO UPDATE SET total_words = total_words + excluded.total_words, total_transcriptions = total_transcriptions + excluded.total_transcriptions, speaking_ms = speaking_ms + excluded.speaking_ms, wpm_sum = wpm_sum + excluded.wpm_sum, wpm_count = wpm_count + excluded.wpm_count, best_wpm = MAX(best_wpm, excluded.best_wpm)", params![day.day, context, words, count, speaking, sum, wpm_count, day.best_wpm])?;
    }
    conn.execute("INSERT INTO backup_daily_contributions(source_id, day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(source_id, day, context_uuid) DO UPDATE SET total_words=excluded.total_words, total_transcriptions=excluded.total_transcriptions, speaking_ms=excluded.speaking_ms, wpm_sum=excluded.wpm_sum, wpm_count=excluded.wpm_count", params![source_id, day.day, context, day.total_words, day.total_transcriptions, day.speaking_ms, day.wpm_sum, day.wpm_count])?;
    conn.execute("INSERT OR IGNORE INTO legacy_pruned_days(day) VALUES (?1)", [&day.day])?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_preupgrade_ambiguous_history_does_not_duplicate_words() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            crate::sync::store::ensure_self_identity(&conn, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test','2020-01-01 01:00:00','2020-01-01',1)", []).unwrap();
            conn.execute("UPDATE transcription_daily_stats SET day='2019-12-31' WHERE day='2020-01-01'", []).unwrap();
            conn.execute("INSERT INTO backup_preupgrade_history VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
        }
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        let conn = source.lock().unwrap();
        assert!(!conn.query_row("SELECT counted FROM transcription_receipts WHERE uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_, bool>(0)).unwrap());
        let activity = export_activity_conn(&conn).unwrap();
        drop(conn);
        let conn = dest.lock().unwrap();
        import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn deleting_context_keeps_best_wpm_from_compacted_activity() {
        let db = db::open(":memory:").unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
            let receipt = serde_json::json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":5,"duration_ms":3000});
            import_sync_receipt(&conn, "transcription_receipts", receipt).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc','a','a',1,6000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Synthetic'", []).unwrap();
        }
        let id: i64 = db.lock().unwrap().query_row("SELECT id FROM contexts WHERE name='Synthetic'", [], |r| r.get(0)).unwrap();
        db::delete_context(&db, id).unwrap();
        let conn = db.lock().unwrap();
        let best: f64 = conn.query_row("SELECT best_wpm FROM transcription_context_daily_stats WHERE context_uuid=(SELECT uuid FROM contexts WHERE is_everywhere=1)", [], |r| r.get(0)).unwrap();
        assert_eq!(best, 100.0);
    }

    #[test]
    fn deleting_pending_context_does_not_duplicate_restored_provider_usage() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            let parent = serde_json::json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
            let call = serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
            import_sync_receipt(&conn, "transcription_receipts", parent).unwrap();
            import_sync_receipt(&conn, "call_receipts", call).unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','To delete')", []).unwrap();
        }
        let id: i64 = source.lock().unwrap().query_row("SELECT id FROM contexts WHERE name='To delete'", [], |r| r.get(0)).unwrap();
        db::delete_context(&source, id).unwrap();
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
        assert_eq!(conn.query_row("SELECT COALESCE(SUM(calls),0) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COALESCE(SUM(total_words),0) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn matching_receipt_resolves_retained_pending_context_by_name() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
        conn.execute("INSERT INTO pending_transcription_contexts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','Synthetic')", []).unwrap();
        let context: super::super::ExportContext = serde_json::from_value(serde_json::json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","name":"Synthetic"})).unwrap();
        let activity = BackupActivity { lifetime:Lifetime::default(),daily:vec![],contexts:vec![],hourly:vec![],providers:vec![],source_id:None,remote_counters:vec![],call_receipts:vec![],legacy_wpm:vec![],context_provenance:vec![],context_aliases:None,transcription_receipts:vec![TranscriptionReceipt { uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),day:"2020-01-01".into(),hour:10,context_uuid:Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()),words:5,spoken_words:None,duration_ms:1000,counted:true,hourly_counted:true }] };
        assert_eq!(import_receipts(&conn, &activity, &[context]).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT c.uuid FROM transcriptions t JOIN contexts c ON c.id=t.context_id", [], |r| r.get::<_, String>(0)).unwrap(), "cccccccc-cccc-4ccc-8ccc-cccccccccccc");
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn orphan_call_context_rejects_conflicting_parent_and_accepts_matching_retry() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','First'),('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Second')", []).unwrap();
        let call = serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        import_sync_receipt(&conn, "call_receipts", call).unwrap();
        let mut parent = serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        assert!(import_sync_receipt(&conn, "transcription_receipts", parent.clone()).unwrap() > 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        parent["context_uuid"] = serde_json::json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        assert_eq!(import_sync_receipt(&conn, "transcription_receipts", parent).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let backup = export_activity_conn(&conn).unwrap();
        let restored = db::open(":memory:").unwrap();
        let restored_conn = restored.lock().unwrap();
        restored_conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','First')", []).unwrap();
        import_activity(&restored_conn, &backup, &[], &HashSet::new()).unwrap();
        assert_eq!(restored_conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn new_sibling_resolves_pending_calls_before_counting() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let first = serde_json::json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        let second = serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        import_sync_receipt(&conn, "call_receipts", first.clone()).unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','Recovered')", []).unwrap();
        import_sync_receipt(&conn, "call_receipts", second).unwrap();
        import_sync_receipt(&conn, "call_receipts", first).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts WHERE counted=1", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn orphan_calls_keep_pending_context_and_reject_conflicting_siblings() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Known')", []).unwrap();
        let first = serde_json::json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","transcription_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        assert_eq!(import_sync_receipt(&conn, "call_receipts", first.clone()).unwrap(), 0);
        let different = serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","context_uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        assert!(import_sync_receipt(&conn, "call_receipts", different).unwrap() > 0);
        let duplicate = serde_json::json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","transcription_uuid":"ffffffff-ffff-4fff-8fff-ffffffffffff","day":"2020-01-01","context_uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        assert!(import_sync_receipt(&conn, "call_receipts", duplicate).unwrap() > 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        drop(conn);
        let orphan = db::open(":memory:").unwrap();
        let call = serde_json::json!({"uuid":"11111111-1111-4111-8111-111111111111","transcription_uuid":"22222222-2222-4222-8222-222222222222","day":"2020-01-01","context_uuid":"33333333-3333-4333-8333-333333333333","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        import_sync_receipt(&orphan.lock().unwrap(), "call_receipts", call).unwrap();
        db::prune_transcriptions_older_than(&orphan, 7).unwrap();
        assert_eq!(orphan.lock().unwrap().query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn call_before_parent_does_not_duplicate_context_activity() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let call = serde_json::json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        import_sync_receipt(&conn, "call_receipts", call.clone()).unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('dddddddd-dddd-4ddd-8ddd-dddddddddddd','Other')", []).unwrap();
        let conflicting = serde_json::json!({"uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        assert!(import_sync_receipt(&conn, "call_receipts", conflicting).unwrap() > 0);
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','Recovered')", []).unwrap();
        let parent = serde_json::json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","hour":10,"context_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","words":5,"spoken_words":null,"duration_ms":1000});
        import_sync_receipt(&conn, "transcription_receipts", parent).unwrap();
        import_sync_receipt(&conn, "call_receipts", call).unwrap();
        let conflicting = serde_json::json!({"uuid":"ffffffff-ffff-4fff-8fff-ffffffffffff","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        assert!(import_sync_receipt(&conn, "call_receipts", conflicting).unwrap() > 0);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE context_id=(SELECT id FROM contexts WHERE name='Recovered')", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn backup_before_pending_context_arrives_does_not_duplicate_compacted_usage() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
            conn.execute("INSERT INTO pending_transcription_contexts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        {
            let conn = source.lock().unwrap();
            assert_eq!(import_sync_receipt(&conn, "transcription_receipts", serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","hour":10,"context_uuid":null,"words":7,"spoken_words":null,"duration_ms":1000})).unwrap(), 0);
            assert_eq!(import_sync_receipt(&conn, "call_receipts", serde_json::json!({"uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","transcription_uuid":null,"day":"2020-01-01","context_uuid":null,"model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0})).unwrap(), 0);
        }
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_context_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Recovered')", []).unwrap();
        import_activity(&conn, &activity, &[], &existing_days(&conn).unwrap()).unwrap();
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT calls FROM provider_daily_stats WHERE context_id=(SELECT id FROM contexts WHERE name='Recovered')", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn hourly_only_legacy_overlap_does_not_duplicate_receipt_hour() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcription_receipts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','2020-01-01',10,NULL,5,NULL,1000,1)", []).unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day,total_words,total_transcriptions) VALUES ('2020-01-01',5,1)", []).unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01',10,0,5)", []).unwrap();
        }
        dest.lock().unwrap().execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01',10,0,5)", []).unwrap();
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        for _ in 0..2 { assert!(import_activity(&conn, &activity, &[], &existing_days(&conn).unwrap()).unwrap() > 0); }
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE day='2020-01-01' AND hour=10", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(export_activity_conn(&conn).unwrap().hourly[0].words, 5);
    }

    #[test]
    fn provider_only_legacy_activity_does_not_hold_dictation_receipts() {
        let source = db::open(":memory:").unwrap();
        let legacy = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = legacy.lock().unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01', 0, 'legacy', 'test', 'stt', 1, 100, 0, 0)", []).unwrap();
        }
        let legacy_activity = export_activity(&legacy).unwrap();
        import_activity(&dest.lock().unwrap(), &legacy_activity, &[], &HashSet::new()).unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcription_receipts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', '2020-01-01', 10, NULL, 5, NULL, 1000, 1)", []).unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day,total_words,total_transcriptions) VALUES ('2020-01-01',5,1)", []).unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01',10,0,5)", []).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        for _ in 0..2 {
            let days = existing_days(&conn).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn same_device_restores_unattributed_wpm_once() {
        let source = db::open(":memory:").unwrap();
        let relay = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        for db in [&source, &dest] {
            crate::sync::store::ensure_self_identity(&db.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "test").unwrap();
        }
        source.lock().unwrap().execute("UPDATE lifetime_stats SET wpm_sum=120, wpm_count=1 WHERE id=1", []).unwrap();
        let activity = export_activity(&source).unwrap();
        import_activity(&relay.lock().unwrap(), &activity, &[], &HashSet::new()).unwrap();
        let relayed = export_activity(&relay).unwrap();
        let conn = dest.lock().unwrap();
        for snapshot in [&relayed, &activity] { assert_eq!(import_activity(&conn, snapshot, &[], &existing_days(&conn).unwrap()).unwrap(), 0); }
        assert_eq!(conn.query_row("SELECT wpm_sum, wpm_count FROM lifetime_stats WHERE id=1", [], |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)?))).unwrap(), (120.0, 1));
    }

    #[test]
    fn provider_receipts_and_legacy_insights_merge_distinct_buckets() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO call_receipts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', NULL, '2020-01-01', NULL, 'new-model', 'new-provider', 'stt', 100, 0, 0, 1)", []).unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01', 0, 'new-model', 'new-provider', 'stt', 1, 100, 0, 0)", []).unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01', 0, 'other-model', 'other-provider', 'stt', 1, 100, 0, 0)", []).unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01', 11, 0, 3)", []).unwrap();
        }
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO legacy_pruned_days VALUES ('2020-01-01')", []).unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01', 0, 'old-model', 'old-provider', 'stt', 1, 100, 0, 0)", []).unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01', 10, 0, 2)", []).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        for _ in 0..2 {
            let days = existing_days(&conn).unwrap();
            assert_eq!(import_activity(&conn, &activity, &[], &days).unwrap(), 0);
        }
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn reexport_restores_original_device_counter() {
        let source = db::open(":memory:").unwrap();
        let relay = db::open(":memory:").unwrap();
        let restored = db::open(":memory:").unwrap();
        for (db, id) in [(&source, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"), (&relay, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"), (&restored, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")] {
            crate::sync::store::ensure_self_identity(&db.lock().unwrap(), id, "test").unwrap();
        }
        source.lock().unwrap().execute("UPDATE lifetime_stats SET total_words=10, dictionary_fixes=4 WHERE id=1", []).unwrap();
        restored.lock().unwrap().execute("UPDATE lifetime_stats SET total_words=5, dictionary_fixes=2 WHERE id=1", []).unwrap();
        let activity = export_activity(&source).unwrap();
        import_activity(&relay.lock().unwrap(), &activity, &[], &HashSet::new()).unwrap();
        let relayed = export_activity(&relay).unwrap();
        let conn = restored.lock().unwrap();
        for _ in 0..2 { import_activity(&conn, &relayed, &[], &existing_days(&conn).unwrap()).unwrap(); }
        assert_eq!(conn.query_row("SELECT total_words, dictionary_fixes FROM lifetime_stats WHERE id=1", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap(), (10, 4));
    }

    #[test]
    fn receipt_conflicts_with_a_retained_record_in_a_different_calendar_bucket() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at, activity_day, activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'a', 'a', 1, 1000, 'test', '2020-01-01 01:00:00', '2019-12-31', 17)", []).unwrap();
        let receipt = serde_json::json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "day":"2020-01-01", "hour":1, "context_uuid":null, "words":1, "spoken_words":null, "duration_ms":1000, "counted":true});
        assert_eq!(import_sync_receipt(&conn, "transcription_receipts", receipt).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn legacy_and_identified_insights_in_one_bucket_are_added_once() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            crate::sync::store::ensure_self_identity(&conn, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at, activity_day, activity_hour) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'a', 'a', 5, 1000, 'test', '2020-01-01 10:00:00', '2020-01-01', 10)", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid, transcription_id, model, provider, task, audio_ms, created_at, activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', id, 'synthetic', 'test', 'stt', 1000, '2020-01-01 10:00:00', '2020-01-01' FROM transcriptions", []).unwrap();
        }
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("UPDATE transcription_hourly_stats SET total_words=total_words+10 WHERE day='2020-01-01' AND hour=10", []).unwrap();
            conn.execute("UPDATE provider_daily_stats SET calls=calls+2, audio_ms=audio_ms+2000 WHERE day='2020-01-01'", []).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        assert_eq!(import_activity(&conn, &activity, &[], &HashSet::new()).unwrap(), 0);
        let days = existing_days(&conn).unwrap();
        assert_eq!(import_activity(&conn, &activity, &[], &days).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE day='2020-01-01' AND hour=10", [], |r| r.get::<_, i64>(0)).unwrap(), 15);
        assert_eq!(conn.query_row("SELECT calls FROM provider_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
    }

    #[test]
    fn restored_receipts_propagate_to_a_synced_peer() {
        let source = db::open(":memory:").unwrap();
        let restored = db::open(":memory:").unwrap();
        let peer = db::open(":memory:").unwrap();
        for (db, id) in [(&source, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"), (&restored, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"), (&peer, "cccccccc-cccc-4ccc-8ccc-cccccccccccc")] {
            crate::sync::store::ensure_self_identity(&db.lock().unwrap(), id, "test").unwrap();
        }
        let entry = db::insert_transcription_returning(&source, "one two", "one two", 2, 1000, "test", None, None).unwrap();
        source.lock().unwrap().execute("UPDATE transcriptions SET created_at='2020-01-01 12:00:00' WHERE id=?1", [entry.id]).unwrap();
        db::insert_api_calls(&source, &[db::ApiCall { transcription_id:entry.id, model:"synthetic".into(), provider:"test".into(), task:"stt".into(), audio_ms:1000, input_chars:0, output_chars:1, created_at:"2020-01-01 12:00:00".into() }]).unwrap();
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        let activity = export_activity(&source).unwrap();
        import_activity(&restored.lock().unwrap(), &activity, &[], &HashSet::new()).unwrap();
        let (ops, _, done) = crate::sync::engine::collect_ops(&restored.lock().unwrap(), 0, true, 1000, &mut crate::sync::engine::SnapshotProgress::default()).unwrap();
        assert!(done);
        let conn = peer.lock().unwrap();
        crate::sync::engine::apply_ops(&conn, &ops).unwrap();
        crate::sync::engine::apply_ops(&conn, &ops).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn activity_round_trip_and_repeat_import_preserves_daily_and_lifetime() {
        let source = db::open(":memory:").unwrap();
        db::insert_transcription_returning(
            &source,
            "synthetic words",
            "synthetic words",
            2,
            1000,
            "test",
            None,
            None,
        )
        .unwrap();
        db::increment_lifetime_dictionary_fixes(&source, 3).unwrap();
        crate::sync::store::ensure_self_identity(&source.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
        let activity = export_activity(&source).unwrap();
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        for _ in 0..2 {
            let conn = dest.lock().unwrap();
            let days = existing_days(&conn).unwrap();
            super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        assert_eq!(db::query_stats(&dest).unwrap().total_words, 2);
        assert_eq!(dest.lock().unwrap().query_row("SELECT SUM(dictionary_fixes) FROM backup_device_counters", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(db::query_stats(&dest).unwrap().day_streak, 1);
    }

    #[test]
    fn invalid_activity_is_rejected_before_any_write() {
        let dest = db::open(":memory:").unwrap();
        let mut activity = export_activity(&dest).unwrap();
        activity.lifetime.total_words = 99;
        activity.daily.push(Daily {
            day: "bad".into(),
            total_words: 1,
            total_transcriptions: 1,
            speaking_ms: 1,
            wpm_sum: 1.0,
            wpm_count: 1,
            best_wpm: 1.0,
        });
        assert!(import_activity(&dest.lock().unwrap(), &activity, &[], &HashSet::new()).is_err());
        assert_eq!(export_activity(&dest).unwrap().lifetime.total_words, 0);
    }

    #[test]
    fn counter_overflow_rejects_and_rolls_back_import() {
        let dest = db::open(":memory:").unwrap();
        let mut activity = export_activity(&dest).unwrap();
        activity.source_id = Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into());
        activity.lifetime.total_words = i64::MAX;
        activity.remote_counters.push(RemoteCounter {
            device_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
            total_words: 1,
            dictionary_fixes: 0,
        });
        let mut conn = dest.lock().unwrap();
        let tx = conn.transaction().unwrap();
        assert!(import_activity(&tx, &activity, &[], &HashSet::new()).is_err());
        tx.rollback().unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_device_counters", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn unattributed_lifetime_wpm_round_trips_through_another_device_once() {
        let a = db::open(":memory:").unwrap();
        let b = db::open(":memory:").unwrap();
        let c = db::open(":memory:").unwrap();
        a.lock().unwrap().execute("UPDATE lifetime_stats SET wpm_sum=120, wpm_count=1 WHERE id=1", []).unwrap();
        let first = export_activity(&a).unwrap();
        import_activity(&b.lock().unwrap(), &first, &[], &HashSet::new()).unwrap();
        let second = export_activity(&b).unwrap();
        let conn = c.lock().unwrap();
        for activity in [&first, &second] {
            assert_eq!(import_activity(&conn, activity, &[], &HashSet::new()).unwrap(), 0);
        }
        let totals: (f64, i64) = conn.query_row("SELECT wpm_sum, wpm_count FROM lifetime_stats WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(totals, (120.0, 1));
    }

    #[test]
    fn growing_compacted_snapshot_is_reported_instead_of_double_counted() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES ('2020-01-01', 10, 1, 1000, 60, 1, 60)", []).unwrap();
        }
        let mut activity = export_activity(&source).unwrap();
        {
            let conn = dest.lock().unwrap();
            import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
        }
        activity.daily[0].total_words = 30;
        activity.daily[0].total_transcriptions = 2;
        activity.daily[0].speaking_ms = 2000;
        activity.daily[0].wpm_sum = 120.0;
        activity.daily[0].wpm_count = 2;
        let conn = dest.lock().unwrap();
        let days = existing_days(&conn).unwrap();
        assert!(import_activity(&conn, &activity, &[], &days).unwrap() > 0);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 10);
    }

    #[test]
    fn existing_larger_day_and_lifetime_are_not_reduced() {
        let dest = db::open(":memory:").unwrap();
        db::insert_transcription_returning(
            &dest, "one two", "one two", 2, 1000, "test", None, None,
        )
        .unwrap();
        db::insert_transcription_returning(
            &dest,
            "three four",
            "three four",
            2,
            1000,
            "test",
            None,
            None,
        )
        .unwrap();
        let before = export_activity(&dest).unwrap();
        let mut smaller = before.clone();
        smaller.lifetime.total_words = 1;
        smaller.daily[0].total_words = 1;
        smaller.daily[0].total_transcriptions = 1;
        smaller.daily[0].wpm_count = 1;
        let days = existing_days(&dest.lock().unwrap()).unwrap();
        import_activity(&dest.lock().unwrap(), &smaller, &[], &days).unwrap();
        assert_eq!(export_activity(&dest).unwrap(), before);
    }

    #[test]
    fn compacted_hourly_and_provider_usage_round_trip_without_duplicates() {
        let source = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute(
                "INSERT INTO transcription_hourly_stats VALUES ('2020-01-01', 10, 0, 42)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01', 0, 'synthetic', 'test', 'stt', 2, 3000, 4, 5)", []).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let dest = db::open(":memory:").unwrap();
        for _ in 0..2 {
            import_activity(&dest.lock().unwrap(), &activity, &[], &HashSet::new()).unwrap();
        }
        let restored = export_activity(&dest).unwrap();
        assert_eq!(restored.hourly, activity.hourly);
        assert_eq!(restored.providers, activity.providers);
    }

    #[test]
    fn disjoint_pruned_activity_merges_by_device_and_survives_repeat() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        for (db, day, words, fixes, sum) in [(&source, "2020-01-01", 100, 3, 120.0), (&dest, "2020-01-02", 50, 2, 60.0)] {
            let conn = db.lock().unwrap();
            conn.execute("INSERT OR REPLACE INTO lifetime_stats(id, total_words, dictionary_fixes, wpm_sum, wpm_count) VALUES (1, ?1, ?2, ?3, 1)", params![words, fixes, sum]).unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, 1, 1000, ?3, 1, ?3)", params![day, words, sum]).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        for _ in 0..2 {
            let conn = dest.lock().unwrap();
            let days = existing_days(&conn).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        let stats = db::query_stats(&dest).unwrap();
        assert_eq!(stats.total_words, 150);
        assert_eq!(stats.avg_wpm, 90.0);
        let conn = dest.lock().unwrap();
        assert_eq!(conn.query_row("SELECT SUM(dictionary_fixes) FROM backup_device_counters", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 150);
    }

    #[test]
    fn independent_compacted_activity_on_the_same_day_is_preserved() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        for (db, id, words) in [(&source, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", 5), (&dest, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", 7)] {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES (?1, 'synthetic', 'synthetic', ?2, 1000, 'test', '2020-01-01 12:00:00')", params![id, words]).unwrap();
            drop(conn);
            db::prune_transcriptions_older_than(db, 7).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let conn = dest.lock().unwrap();
        for _ in 0..2 {
            let days = existing_days(&conn).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 12);
    }

    #[test]
    fn compacted_events_reexport_through_a_third_device_without_duplicates() {
        let a = db::open(":memory:").unwrap();
        let b = db::open(":memory:").unwrap();
        let c = db::open(":memory:").unwrap();
        for (db, id, words) in [(&a, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", 5), (&b, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", 7), (&c, "cccccccc-cccc-4ccc-8ccc-cccccccccccc", 9)] {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES (?1, 'synthetic', 'synthetic', ?2, 1000, 'test', '2020-01-01 12:00:00')", params![id, words]).unwrap();
            drop(conn);
            db::prune_transcriptions_older_than(db, 7).unwrap();
        }
        let a_export = export_activity(&a).unwrap();
        {
            let conn = b.lock().unwrap();
            let days = existing_days(&conn).unwrap();
            import_activity(&conn, &a_export, &[], &days).unwrap();
        }
        let b_export = export_activity(&b).unwrap();
        let conn = c.lock().unwrap();
        for activity in [&a_export, &b_export, &a_export, &b_export] {
            let encoded = serde_json::to_string(activity).unwrap();
            let decoded: BackupActivity = serde_json::from_str(&encoded).unwrap();
            let days = existing_days(&conn).unwrap();
            assert_eq!(import_activity(&conn, &decoded, &[], &days).unwrap(), 0);
        }
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 21);
    }

    #[test]
    fn backup_after_source_pruning_does_not_count_restored_history_twice() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcription_daily_stats(day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES ('2020-01-01', 10, 1, 1000, 60, 1, 60)", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'synthetic', 'synthetic', 20, 20, 1000, 'test', '2020-01-01 12:00:00')", []).unwrap();
        }
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let first = export_activity(&source).unwrap();
        {
            let conn = dest.lock().unwrap();
            let days = existing_days(&conn).unwrap();
            super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
            import_activity(&conn, &first, &[], &days).unwrap();
        }
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        let second = export_activity(&source).unwrap();
        let empty_history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        let days = existing_days(&conn).unwrap();
        super::super::backup_history::import_history(&conn, &empty_history, &[]).unwrap();
        let conflicts = import_activity(&conn, &second, &[], &days).unwrap();
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 30);
        assert_eq!(conflicts, 0, "known restored records can be reconciled without an ambiguous conflict");
    }

    #[test]
    fn retained_history_is_not_reimported_as_a_second_source_calendar_day() {
        let source = db::open(":memory:").unwrap();
        db::insert_transcription_returning(&source, "synthetic words", "synthetic words", 2, 1000, "test", None, None).unwrap();
        let mut activity = export_activity(&source).unwrap();
        assert_eq!(activity.daily[0].total_transcriptions, 0);
        activity.daily[0].day = "2000-01-01".into();
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = dest.lock().unwrap();
        super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
        import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_transcriptions) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT count(*) FROM transcription_daily_stats WHERE day = '2000-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn recorded_calendar_bucket_survives_import_and_pruning() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at, activity_day, activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'one', 'one', 1, 1000, 'test', '2020-01-01 01:00:00', '2019-12-31', 17)", []).unwrap();
        }
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let activity = export_activity(&source).unwrap();
        {
            let conn = dest.lock().unwrap();
            super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
            import_activity(&conn, &activity, &[], &HashSet::new()).unwrap();
            assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day = '2019-12-31'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_daily_stats WHERE day = '2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        let conn = dest.lock().unwrap();
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE day = '2019-12-31' AND hour = 17", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn preupgrade_history_round_trip_preserves_summary_and_hourly_data() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let entry = db::insert_transcription_returning(&source, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("UPDATE transcriptions SET created_at='2020-01-01 12:00:00' WHERE id=?1", [entry.id]).unwrap();
            conn.execute("INSERT INTO backup_preupgrade_history(uuid) SELECT uuid FROM transcriptions WHERE id=?1", [entry.id]).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let conn = dest.lock().unwrap();
        for _ in 0..2 {
            let days = existing_days(&conn).unwrap();
            super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn compacted_insights_and_restored_history_in_same_period_both_survive() {
        let source = db::open(":memory:").unwrap();
        let row = db::insert_transcription_returning(&source, "synthetic words", "synthetic words", 2, 1000, "test", None, None).unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("UPDATE transcription_daily_stats SET total_words = total_words + 3, total_transcriptions = total_transcriptions + 1", []).unwrap();
            conn.execute("UPDATE transcription_context_daily_stats SET total_words = total_words + 3, total_transcriptions = total_transcriptions + 1", []).unwrap();
            conn.execute("UPDATE lifetime_stats SET total_words = total_words + 3 WHERE id = 1", []).unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats SELECT date(created_at, 'localtime'), CAST(strftime('%H', created_at, 'localtime') AS INTEGER), 0, 3 FROM transcriptions WHERE id = ?1", [row.id]).unwrap();
            conn.execute("INSERT INTO provider_daily_stats SELECT date(created_at, 'localtime'), 0, 'synthetic', 'test', 'stt', 1, 1000, 0, 2 FROM transcriptions WHERE id = ?1", [row.id]).unwrap();
            conn.execute("INSERT INTO api_calls(transcription_id, model, provider, task, created_at) SELECT id, 'synthetic', 'test', 'stt', created_at FROM transcriptions WHERE id = ?1", [row.id]).unwrap();
        }
        let activity = export_activity(&source).unwrap();
        let history = super::super::backup_history::export_history_conn(&source.lock().unwrap()).unwrap();
        let dest = db::open(":memory:").unwrap();
        for _ in 0..2 {
            let conn = dest.lock().unwrap();
            let days = existing_days(&conn).unwrap();
            super::super::backup_history::import_history(&conn, &history, &[]).unwrap();
            import_activity(&conn, &activity, &[], &days).unwrap();
        }
        let conn = dest.lock().unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
}
