//! Supported, bounded historical reads. No arbitrary SQL or duplicated text storage.
use super::{lock_conn, Db};
use anyhow::{bail, Result};
use chrono::NaiveDate;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRange {
    pub start: String,
    pub end: String,
}

impl HistoryRange {
    pub fn validate(&self) -> Result<()> {
        let parse = |date: &str| -> Result<NaiveDate> {
            let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
            if parsed.to_string() != date {
                bail!("Use YYYY-MM-DD UTC dates");
            }
            Ok(parsed)
        };
        if parse(&self.start)? >= parse(&self.end)? {
            bail!("end must be after start (exclusive)");
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPage {
    #[serde(flatten)]
    pub range: HistoryRange,
    #[serde(default)]
    pub after_id: i64,
    #[serde(default = "page_size")]
    pub limit: u32,
    #[serde(default)]
    pub include_text: bool,
}
fn page_size() -> u32 {
    100
}

#[derive(Serialize)]
struct HistoryRow {
    id: i64,
    created_at: String,
    context_id: Option<i64>,
    app_name: Option<String>,
    api_used: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clean_text: Option<String>,
}

pub fn agent_history_page(db: &Db, query: HistoryPage) -> Result<Value> {
    query.range.validate()?;
    if !(1..=500).contains(&query.limit) || query.after_id < 0 {
        bail!("limit must be 1..500; after_id must be nonnegative");
    }
    let conn = lock_conn(db)?;
    // Text columns are not selected at all without explicit permission.
    let columns = if query.include_text {
        "raw_text, clean_text"
    } else {
        "NULL, NULL"
    };
    let mut stmt = conn.prepare(&format!("SELECT id, created_at, context_id, app_name, api_used, {columns}
        FROM transcriptions WHERE created_at >= ?1 AND created_at < ?2 AND id > ?3 ORDER BY id LIMIT ?4"))?;
    let rows = stmt
        .query_map(
            params![
                query.range.start,
                query.range.end,
                query.after_id,
                query.limit + 1
            ],
            |r| {
                Ok(HistoryRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    context_id: r.get(2)?,
                    app_name: r.get(3)?,
                    api_used: r.get(4)?,
                    raw_text: r.get(5)?,
                    clean_text: r.get(6)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = rows.len() > query.limit as usize;
    let rows = rows
        .into_iter()
        .take(query.limit as usize)
        .collect::<Vec<_>>();
    Ok(json!({"next_after_id": rows.last().map(|r| r.id), "has_more": has_more, "rows": rows}))
}

pub fn agent_history_report(db: &Db, range: HistoryRange) -> Result<Value> {
    range.validate()?;
    let conn = lock_conn(db)?;
    let (count, oldest, newest): (i64, Option<String>, Option<String>) = conn.query_row(
        "SELECT count(*), min(created_at), max(created_at) FROM transcriptions WHERE created_at >= ?1 AND created_at < ?2",
        params![range.start, range.end], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut stmt = conn.prepare("SELECT raw_text, clean_text FROM transcriptions WHERE created_at >= ?1 AND created_at < ?2 ORDER BY created_at LIMIT 100000")?;
    let mut rows = stmt.query(params![range.start, range.end])?;
    let mut signals = BTreeMap::<&str, u64>::new();
    while let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let clean: String = row.get(1)?;
        let signal = if raw == clean {
            "unchanged"
        } else if raw.to_lowercase() == clean.to_lowercase() {
            "capitalization_only_change"
        } else if raw.split_whitespace().eq(clean.split_whitespace()) {
            "spacing_only_change"
        } else {
            "other_cleanup_change"
        };
        *signals.entry(signal).or_default() += 1;
    }
    let mut apps = conn.prepare("SELECT app_name, context_id, count(*) FROM transcriptions WHERE created_at >= ?1 AND created_at < ?2 GROUP BY app_name, context_id ORDER BY count(*) DESC, app_name, context_id LIMIT 10")?;
    let apps = apps.query_map(params![range.start, range.end], |r| Ok(json!({"app_name": r.get::<_, Option<String>>(0)?, "context_id": r.get::<_, Option<i64>>(1)?, "count": r.get::<_, i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"retained_count": count, "oldest": oldest, "newest": newest, "signals": signals,
        "scanned": count.min(100000), "truncated": count > 100000, "top_app_contexts": apps,
        "limits": "Changes are investigation signals, not confirmed mistakes. Deleted history, original prompts, settings snapshots, audio and final user corrections cannot be reconstructed."}),
    )
}

pub fn diagnostic_retention(db: &Db, days: Option<u32>) -> Result<u32> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    if let Some(days) = days {
        if days > 365 {
            bail!("retention_days must be 0..365; 0 disables and clears the journal");
        }
        tx.execute(
            "UPDATE diagnostic_policy SET retention_days = ?1 WHERE id = 1",
            [days],
        )?;
    }
    let days: u32 = tx.query_row(
        "SELECT retention_days FROM diagnostic_policy WHERE id = 1",
        [],
        |r| r.get(0),
    )?;
    if days == 0 {
        tx.execute("DELETE FROM diagnostic_daily_failures", [])?;
    } else {
        // The journal stores UTC date buckets. Count today as one of the
        // requested retention days so the table never keeps an extra bucket.
        let cutoff_days = days - 1;
        tx.execute(
            "DELETE FROM diagnostic_daily_failures WHERE day < date('now', ?1)",
            [format!("-{cutoff_days} days")],
        )?;
    }
    tx.commit()?;
    Ok(days)
}

pub fn record_diagnostic_failure(db: &Db, subsystem: &str, fingerprint: &str) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO diagnostic_daily_failures(day, subsystem, fingerprint)
        SELECT date('now'), ?1, ?2 WHERE (SELECT retention_days FROM diagnostic_policy WHERE id=1) > 0
        ON CONFLICT(day, subsystem, fingerprint) DO UPDATE SET count = count + 1", params![subsystem, fingerprint])?;
    tx.execute("DELETE FROM diagnostic_daily_failures WHERE rowid IN
        (SELECT rowid FROM diagnostic_daily_failures ORDER BY day DESC, rowid DESC LIMIT -1 OFFSET 50000)", [])?;
    tx.commit()?;
    Ok(())
}

pub fn diagnostic_top(db: &Db, range: HistoryRange) -> Result<Value> {
    range.validate()?;
    let days = diagnostic_retention(db, None)?;
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare(
        "SELECT subsystem, fingerprint, sum(count), min(day), max(day)
        FROM diagnostic_daily_failures WHERE day >= ?1 AND day < ?2 GROUP BY subsystem, fingerprint
        ORDER BY sum(count) DESC, subsystem, fingerprint LIMIT 10",
    )?;
    let groups = stmt.query_map(params![range.start, range.end], |r| Ok(json!({
        "subsystem": r.get::<_, String>(0)?, "fingerprint": r.get::<_, String>(1)?, "count": r.get::<_, i64>(2)?,
        "first_day": r.get::<_, String>(3)?, "last_day": r.get::<_, String>(4)?
    })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let oldest: Option<String> =
        conn.query_row("SELECT min(day) FROM diagnostic_daily_failures", [], |r| {
            r.get(0)
        })?;
    Ok(
        json!({"retention_days": days, "oldest_retained_day": oldest, "top_ten": groups,
        "limits": "Opt-in metadata only; no historical backfill. Counts are recorded failure events, not unique dictations. Maximum 50000 daily groups; oldest groups may be evicted. No messages, prompts or transcripts are stored in this journal."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn range() -> HistoryRange {
        HistoryRange {
            start: "2000-01-01".into(),
            end: "2100-01-01".into(),
        }
    }
    #[test]
    fn agent_history_json_boundary_rejects_unknowns_and_defaults_to_no_text() {
        let query: HistoryPage =
            serde_json::from_value(json!({"start":"2000-01-01", "end":"2100-01-01", "limit":1}))
                .unwrap();
        assert!(!query.include_text);
        assert_eq!(query.after_id, 0);
        assert!(serde_json::from_value::<HistoryPage>(
            json!({"start":"2000-01-01", "end":"2100-01-01", "sql":"SELECT 1"})
        )
        .is_err());
    }
    #[test]
    fn agent_history_is_explicit_paginated_and_does_not_call_changes_errors() {
        let db = super::super::open(":memory:").unwrap();
        for (raw, clean) in [("hello", "Hello"), ("a  b", "a b"), ("hi", "good morning")] {
            super::super::insert_transcription_returning(
                &db,
                raw,
                clean,
                1,
                1000,
                "synthetic",
                None,
                None,
            )
            .unwrap();
        }
        let report = agent_history_report(&db, range()).unwrap();
        assert_eq!(report["signals"]["capitalization_only_change"], 1);
        assert_eq!(report["signals"]["spacing_only_change"], 1);
        let page = agent_history_page(
            &db,
            HistoryPage {
                range: range(),
                after_id: 0,
                limit: 1,
                include_text: false,
            },
        )
        .unwrap();
        assert_eq!(page["has_more"], true);
        assert!(page["rows"][0].get("raw_text").is_none());
        let next = agent_history_page(
            &db,
            HistoryPage {
                range: range(),
                after_id: page["next_after_id"].as_i64().unwrap(),
                limit: 10,
                include_text: true,
            },
        )
        .unwrap();
        assert_eq!(next["rows"].as_array().unwrap().len(), 2);
        assert_eq!(next["rows"][0]["raw_text"], "a  b");
        assert_eq!(
            agent_history_report(
                &db,
                HistoryRange {
                    start: "2100-01-01".into(),
                    end: "2100-02-01".into()
                }
            )
            .unwrap()["retained_count"],
            0
        );
        assert!(agent_history_report(
            &db,
            HistoryRange {
                start: "bad".into(),
                end: "bad".into()
            }
        )
        .is_err());
    }
    #[test]
    fn diagnostic_journal_is_opt_in_and_disable_clears_without_touching_history() {
        let db = super::super::open(":memory:").unwrap();
        super::super::insert_transcription_returning(
            &db,
            "public synthetic",
            "Public synthetic",
            1,
            1000,
            "synthetic",
            None,
            None,
        )
        .unwrap();
        record_diagnostic_failure(&db, "cleanup", "hash").unwrap();
        assert_eq!(diagnostic_top(&db, range()).unwrap()["top_ten"], json!([]));
        diagnostic_retention(&db, Some(90)).unwrap();
        record_diagnostic_failure(&db, "cleanup", "hash").unwrap();
        record_diagnostic_failure(&db, "cleanup", "hash").unwrap();
        assert_eq!(
            diagnostic_top(&db, range()).unwrap()["top_ten"][0]["count"],
            2
        );
        diagnostic_retention(&db, Some(0)).unwrap();
        assert_eq!(diagnostic_top(&db, range()).unwrap()["top_ten"], json!([]));
        assert!(diagnostic_retention(&db, Some(366)).is_err());
        assert_eq!(
            agent_history_report(&db, range()).unwrap()["retained_count"],
            1
        );
    }

    #[test]
    fn diagnostic_retention_keeps_only_the_requested_number_of_utc_days() {
        let db = super::super::open(":memory:").unwrap();
        lock_conn(&db)
            .unwrap()
            .execute(
                "INSERT INTO diagnostic_daily_failures(day, subsystem, fingerprint)
                 VALUES(date('now'), 'cleanup', 'today'),
                       (date('now', '-1 day'), 'cleanup', 'yesterday')",
                [],
            )
            .unwrap();

        assert_eq!(diagnostic_retention(&db, Some(1)).unwrap(), 1);
        let report = diagnostic_top(&db, range()).unwrap();
        assert_eq!(report["top_ten"].as_array().unwrap().len(), 1);
        assert_eq!(report["top_ten"][0]["fingerprint"], "today");
    }

    #[test]
    fn agent_history_reopen_preserves_history_policy_and_prunes_old_events() {
        let path =
            std::env::temp_dir().join(format!("verenu-agent-test-{}.db", uuid::Uuid::new_v4()));
        {
            let db = super::super::open(path.to_str().unwrap()).unwrap();
            super::super::insert_transcription_returning(
                &db,
                "public synthetic",
                "Public synthetic",
                1,
                1000,
                "synthetic",
                None,
                None,
            )
            .unwrap();
            diagnostic_retention(&db, Some(90)).unwrap();
            record_diagnostic_failure(&db, "cleanup", "current").unwrap();
            lock_conn(&db).unwrap().execute("INSERT INTO diagnostic_daily_failures(day,subsystem,fingerprint) VALUES('2000-01-01','cleanup','expired')", []).unwrap();
        }
        let db = super::super::open(path.to_str().unwrap()).unwrap();
        assert_eq!(
            agent_history_report(&db, range()).unwrap()["retained_count"],
            1
        );
        let report = diagnostic_top(&db, range()).unwrap();
        assert_eq!(report["retention_days"], 90);
        assert_eq!(report["top_ten"].as_array().unwrap().len(), 1);
        assert_eq!(report["top_ten"][0]["fingerprint"], "current");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
