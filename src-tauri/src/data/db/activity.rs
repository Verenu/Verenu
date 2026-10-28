use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

pub(crate) struct CompactedCall<'a> {
    pub uuid: &'a str,
    pub transcription_uuid: Option<&'a str>,
    pub day: &'a str,
    pub context_uuid: Option<&'a str>,
    pub context_id: Option<i64>,
    pub model: &'a str,
    pub provider: &'a str,
    pub task: &'a str,
    pub audio_ms: i64,
    pub input_chars: i64,
    pub output_chars: i64,
    pub counted: bool,
}

pub(crate) enum ReceiptOutcome { Inserted, Existing, Conflict }

type PendingReceipt = (String, i64, i64, Option<i64>, i64, bool, bool);
type PendingCall = (String, String, String, String, String, i64, i64, i64);

pub(crate) fn has_conflicting_call_context(conn: &Connection, parent: &str, context_uuid: Option<&str>) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid IS NOT ?2)", params![parent,context_uuid], |r| r.get(0))?)
}

pub(crate) fn resolve_pending_contexts(conn: &Connection, source_uuid: &str, context_id: i64, context_uuid: &str) -> Result<()> {
    let pending: Vec<String> = {
        let mut stmt = conn.prepare("SELECT transcription_uuid FROM pending_transcription_contexts WHERE context_uuid=?1")?;
        let rows = stmt.query_map([source_uuid], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for uuid in pending { resolve_pending_context(conn, &uuid, context_id, context_uuid)?; }
    Ok(())
}

pub(crate) fn resolve_pending_context(conn: &Connection, uuid: &str, context_id: i64, context_uuid: &str) -> Result<()> {
    let source_uuid: String = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [uuid], |r| r.get(0))?;
    conn.execute("UPDATE transcriptions SET context_id=?1 WHERE uuid=?2", params![context_id,uuid])?;
    resolve_compacted_context(conn, uuid, context_id, context_uuid, &source_uuid)?;
    conn.execute("UPDATE transcription_receipts SET context_uuid=?1 WHERE uuid=?2", params![context_uuid,uuid])?;
    conn.execute("UPDATE call_receipts SET context_uuid=?1 WHERE transcription_uuid=?2 AND context_uuid IS ?3", params![context_uuid,uuid,source_uuid])?;
    conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) SELECT ?1,source_uuid,local_uuid,source_name FROM backup_context_aliases WHERE source_uuid=?2 AND local_uuid=?3", params![uuid,source_uuid,context_uuid])?;
    conn.execute("DELETE FROM pending_transcription_contexts WHERE transcription_uuid=?1", [uuid])?;
    hold_ambiguous_retained_hour(conn, uuid, false)?;
    let calls: Vec<String> = {
        let mut stmt = conn.prepare("SELECT a.uuid FROM api_calls a JOIN transcriptions t ON t.id=a.transcription_id WHERE t.uuid=?1")?;
        let rows = stmt.query_map([uuid], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for call in calls { hold_ambiguous_retained_call(conn, &call, false)?; }
    if conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1)", [uuid], |r| r.get::<_, bool>(0))? {
        crate::sync::engine::append_self_log(conn, "transcriptions", uuid, "upsert")?;
    }
    let receipt: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND counted=1)", [uuid], |r| r.get(0))?;
    if receipt { crate::sync::engine::append_self_log(conn, "transcription_receipts", uuid, "upsert")?; }
    let calls: Vec<String> = {
        let mut stmt = conn.prepare("SELECT uuid FROM call_receipts WHERE transcription_uuid=?1 AND counted=1")?;
        let rows = stmt.query_map([uuid], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for call in calls { crate::sync::engine::append_self_log(conn, "call_receipts", &call, "upsert")?; }
    Ok(())
}

/// Move already compacted pending activity out of the temporary unscoped buckets.
fn resolve_compacted_context(conn: &Connection, uuid: &str, context_id: i64, context_uuid: &str, source_uuid: &str) -> Result<()> {
    let receipt: Option<PendingReceipt> = conn.query_row(
        "SELECT day,hour,words,spoken_words,duration_ms,counted,NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=r.uuid) FROM transcription_receipts r WHERE uuid=?1",
        [uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional()?;
    if let Some((day, hour, words, spoken, duration, counted, hourly)) = receipt {
        if counted {
            let spoken = spoken.unwrap_or(words);
            let count = i64::from(duration > 0 && spoken > 0);
            let wpm = if count > 0 { spoken as f64 * 60000.0 / duration as f64 } else { 0.0 };
            conn.execute("UPDATE transcription_context_daily_stats SET total_words=total_words-?2,total_transcriptions=total_transcriptions-1,speaking_ms=speaking_ms-?3,wpm_sum=wpm_sum-?4,wpm_count=wpm_count-?5 WHERE day=?1 AND context_uuid='unscoped'", params![day,words,duration,wpm,count])?;
            conn.execute("DELETE FROM transcription_context_daily_stats WHERE day=?1 AND context_uuid='unscoped' AND total_transcriptions<=0", [&day])?;
            conn.execute("UPDATE transcription_context_daily_stats SET best_wpm=COALESCE((SELECT MAX(wpm) FROM (SELECT CASE WHEN duration_ms>0 AND COALESCE(spoken_words,words)>0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END AS wpm FROM transcriptions t WHERE t.context_id IS NULL AND COALESCE(t.activity_day,date(t.created_at,'localtime'))=?1 AND NOT EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid=t.uuid) UNION ALL SELECT CASE WHEN duration_ms>0 AND COALESCE(spoken_words,words)>0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END FROM transcription_receipts r WHERE r.day=?1 AND r.counted=1 AND (r.context_uuid IS NULL OR EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=r.uuid AND r.uuid<>?2)))),0) WHERE day=?1 AND context_uuid='unscoped' AND NOT EXISTS(SELECT 1 FROM legacy_pruned_days WHERE day=?1)", params![day,uuid])?;
            conn.execute("INSERT INTO transcription_context_daily_stats(day,context_uuid,total_words,total_transcriptions,speaking_ms,wpm_sum,wpm_count,best_wpm) VALUES (?1,?2,?3,1,?4,?5,?6,?5) ON CONFLICT(day,context_uuid) DO UPDATE SET total_words=total_words+excluded.total_words,total_transcriptions=total_transcriptions+1,speaking_ms=speaking_ms+excluded.speaking_ms,wpm_sum=wpm_sum+excluded.wpm_sum,wpm_count=wpm_count+excluded.wpm_count,best_wpm=MAX(best_wpm,excluded.best_wpm)", params![day,context_uuid,words,duration,wpm,count])?;
            let _: (i64, i64, i64, i64) = conn.query_row("SELECT total_words,total_transcriptions,speaking_ms,wpm_count FROM transcription_context_daily_stats WHERE day=?1 AND context_uuid=?2", params![day,context_uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            if hourly {
                conn.execute("UPDATE transcription_hourly_stats SET total_words=total_words-?3 WHERE day=?1 AND hour=?2 AND context_id=0", params![day,hour,words])?;
                conn.execute("DELETE FROM transcription_hourly_stats WHERE day=?1 AND hour=?2 AND context_id=0 AND total_words<=0", params![day,hour])?;
                conn.execute("UPDATE transcription_receipts SET counted=0 WHERE uuid=?1", [uuid])?;
                let overlap = has_legacy_hour(conn, &day, hour, Some(context_id), Some(context_uuid))?;
                conn.execute("UPDATE transcription_receipts SET counted=1 WHERE uuid=?1", [uuid])?;
                if overlap {
                    conn.execute("INSERT OR IGNORE INTO backup_held_hourly_receipts(uuid) VALUES (?1)", [uuid])?;
                    log::warn!("sync: compacted activity has uncertain hourly overlap after Context resolution");
                } else {
                    conn.execute("INSERT INTO transcription_hourly_stats(day,hour,context_id,total_words) VALUES (?1,?2,?3,?4) ON CONFLICT(day,hour,context_id) DO UPDATE SET total_words=total_words+excluded.total_words", params![day,hour,context_id,words])?;
                    let _: i64 = conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE day=?1 AND hour=?2 AND context_id=?3", params![day,hour,context_id], |r| r.get(0))?;
                }
            }
        }
    }
    let calls: Vec<PendingCall> = {
        let mut stmt = conn.prepare("SELECT uuid,day,model,provider,task,audio_ms,input_chars,output_chars FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid IS ?2 AND counted=1")?;
        let rows = stmt.query_map(params![uuid,source_uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut overlaps = HashMap::new();
    for (call_uuid,day,model,provider,task,audio,input,output) in calls {
        conn.execute("UPDATE provider_daily_stats SET calls=calls-1,audio_ms=audio_ms-?5,input_chars=input_chars-?6,output_chars=output_chars-?7 WHERE day=?1 AND context_id=0 AND model=?2 AND provider=?3 AND task=?4", params![day,model,provider,task,audio,input,output])?;
        conn.execute("DELETE FROM provider_daily_stats WHERE day=?1 AND context_id=0 AND model=?2 AND provider=?3 AND task=?4 AND calls<=0", params![day,model,provider,task])?;
        let call = CompactedCall { uuid:&call_uuid, transcription_uuid:Some(uuid), day:&day, context_uuid:Some(context_uuid), context_id:Some(context_id), model:&model, provider:&provider, task:&task, audio_ms:audio, input_chars:input, output_chars:output, counted:true };
        // Check each target bucket before moving the first pending call into it.
        let key = (day.clone(), model.clone(), provider.clone(), task.clone());
        let overlap = match overlaps.get(&key) {
            Some(overlap) => *overlap,
            None => { let overlap = has_legacy_provider(conn, &call)?; overlaps.insert(key, overlap); overlap }
        };
        conn.execute("UPDATE call_receipts SET counted=0 WHERE uuid=?1", [&call_uuid])?;
        if overlap {
            log::warn!("sync: compacted provider usage has uncertain overlap after Context resolution");
        } else {
            conn.execute("UPDATE call_receipts SET counted=1 WHERE uuid=?1", [&call_uuid])?;
            conn.execute("INSERT INTO provider_daily_stats(day,context_id,model,provider,task,calls,audio_ms,input_chars,output_chars) VALUES (?1,?2,?3,?4,?5,1,?6,?7,?8) ON CONFLICT(day,context_id,model,provider,task) DO UPDATE SET calls=calls+1,audio_ms=audio_ms+excluded.audio_ms,input_chars=input_chars+excluded.input_chars,output_chars=output_chars+excluded.output_chars", params![day,context_id,model,provider,task,audio,input,output])?;
            let _: (i64, i64, i64, i64) = conn.query_row("SELECT calls,audio_ms,input_chars,output_chars FROM provider_daily_stats WHERE day=?1 AND context_id=?2 AND model=?3 AND provider=?4 AND task=?5", params![day,context_id,model,provider,task], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        }
    }
    Ok(())
}

struct RetainedCallBucket {
    context_uuid: Option<String>,
    context_id: Option<i64>,
    day: String,
    model: String,
    provider: String,
    task: String,
}

pub(crate) fn has_legacy_provider(conn: &Connection, call: &CompactedCall<'_>) -> Result<bool> {
    Ok(conn.query_row("SELECT COALESCE((SELECT calls FROM provider_daily_stats WHERE day=?1 AND context_id=?2 AND model=?3 AND provider=?4 AND task=?5),0) > (SELECT COUNT(*) FROM call_receipts r WHERE counted=1 AND day=?1 AND model=?3 AND provider=?4 AND task=?5 AND (CASE WHEN EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=r.transcription_uuid AND p.context_uuid IS r.context_uuid) THEN NULL ELSE r.context_uuid END) IS ?6)", params![call.day, call.context_id.unwrap_or(0), call.model, call.provider, call.task, call.context_uuid], |r| r.get(0))?)
}

pub(crate) fn has_legacy_hour(conn: &Connection, day: &str, hour: i64, context_id: Option<i64>, context_uuid: Option<&str>) -> Result<bool> {
    Ok(conn.query_row("SELECT COALESCE((SELECT total_words FROM transcription_hourly_stats WHERE day=?1 AND hour=?2 AND context_id=?3),0) > COALESCE((SELECT SUM(words) FROM transcription_receipts r WHERE r.day=?1 AND r.hour=?2 AND (CASE WHEN EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=r.uuid) THEN NULL ELSE r.context_uuid END) IS ?4 AND r.counted=1 AND NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=r.uuid)),0)", params![day, hour, context_id.unwrap_or(0), context_uuid], |r| r.get(0))?)
}

pub(crate) fn hold_ambiguous_retained_hour(conn: &Connection, uuid: &str, held: bool) -> Result<bool> {
    let row: Option<(String, i64, Option<i64>, Option<String>)> = conn.query_row("SELECT COALESCE(t.activity_day,date(t.created_at,'localtime')), COALESCE(t.activity_hour,CAST(strftime('%H',t.created_at,'localtime') AS INTEGER)), t.context_id, c.uuid FROM transcriptions t LEFT JOIN contexts c ON c.id=t.context_id WHERE t.uuid=?1", [uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((day, hour, context_id, context_uuid)) = row else { return Ok(false); };
    let overlap = has_legacy_hour(conn, &day, hour, context_id, context_uuid.as_deref())?;
    if held || overlap { conn.execute("INSERT OR IGNORE INTO backup_held_hourly_receipts(uuid) VALUES (?1)", [uuid])?; }
    Ok(overlap && !held)
}

pub(crate) fn hold_ambiguous_retained_call(conn: &Connection, uuid: &str, held: bool) -> Result<bool> {
    let row: Option<RetainedCallBucket> = conn.query_row(
        "SELECT c.uuid, t.context_id, COALESCE(a.activity_day,date(a.created_at,'localtime')), a.model, a.provider, a.task FROM api_calls a LEFT JOIN transcriptions t ON t.id=a.transcription_id LEFT JOIN contexts c ON c.id=t.context_id WHERE a.uuid=?1",
        [uuid], |r| Ok(RetainedCallBucket { context_uuid:r.get(0)?, context_id:r.get(1)?, day:r.get(2)?, model:r.get(3)?, provider:r.get(4)?, task:r.get(5)? }),
    ).optional()?;
    let Some(row) = row else { return Ok(false); };
    let call = CompactedCall { uuid, transcription_uuid:None, day:&row.day, context_uuid:row.context_uuid.as_deref(), context_id:row.context_id, model:&row.model, provider:&row.provider, task:&row.task, audio_ms:0, input_chars:0, output_chars:0, counted:true };
    let overlap = has_legacy_provider(conn, &call)?;
    if held || overlap { conn.execute("INSERT OR IGNORE INTO backup_held_calls(uuid) VALUES (?1)", [uuid])?; }
    Ok(overlap && !held)
}

pub(crate) fn insert_compacted_call(conn: &Connection, call: &CompactedCall<'_>) -> Result<ReceiptOutcome> {
    let existing: Option<bool> = conn.query_row("SELECT transcription_uuid IS ?2 AND day = ?3 AND context_uuid IS ?4 AND model = ?5 AND provider = ?6 AND task = ?7 AND audio_ms = ?8 AND input_chars = ?9 AND output_chars = ?10 FROM call_receipts WHERE uuid = ?1", params![call.uuid, call.transcription_uuid, call.day, call.context_uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars], |r| r.get(0)).optional()?;
    if let Some(matches) = existing {
        return Ok(if matches { ReceiptOutcome::Existing } else { ReceiptOutcome::Conflict });
    }
    conn.execute("INSERT INTO call_receipts(uuid, transcription_uuid, day, context_uuid, model, provider, task, audio_ms, input_chars, output_chars, counted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![call.uuid, call.transcription_uuid, call.day, call.context_uuid, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars, call.counted])?;
    if call.counted {
        let context_id = call.context_id.unwrap_or(0);
        conn.execute("INSERT INTO provider_daily_stats(day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8) ON CONFLICT(day,context_id,model,provider,task) DO UPDATE SET calls=calls+1, audio_ms=audio_ms+excluded.audio_ms, input_chars=input_chars+excluded.input_chars, output_chars=output_chars+excluded.output_chars", params![call.day, context_id, call.model, call.provider, call.task, call.audio_ms, call.input_chars, call.output_chars])?;
        let _: (i64, i64, i64, i64) = conn.query_row("SELECT calls, audio_ms, input_chars, output_chars FROM provider_daily_stats WHERE day=?1 AND context_id=?2 AND model=?3 AND provider=?4 AND task=?5", params![call.day, context_id, call.model, call.provider, call.task], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    }
    Ok(ReceiptOutcome::Inserted)
}
