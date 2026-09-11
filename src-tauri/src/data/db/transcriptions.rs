//! Transcription history queries and lifetime/derived stats.

use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecentEntry {
    pub id: i64,
    pub clean_text: String,
    pub words: i64,
    pub duration_ms: i64,
    pub app_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Stats {
    pub total_words: i64,
    pub avg_wpm: f64,
    pub day_streak: i64,
}

// One flat call site in the pipeline; bundling these into a params struct
// would add a type without removing a caller.
#[allow(clippy::too_many_arguments)]
pub fn insert_transcription_returning(
    db: &Db,
    raw: &str,
    clean: &str,
    words: i64,
    duration_ms: i64,
    api_used: &str,
    app_name: Option<&str>,
    context_id: Option<i64>,
) -> Result<RecentEntry> {
    // Compute the snippet-aware spoken count before taking the DB mutex. The
    // snippet trigger snapshot is cached by the schema layer, so this does
    // not reload complete snippet rows for every transcription.
    let spoken_words = compute_spoken_words(db, raw)?;
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let entry = tx.query_row(
        "INSERT INTO transcriptions (raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         RETURNING id, clean_text, words, duration_ms, app_name, created_at",
        params![raw, clean, words, spoken_words, duration_ms, api_used, app_name, context_id],
        |r| {
            Ok(RecentEntry {
                id: r.get(0)?,
                clean_text: r.get(1)?,
                words: r.get(2)?,
                duration_ms: r.get(3)?,
                app_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        },
    )?;
    // Lifetime counter is intentionally separate from the transcriptions
    // table so history retention pruning never shrinks it. Committed in the
    // same transaction as the insert so a crash between the two can't leave
    // total_words permanently undercounted. Upsert because a fresh database
    // (no transcriptions at migration time) may have no id=1 row yet.
    tx.execute(
        "INSERT INTO lifetime_stats (id, total_words) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET total_words = total_words + ?1",
        params![words],
    )?;
    tx.commit()?;
    Ok(entry)
}

/// Lifetime counter for dictionary substitutions actually applied to
/// dictations. Like `total_words`, it is only ever incremented — never
/// recomputed from history — so retention pruning can't shrink it. `count`
/// is the number of dictionary substitution events from one dictation.
pub fn increment_lifetime_dictionary_fixes(db: &Db, count: i64) -> Result<()> {
    if count <= 0 {
        return Ok(());
    }
    let conn = lock_conn(db)?;
    conn.execute(
        "INSERT INTO lifetime_stats (id, dictionary_fixes) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET dictionary_fixes = dictionary_fixes + ?1",
        params![count],
    )?;
    Ok(())
}

pub fn query_recent(db: &Db) -> Result<Vec<RecentEntry>> {
    let conn = lock_conn(db)?;
    // We order by id DESC instead of created_at DESC because id is the autoincrementing
    // primary key. Since IDs are monotonically increasing, this retrieves items in the
    // same chronological order but leverages the primary key index directly, avoiding
    // full table scans and manual sorting overhead in SQLite.
    let mut stmt = conn.prepare(
        "SELECT id, clean_text, words, duration_ms, app_name, created_at \
         FROM transcriptions ORDER BY id DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RecentEntry {
                id: r.get(0)?,
                clean_text: r.get(1)?,
                words: r.get(2)?,
                duration_ms: r.get(3)?,
                app_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Escapes `%`, `_`, and `\` so a user's search text is treated literally inside
/// a `LIKE ... ESCAPE '\'` pattern instead of acting as wildcards.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn fts_searchable(search: &str) -> bool {
    search
        .split_whitespace()
        .all(|term| term.chars().count() >= 3)
}

pub(super) fn history_fts_available(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1
             FROM transcription_fts_meta
            WHERE name = 'history' AND populated = 1
              AND (SELECT COUNT(*) FROM sqlite_master
                   WHERE type = 'trigger' AND name IN (
                     'trg_transcriptions_fts_ins', 'trg_transcriptions_fts_del',
                     'trg_transcriptions_fts_upd')) = 3
         )",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .unwrap_or(false)
}

fn fts_match_query(search: &str) -> String {
    search
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn query_recent_page_fts(
    conn: &rusqlite::Connection,
    limit: i64,
    offset: i64,
    before_id: Option<i64>,
    search: &str,
    app_name: Option<&str>,
) -> Result<Vec<RecentEntry>> {
    let mut sql = String::from(
        "SELECT t.id, t.clean_text, t.words, t.duration_ms, t.app_name, t.created_at
           FROM transcriptions_fts f
           JOIN transcriptions t ON t.id = f.rowid
          WHERE transcriptions_fts MATCH ?",
    );
    let mut values = vec![rusqlite::types::Value::from(fts_match_query(search))];
    if let Some(app_name) = app_name {
        sql.push_str(" AND t.app_name = ?");
        values.push(rusqlite::types::Value::from(app_name.to_string()));
    }
    if let Some(before_id) = before_id {
        sql.push_str(" AND t.id < ?");
        values.push(rusqlite::types::Value::from(before_id));
    }
    sql.push_str(" ORDER BY t.id DESC LIMIT ? OFFSET ?");
    values.push(rusqlite::types::Value::from(limit));
    values.push(rusqlite::types::Value::from(offset));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            Ok(RecentEntry {
                id: r.get(0)?,
                clean_text: r.get(1)?,
                words: r.get(2)?,
                duration_ms: r.get(3)?,
                app_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Recent transcription history, newest-first. `search` (when present) matches
/// case-insensitively against the cleaned text, the raw transcription, AND the
/// app name — each whitespace-separated term must match at least one of those
/// fields (multi-term AND), so typing an app name like "chrome" works straight
/// in the search box; `app_name` (when present) narrows to a single app.
/// Search/filtering lives in SQLite so pagination stays intact and the whole
/// table never reaches the frontend — a `LIKE` scan over short dictation text
/// is cheap even for large histories, and the `idx_transcriptions_app_name`
/// index narrows app-filtered queries before the text scan runs.
pub fn query_recent_page(
    db: &Db,
    limit: usize,
    offset: usize,
    search: Option<&str>,
    app_name: Option<&str>,
) -> Result<Vec<RecentEntry>> {
    let conn = lock_conn(db)?;
    let limit = limit.clamp(1, 500) as i64;
    let offset = offset.min(i64::MAX as usize) as i64;
    let search = search.map(str::trim).filter(|s| !s.is_empty());
    let app_name = app_name.map(str::trim).filter(|s| !s.is_empty());
    let terms: Vec<String> = search
        .map(|s| s.split_whitespace().map(escape_like).collect())
        .unwrap_or_default();
    if let Some(search) = search.filter(|s| fts_searchable(s)) {
        if history_fts_available(&conn) {
            if let Ok(rows) = query_recent_page_fts(&conn, limit, offset, None, search, app_name) {
                return Ok(rows);
            }
        }
    }

    // We order by id DESC instead of created_at DESC because id is the
    // autoincrementing primary key. Since IDs are monotonically increasing,
    // this retrieves items in the same chronological order but leverages the
    // primary key index directly, avoiding full table scans and manual sorting
    // overhead in SQLite.
    let mut sql = if app_name.is_some() {
        String::from(
            "SELECT id, clean_text, words, duration_ms, app_name, created_at \
             FROM transcriptions WHERE app_name = ?",
        )
    } else {
        String::from(
            "SELECT id, clean_text, words, duration_ms, app_name, created_at \
             FROM transcriptions WHERE 1 = 1",
        )
    };
    let mut values = Vec::<rusqlite::types::Value>::new();
    if let Some(app_name) = app_name {
        values.push(rusqlite::types::Value::from(app_name.to_string()));
    }
    for term in &terms {
        sql.push_str(
            " AND (lower(clean_text) LIKE '%' || lower(?) || '%' ESCAPE '\\' \
             OR lower(raw_text) LIKE '%' || lower(?) || '%' ESCAPE '\\' \
             OR lower(app_name) LIKE '%' || lower(?) || '%' ESCAPE '\\')",
        );
        for _ in 0..3 {
            values.push(rusqlite::types::Value::from(term.clone()));
        }
    }
    sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
    values.push(rusqlite::types::Value::from(limit));
    values.push(rusqlite::types::Value::from(offset));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            Ok(RecentEntry {
                id: r.get(0)?,
                clean_text: r.get(1)?,
                words: r.get(2)?,
                duration_ms: r.get(3)?,
                app_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Cursor-based history page. `before_id` is the last row already displayed;
/// using the AUTOINCREMENT primary key keeps newest-first pagination stable
/// when rows are inserted while the user is scrolling and avoids SQLite
/// walking and discarding a deep OFFSET.
#[allow(dead_code)]
pub fn query_recent_page_before(
    db: &Db,
    limit: usize,
    before_id: Option<i64>,
    search: Option<&str>,
    app_name: Option<&str>,
) -> Result<Vec<RecentEntry>> {
    let conn = lock_conn(db)?;
    let limit = limit.clamp(1, 500) as i64;
    let search = search.map(str::trim).filter(|s| !s.is_empty());
    let app_name = app_name.map(str::trim).filter(|s| !s.is_empty());
    let terms: Vec<String> = search
        .map(|s| s.split_whitespace().map(escape_like).collect())
        .unwrap_or_default();
    if let Some(search) = search.filter(|s| fts_searchable(s)) {
        if history_fts_available(&conn) {
            if let Ok(rows) = query_recent_page_fts(&conn, limit, 0, before_id, search, app_name) {
                return Ok(rows);
            }
        }
    }

    // Keep the filtered and unfiltered shapes separate. The former can use
    // idx_transcriptions_app_name directly; the latter can seek by rowid.
    let mut sql = if app_name.is_some() {
        String::from(
            "SELECT id, clean_text, words, duration_ms, app_name, created_at
             FROM transcriptions WHERE app_name = ?",
        )
    } else {
        String::from(
            "SELECT id, clean_text, words, duration_ms, app_name, created_at
             FROM transcriptions WHERE 1 = 1",
        )
    };
    let mut values = Vec::<rusqlite::types::Value>::new();
    if let Some(app_name) = app_name {
        values.push(rusqlite::types::Value::from(app_name.to_string()));
    }
    if let Some(before_id) = before_id {
        sql.push_str(" AND id < ?");
        values.push(rusqlite::types::Value::from(before_id));
    }
    for term in &terms {
        sql.push_str(
            " AND (lower(clean_text) LIKE '%' || lower(?) || '%' ESCAPE '\\'
             OR lower(raw_text) LIKE '%' || lower(?) || '%' ESCAPE '\\'
             OR lower(app_name) LIKE '%' || lower(?) || '%' ESCAPE '\\')",
        );
        for _ in 0..3 {
            values.push(rusqlite::types::Value::from(term.clone()));
        }
    }
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    values.push(rusqlite::types::Value::from(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            Ok(RecentEntry {
                id: r.get(0)?,
                clean_text: r.get(1)?,
                words: r.get(2)?,
                duration_ms: r.get(3)?,
                app_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Distinct apps that have dictation history, for the History app filter. Only
/// non-empty names are returned; pre-v13 rows have no app and simply appear
/// under the unfiltered view.
pub fn query_distinct_apps(db: &Db) -> Result<Vec<String>> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT app_name FROM transcriptions \
         WHERE app_name IS NOT NULL AND app_name != '' ORDER BY app_name",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn query_stats(db: &Db) -> Result<Stats> {
    let conn = lock_conn(db)?;

    // Own lifetime counter plus any counters synced from paired devices —
    // each dictation is counted once, by the device it happened on.
    // The lifetime counter and WPM aggregate are maintained by the insert and
    // delete triggers. Home therefore reads two tiny summary rows instead of
    // rescanning every retained transcription on each refresh.
    let (total_words, avg_wpm): (i64, f64) = conn.query_row(
        "SELECT
           COALESCE((SELECT total_words FROM lifetime_stats WHERE id = 1), 0)
             + COALESCE((SELECT SUM(total_words) FROM sync_remote_stats), 0),
           COALESCE((SELECT wpm_sum / NULLIF(wpm_count, 0)
                       FROM lifetime_stats WHERE id = 1), 0.0)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let day_streak: i64 = conn.query_row(
        "WITH consecutive AS (
           SELECT day AS d
           FROM transcription_daily_stats
           WHERE total_transcriptions > 0
           ORDER BY day DESC
         )
         SELECT COUNT(*) FROM (
           SELECT d,
                  ROW_NUMBER() OVER (ORDER BY d DESC) AS rn,
                  julianday(date('now','localtime')) - julianday(d) AS gap
           FROM consecutive
         )
         WHERE gap = rn - 1",
        [],
        |r| r.get(0),
    )?;

    Ok(Stats {
        total_words,
        avg_wpm,
        day_streak,
    })
}

pub fn count_transcriptions_older_than(db: &Db, max_age_days: i64) -> Result<i64> {
    let conn = lock_conn(db)?;
    let cutoff = (Utc::now().naive_utc() - Duration::days(max_age_days.max(1)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transcriptions WHERE created_at < ?1",
        params![cutoff],
        |r| r.get(0),
    )?;
    Ok(count)
}

pub fn prune_transcriptions_older_than(db: &Db, max_age_days: i64) -> Result<usize> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    // Retention removes only transcript text. Summary triggers must stay
    // quiet so daily activity, streaks, and lifetime WPM remain intact.
    tx.execute(
        "UPDATE stats_maintenance SET retention_prune = 1 WHERE rowid = 1",
        [],
    )?;
    let cutoff = (Utc::now().naive_utc() - Duration::days(max_age_days.max(1)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    tx.execute(
        "INSERT INTO transcription_hourly_stats (day, hour, context_id, total_words)
         SELECT date(created_at, 'localtime'), CAST(strftime('%H', created_at, 'localtime') AS INTEGER),
                COALESCE(context_id, 0), SUM(words)
           FROM transcriptions
          WHERE created_at < ?1
          GROUP BY 1, 2, 3
         ON CONFLICT(day, hour, context_id) DO UPDATE SET total_words = total_words + excluded.total_words",
        [&cutoff],
    )?;
    tx.execute(
        "INSERT INTO provider_daily_stats (day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars)
         SELECT date(a.created_at, 'localtime'), COALESCE(t.context_id, 0), a.model, a.provider, a.task,
                COUNT(*), COALESCE(SUM(a.audio_ms), 0), COALESCE(SUM(a.input_chars), 0), COALESCE(SUM(a.output_chars), 0)
           FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id
          WHERE a.created_at < ?1
          GROUP BY 1, 2, 3, 4, 5
         ON CONFLICT(day, context_id, model, provider, task) DO UPDATE SET
           calls = calls + excluded.calls, audio_ms = audio_ms + excluded.audio_ms,
           input_chars = input_chars + excluded.input_chars, output_chars = output_chars + excluded.output_chars",
        [&cutoff],
    )?;
    let changed = tx.execute(
        "DELETE FROM transcriptions WHERE created_at < ?1",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM api_calls
          WHERE created_at < ?1
             OR NOT EXISTS (SELECT 1 FROM transcriptions t WHERE t.id = api_calls.transcription_id)",
        params![cutoff],
    )?;
    tx.execute(
        "DELETE FROM pending_transcription_contexts
          WHERE transcription_uuid NOT IN (SELECT uuid FROM transcriptions)",
        [],
    )?;
    tx.execute(
        "UPDATE stats_maintenance SET retention_prune = 0 WHERE rowid = 1",
        [],
    )?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::{
        history_fts_available, insert_transcription_returning, query_distinct_apps,
        query_recent_page, query_recent_page_before, query_recent_page_fts, query_stats,
    };

    #[test]
    fn query_recent_page_applies_limit_and_offset() {
        let db = crate::data::db::open(":memory:").expect("db");
        for i in 0..5 {
            insert_transcription_returning(
                &db,
                &format!("raw {i}"),
                &format!("clean {i}"),
                i + 1,
                1000,
                "groq/whisper-large-v3-turbo",
                None,
                None,
            )
            .expect("insert transcription");
        }

        let first_page = query_recent_page(&db, 2, 0, None, None).expect("first page");
        assert_eq!(first_page.len(), 2);
        assert_eq!(first_page[0].clean_text, "clean 4");
        assert_eq!(first_page[1].clean_text, "clean 3");

        let second_page = query_recent_page(&db, 2, 2, None, None).expect("second page");
        assert_eq!(second_page.len(), 2);
        assert_eq!(second_page[0].clean_text, "clean 2");
        assert_eq!(second_page[1].clean_text, "clean 1");
    }

    #[test]
    fn stats_wpm_summary_tracks_edits_and_deletes() {
        let db = crate::data::db::open(":memory:").expect("db");
        let entry = insert_transcription_returning(
            &db,
            "hello world",
            "hello world",
            2,
            1_000,
            "test",
            None,
            None,
        )
        .expect("transcription");
        assert_eq!(query_stats(&db).expect("initial stats").avg_wpm, 120.0);

        {
            let conn = crate::data::db::lock_conn(&db).expect("lock");
            conn.execute(
                "UPDATE transcriptions SET duration_ms = 2_000 WHERE id = ?1",
                rusqlite::params![entry.id],
            )
            .expect("update duration");
        }
        assert_eq!(query_stats(&db).expect("updated stats").avg_wpm, 60.0);

        {
            let conn = crate::data::db::lock_conn(&db).expect("lock");
            conn.execute(
                "DELETE FROM transcriptions WHERE id = ?1",
                rusqlite::params![entry.id],
            )
            .expect("delete transcription");
        }
        assert_eq!(query_stats(&db).expect("deleted stats").avg_wpm, 0.0);
    }

    #[test]
    fn query_recent_page_filters_by_search_case_insensitive_and_partial() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw apple pie",
            "Clean Apple Pie",
            3,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert apple");
        insert_transcription_returning(
            &db,
            "raw banana",
            "Clean Banana Split",
            2,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert banana");
        insert_transcription_returning(
            &db,
            "raw raisin",
            "Clean Raisin Bread",
            2,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert raisin");

        // Partial + case-insensitive on clean_text.
        let hits = query_recent_page(&db, 50, 0, Some("apple"), None).expect("search apple");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Clean Apple Pie");

        // Lowercase query matches uppercase stored text.
        let hits =
            query_recent_page(&db, 50, 0, Some("banana split"), None).expect("search banana");
        assert_eq!(hits.len(), 1);

        // Case-insensitive on raw_text too.
        let hits = query_recent_page(&db, 50, 0, Some("RAW RAISIN"), None).expect("search raw");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Clean Raisin Bread");

        // No match.
        let hits = query_recent_page(&db, 50, 0, Some("kiwi"), None).expect("search kiwi");
        assert!(hits.is_empty());

        // Missing search returns everything.
        let hits = query_recent_page(&db, 50, 0, None, None).expect("all");
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn history_search_uses_substring_index_for_mid_word_matches() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw quarterly",
            "Quarterly planning",
            2,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert quarterly");

        // `arter` is not a token prefix. The trigram FTS index must still
        // find it, while the LIKE fallback remains available on SQLite builds
        // without FTS5.
        let hits = query_recent_page(&db, 50, 0, Some("arter"), None).expect("substring search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Quarterly planning");
    }

    #[test]
    fn history_fts_query_executes_when_the_index_is_populated() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw quarterly",
            "Quarterly planning",
            2,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert quarterly");
        let conn = crate::data::db::lock_conn(&db).expect("lock");
        if !history_fts_available(&conn) {
            return;
        }
        let hits = query_recent_page_fts(&conn, 50, 0, None, "arter", None)
            .expect("FTS query must not fall back");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn query_recent_page_before_uses_a_stable_newest_first_cursor() {
        let db = crate::data::db::open(":memory:").expect("db");
        for i in 0..5 {
            insert_transcription_returning(
                &db,
                &format!("raw {i}"),
                &format!("clean {i}"),
                i + 1,
                1000,
                "t",
                None,
                None,
            )
            .expect("insert");
        }

        let first = query_recent_page_before(&db, 2, None, None, None).expect("first page");
        assert_eq!(first.iter().map(|e| e.id).collect::<Vec<_>>(), vec![5, 4]);

        // A new row inserted after page one must not shift page two.
        insert_transcription_returning(&db, "raw new", "clean new", 1, 1000, "t", None, None)
            .expect("insert new");
        let second =
            query_recent_page_before(&db, 2, Some(first[1].id), None, None).expect("second page");
        assert_eq!(second.iter().map(|e| e.id).collect::<Vec<_>>(), vec![3, 2]);
    }

    #[test]
    fn cursor_history_keeps_app_filtering_across_pages() {
        let db = crate::data::db::open(":memory:").expect("db");
        for i in 0..6 {
            insert_transcription_returning(
                &db,
                &format!("raw {i}"),
                &format!("clean {i}"),
                1,
                1000,
                "t",
                Some(if i % 2 == 0 { "code.exe" } else { "notes.exe" }),
                None,
            )
            .expect("insert");
        }

        let first =
            query_recent_page_before(&db, 2, None, None, Some("code.exe")).expect("first page");
        assert_eq!(first.iter().map(|e| e.id).collect::<Vec<_>>(), vec![5, 3]);
        let second = query_recent_page_before(&db, 2, Some(first[1].id), None, Some("code.exe"))
            .expect("second page");
        assert_eq!(second.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1]);
        assert!(second
            .iter()
            .all(|entry| entry.app_name.as_deref() == Some("code.exe")));
    }

    #[test]
    fn history_cursor_plans_seek_and_app_filter_uses_app_index() {
        let db = crate::data::db::open(":memory:").expect("db");
        let conn = super::super::lock_conn(&db).expect("lock");

        let unfiltered: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM transcriptions
                 WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
            )
            .expect("unfiltered explain")
            .query_map([10_i64, 10_i64], |row| row.get(3))
            .expect("unfiltered plan rows")
            .collect::<rusqlite::Result<_>>()
            .expect("unfiltered plan");
        assert!(
            unfiltered
                .iter()
                .any(|detail| detail.contains("INTEGER PRIMARY KEY")),
            "expected rowid seek plan, got {unfiltered:?}"
        );

        let filtered: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM transcriptions
                 WHERE app_name = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
            )
            .expect("filtered explain")
            .query_map(["code.exe", "10", "10"], |row| row.get(3))
            .expect("filtered plan rows")
            .collect::<rusqlite::Result<_>>()
            .expect("filtered plan");
        assert!(
            filtered
                .iter()
                .any(|detail| detail.contains("idx_transcriptions_app_name")),
            "expected app-name index plan, got {filtered:?}"
        );
    }

    #[test]
    fn query_recent_page_filters_by_app_and_combines_with_search() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw a",
            "Clean A",
            1,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert outlook a");
        insert_transcription_returning(
            &db,
            "raw b",
            "Clean B",
            1,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert outlook b");
        insert_transcription_returning(
            &db,
            "raw c",
            "Clean C",
            1,
            1000,
            "t",
            Some("code.exe"),
            None,
        )
        .expect("insert code c");

        let outlook = query_recent_page(&db, 50, 0, None, Some("outlook.exe")).expect("outlook");
        assert_eq!(outlook.len(), 2);
        assert!(outlook
            .iter()
            .all(|e| e.app_name.as_deref() == Some("outlook.exe")));

        // App + search combine: only Outlook rows matching the search text.
        let combined =
            query_recent_page(&db, 50, 0, Some("clean b"), Some("outlook.exe")).expect("combined");
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].clean_text, "Clean B");

        // Unknown app matches nothing.
        let none = query_recent_page(&db, 50, 0, None, Some("slack.exe")).expect("slack");
        assert!(none.is_empty());

        // Entries round-trip their app name + duration.
        assert_eq!(outlook[0].duration_ms, 1000);
    }

    #[test]
    fn query_recent_page_treats_like_wildcards_in_search_literally() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw 100%",
            "Clean 100% Sure",
            3,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert percent");

        // A literal "%" must match its own character, not act as a wildcard.
        let hits = query_recent_page(&db, 50, 0, Some("100%"), None).expect("literal percent");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Clean 100% Sure");

        let hits = query_recent_page(&db, 50, 0, Some("100"), None).expect("numeric");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn query_recent_page_matches_app_name_from_search_box() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw a",
            "Clean A",
            1,
            1000,
            "t",
            Some("chrome.exe"),
            None,
        )
        .expect("insert chrome");
        insert_transcription_returning(
            &db,
            "raw b",
            "Clean B",
            1,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert outlook");

        // Typing an app name finds that app's dictations without the dropdown.
        let hits = query_recent_page(&db, 50, 0, Some("chrome"), None).expect("search chrome");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Clean A");

        // Case-insensitive partial matches the .exe too.
        let hits = query_recent_page(&db, 50, 0, Some("LOOK"), None).expect("search look");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Clean B");
    }

    #[test]
    fn query_recent_page_multi_term_search_requires_all_terms() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw apple pie",
            "Send the quarterly report",
            4,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert quarterly");
        insert_transcription_returning(
            &db,
            "raw banana",
            "Send a follow-up to Dan",
            5,
            1000,
            "t",
            None,
            None,
        )
        .expect("insert follow-up");
        insert_transcription_returning(
            &db,
            "raw raisin",
            "Refactor the report module",
            4,
            1000,
            "t",
            Some("code.exe"),
            None,
        )
        .expect("insert report");

        // Both terms must match (AND) across the searched fields.
        let hits = query_recent_page(&db, 50, 0, Some("send report"), None).expect("two terms");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Send the quarterly report");

        // Term order within the query doesn't matter.
        let hits = query_recent_page(&db, 50, 0, Some("REPORT send"), None).expect("reversed");
        assert_eq!(hits.len(), 1);

        // One term in text + one in the app name: both must hold.
        let hits = query_recent_page(&db, 50, 0, Some("report code"), None).expect("text+app");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].clean_text, "Refactor the report module");

        // A term matching nothing kills the whole query.
        let hits = query_recent_page(&db, 50, 0, Some("send zzz"), None).expect("no match");
        assert!(hits.is_empty());

        // Extra whitespace is ignored.
        let hits = query_recent_page(&db, 50, 0, Some("  send   report "), None).expect("padded");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn query_distinct_apps_returns_non_empty_unique_names() {
        let db = crate::data::db::open(":memory:").expect("db");
        insert_transcription_returning(
            &db,
            "raw a",
            "Clean A",
            1,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert outlook");
        insert_transcription_returning(
            &db,
            "raw b",
            "Clean B",
            1,
            1000,
            "t",
            Some("outlook.exe"),
            None,
        )
        .expect("insert outlook again");
        insert_transcription_returning(
            &db,
            "raw c",
            "Clean C",
            1,
            1000,
            "t",
            Some("code.exe"),
            None,
        )
        .expect("insert code");
        insert_transcription_returning(&db, "raw d", "Clean D", 1, 1000, "t", None, None)
            .expect("insert no app");

        let apps = query_distinct_apps(&db).expect("distinct apps");
        assert_eq!(
            apps,
            vec!["code.exe".to_string(), "outlook.exe".to_string()]
        );
    }
}
