
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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
    remote_totals_excluded: bool,
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
    let daily = stmt
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
    let contexts = stmt
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
    let mut stmt = conn.prepare("SELECT s.day, s.hour, c.uuid, s.total_words FROM transcription_hourly_stats s LEFT JOIN contexts c ON c.id = s.context_id ORDER BY s.day, s.hour, s.context_id")?;
    let hourly = stmt
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
    let providers = stmt
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
    Ok(BackupActivity {
        lifetime,
        daily,
        contexts,
        hourly,
        providers,
        remote_totals_excluded: conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_remote_stats WHERE total_words > 0 OR dictionary_fixes > 0)", [], |r| r.get(0))?,
    })
}

pub(super) fn existing_days(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT day FROM transcription_daily_stats WHERE total_transcriptions > 0")?;
    let days = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(days)
}

// A backup is a snapshot, not a set of independent events. Taking the larger
// snapshot prevents repeated imports from counting the same activity twice.
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
    let prior_lifetime: Option<(i64, i64)> = conn.query_row("SELECT total_words, dictionary_fixes FROM lifetime_stats WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let mut conflicts = usize::from(activity.remote_totals_excluded) + usize::from(!existing_days.is_empty() && prior_lifetime.is_some_and(|v| v != (life.total_words, life.dictionary_fixes)));
    conn.execute("INSERT INTO lifetime_stats (id, total_words, dictionary_fixes, wpm_sum, wpm_count) VALUES (1, ?1, ?2, ?3, ?4)
        ON CONFLICT(id) DO UPDATE SET total_words = MAX(total_words, excluded.total_words), dictionary_fixes = MAX(dictionary_fixes, excluded.dictionary_fixes),
        wpm_sum = CASE WHEN excluded.wpm_count > wpm_count THEN excluded.wpm_sum ELSE wpm_sum END,
        wpm_count = MAX(wpm_count, excluded.wpm_count)", params![life.total_words, life.dictionary_fixes, life.wpm_sum, life.wpm_count])?;
    for day in &activity.daily {
        let existing: Option<(i64, i64)> = conn.query_row("SELECT total_words, total_transcriptions FROM transcription_daily_stats WHERE day = ?1", [&day.day], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if existing.is_some_and(|value| value != (day.total_words, day.total_transcriptions)) {
            conflicts += 1;
        }
        if existing_days.contains(&day.day) {
            continue;
        }
        conn.execute("INSERT INTO transcription_daily_stats (day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(day) DO UPDATE SET total_words = excluded.total_words, total_transcriptions = excluded.total_transcriptions,
            speaking_ms = excluded.speaking_ms, wpm_sum = excluded.wpm_sum, wpm_count = excluded.wpm_count, best_wpm = excluded.best_wpm
            WHERE excluded.total_transcriptions > transcription_daily_stats.total_transcriptions", params![day.day, day.total_words, day.total_transcriptions, day.speaking_ms, day.wpm_sum, day.wpm_count, day.best_wpm])?;
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
        if existing_days.contains(&day.day) {
            continue;
        }
        conn.execute("INSERT INTO transcription_context_daily_stats (day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(day, context_uuid) DO UPDATE SET total_words = excluded.total_words, total_transcriptions = excluded.total_transcriptions,
            speaking_ms = excluded.speaking_ms, wpm_sum = excluded.wpm_sum, wpm_count = excluded.wpm_count, best_wpm = excluded.best_wpm
            WHERE excluded.total_transcriptions > transcription_context_daily_stats.total_transcriptions", params![day.day, local_uuid, day.total_words, day.total_transcriptions, day.speaking_ms, day.wpm_sum, day.wpm_count, day.best_wpm])?;
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
        let overlap: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE date(created_at, 'localtime') = ?1 AND CAST(strftime('%H', created_at, 'localtime') AS INTEGER) = ?2 AND COALESCE(context_id, 0) = ?3)", params![row.day, row.hour, id], |r| r.get(0))?;
        if overlap {
            conflicts += 1;
            continue;
        }
        let prior: Option<i64> = conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE day = ?1 AND hour = ?2 AND context_id = ?3", params![row.day, row.hour, id], |r| r.get(0)).optional()?;
        if prior.is_some_and(|value| value != row.words) { conflicts += 1; }
        conn.execute("INSERT INTO transcription_hourly_stats(day, hour, context_id, total_words) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(day, hour, context_id) DO UPDATE SET total_words = MAX(total_words, excluded.total_words)", params![row.day, row.hour, id, row.words])?;
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
        let overlap: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id WHERE date(a.created_at, 'localtime') = ?1 AND COALESCE(t.context_id, 0) = ?2 AND a.model = ?3 AND a.provider = ?4 AND a.task = ?5)", params![row.day, id, row.model, row.provider, row.task], |r| r.get(0))?;
        if overlap {
            conflicts += 1;
            continue;
        }
        let prior: Option<i64> = conn.query_row("SELECT calls FROM provider_daily_stats WHERE day = ?1 AND context_id = ?2 AND model = ?3 AND provider = ?4 AND task = ?5", params![row.day, id, row.model, row.provider, row.task], |r| r.get(0)).optional()?;
        if prior.is_some_and(|value| value != row.calls) { conflicts += 1; }
        conn.execute("INSERT INTO provider_daily_stats(day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(day, context_id, model, provider, task) DO UPDATE SET calls = excluded.calls, audio_ms = excluded.audio_ms, input_chars = excluded.input_chars, output_chars = excluded.output_chars WHERE excluded.calls > provider_daily_stats.calls", params![row.day, id, row.model, row.provider, row.task, row.calls, row.audio_ms, row.input_chars, row.output_chars])?;
    }
    Ok(conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let activity = export_activity(&source).unwrap();
        let dest = db::open(":memory:").unwrap();
        for _ in 0..2 {
            import_activity(&dest.lock().unwrap(), &activity, &[], &HashSet::new()).unwrap();
        }
        assert_eq!(export_activity(&dest).unwrap(), activity);
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
        import_activity(&dest.lock().unwrap(), &smaller, &[], &HashSet::new()).unwrap();
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
        assert_eq!(export_activity(&dest).unwrap(), activity);
    }
}
