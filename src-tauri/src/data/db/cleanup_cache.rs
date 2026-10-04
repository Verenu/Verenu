//! Cleanup-LLM response cache (keyed by prompt/input hash, with TTL).

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::*;

/// Cleanup responses are useful but disposable.  Keep their persistent
/// footprint bounded even when a user dictates for years without restarting.
pub const CLEANUP_CACHE_MAX_ROWS: i64 = 2_000;
pub const CLEANUP_CACHE_MAX_BYTES: i64 = 16 * 1024 * 1024;
pub const CLEANUP_CACHE_IDLE_DAYS: i64 = 2;
pub const CLEANUP_CACHE_MAX_AGE_DAYS: i64 = 7;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CleanupCacheEntry {
    pub key: String,
    pub clean_text: String,
    pub hit_count: i64,
    pub created_at: String,
    pub last_hit_at: String,
    pub expires_at: String,
    pub is_snippet: bool,
}

pub fn cleanup_cache_get_active(db: &Db, key: &str) -> Result<Option<CleanupCacheEntry>> {
    use rusqlite::OptionalExtension;
    let conn = lock_conn(db)?;
    // Enforce expiry during reads as well as maintenance, including databases
    // whose epoch columns have not been populated.
    conn.query_row(
        "SELECT key, clean_text, hit_count,
                COALESCE(datetime(created_at_epoch, 'unixepoch'), created_at),
                COALESCE(datetime(last_hit_at_epoch, 'unixepoch'), last_hit_at),
                COALESCE(datetime(expires_at_epoch, 'unixepoch'), expires_at),
                is_snippet
         FROM cleanup_cache WHERE key = ?1
          AND COALESCE(expires_at_epoch, CAST(strftime('%s', expires_at || 'Z') AS INTEGER))
                > CAST(strftime('%s', 'now') AS INTEGER)
          AND COALESCE(last_hit_at_epoch, CAST(strftime('%s', last_hit_at || 'Z') AS INTEGER))
                > CAST(strftime('%s', 'now', ?2) AS INTEGER)
          AND COALESCE(created_at_epoch, CAST(strftime('%s', created_at || 'Z') AS INTEGER))
                > CAST(strftime('%s', 'now', ?3) AS INTEGER)",
        params![
            key,
            format!("-{CLEANUP_CACHE_IDLE_DAYS} days"),
            format!("-{CLEANUP_CACHE_MAX_AGE_DAYS} days")
        ],
        |row| {
            Ok(CleanupCacheEntry {
                key: row.get(0)?,
                clean_text: row.get(1)?,
                hit_count: row.get(2)?,
                created_at: row.get(3)?,
                last_hit_at: row.get(4)?,
                expires_at: row.get(5)?,
                is_snippet: row.get::<_, i64>(6)? != 0,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
pub fn cleanup_cache_insert_new(
    db: &Db,
    key: &str,
    clean_text: &str,
    expires_at: &str,
    is_snippet: bool,
) -> Result<()> {
    cleanup_cache_insert_if(db, key, clean_text, expires_at, is_snippet, || true)
}

/// Recheck the live preference under the same lock as cache clearing. This
/// prevents an in-flight provider request from repopulating a disabled cache.
pub fn cleanup_cache_insert_if(
    db: &Db,
    key: &str,
    clean_text: &str,
    expires_at: &str,
    is_snippet: bool,
    enabled: impl FnOnce() -> bool,
) -> Result<()> {
    let conn = lock_conn(db)?;
    if !enabled() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR REPLACE INTO cleanup_cache
         (key, clean_text, hit_count, created_at, last_hit_at, expires_at,
          created_at_epoch, last_hit_at_epoch, expires_at_epoch, is_snippet)
         VALUES (?1, ?2, 1, datetime('now'), datetime('now'), ?3,
                 CAST(strftime('%s', 'now') AS INTEGER),
                 CAST(strftime('%s', 'now') AS INTEGER),
                 CAST(strftime('%s', ?3 || 'Z') AS INTEGER),
                 ?4)",
        params![key, clean_text, expires_at, is_snippet as i64],
    )?;
    // Insertion is the hot path where a budget violation is created, so do a
    // cheap opportunistic expiry/budget pass rather than waiting for startup.
    cleanup_cache_enforce_budget_conn(&conn)?;
    Ok(())
}

/// Applies a cache hit's touch, but only if the row still matches what the
/// caller read (`expected_created_at` / `expected_hit_count`). The rejection
/// monitor deletes a cache key when the user deletes that dictation's output;
/// a hit/touch and a delete run on different threads with no shared
/// transaction, so a stale touch could otherwise land AFTER a rejection delete
/// and a cache-miss re-insert of the same key, overwriting the freshly
/// regenerated row's `hit_count` / `expires_at` with values derived from the
/// rejected entry. Matching on the row's identity makes a stale touch a no-op
/// instead of a data-clobbering update.
pub fn cleanup_cache_touch_hit(
    db: &Db,
    key: &str,
    expected_created_at: &str,
    expected_hit_count: i64,
    new_hit_count: i64,
    last_hit_at: &str,
    expires_at: &str,
) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "UPDATE cleanup_cache
         SET hit_count = ?3,
             last_hit_at = ?4,
             expires_at = ?5,
             last_hit_at_epoch = CAST(strftime('%s', ?4 || 'Z') AS INTEGER),
             expires_at_epoch = CAST(strftime('%s', ?5 || 'Z') AS INTEGER)
         WHERE key = ?1
           AND created_at = ?2
           AND hit_count = ?6",
        params![
            key,
            expected_created_at,
            new_hit_count,
            last_hit_at,
            expires_at,
            expected_hit_count
        ],
    )?;
    if changed == 0 {
        log::warn!("cleanup cache touch skipped: row for key changed since read (stale touch)");
    }
    Ok(())
}

pub fn cleanup_cache_prune_expired(db: &Db) -> Result<usize> {
    let conn = lock_conn(db)?;
    let changed_epoch = conn.execute(
        "DELETE FROM cleanup_cache
         WHERE (expires_at_epoch IS NOT NULL
                AND expires_at_epoch <= CAST(strftime('%s', 'now') AS INTEGER))
            OR (last_hit_at_epoch IS NOT NULL
                AND last_hit_at_epoch <= CAST(strftime('%s', 'now', ?1) AS INTEGER))
            OR COALESCE(created_at_epoch, CAST(strftime('%s', created_at || 'Z') AS INTEGER))
                <= CAST(strftime('%s', 'now', ?2) AS INTEGER)",
        params![
            format!("-{CLEANUP_CACHE_IDLE_DAYS} days"),
            format!("-{CLEANUP_CACHE_MAX_AGE_DAYS} days")
        ],
    )?;
    let changed_fallback = conn.execute(
        "DELETE FROM cleanup_cache
         WHERE (expires_at_epoch IS NULL
                AND expires_at <= datetime('now'))
            OR (last_hit_at_epoch IS NULL
                AND last_hit_at <= datetime('now', ?1))",
        params![format!("-{CLEANUP_CACHE_IDLE_DAYS} days")],
    )?;
    Ok(changed_epoch + changed_fallback + cleanup_cache_enforce_budget_conn(&conn)?)
}

/// Evict least-recently-used cache responses until both row and byte budgets
/// are satisfied. Snippet entries have eviction priority but share the same
/// lifetime limits as other results.
fn cleanup_cache_enforce_budget_conn(conn: &rusqlite::Connection) -> Result<usize> {
    let (count, bytes): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(length(CAST(key AS BLOB)) +
                                  length(CAST(clean_text AS BLOB))), 0)
         FROM cleanup_cache",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let rows_to_free = (count - CLEANUP_CACHE_MAX_ROWS).max(0);
    let bytes_to_free = (bytes - CLEANUP_CACHE_MAX_BYTES).max(0);
    if rows_to_free == 0 && bytes_to_free == 0 {
        return Ok(0);
    }

    // Pick the complete LRU prefix in one statement. The old loop counted the
    // whole table and deleted one row at a time, which made a large cache
    // budget correction O(rows^2). The window sum finds the first row that
    // frees enough bytes, while the row-number floor enforces the row budget.
    let removed = conn.execute(
        "WITH ordered AS (
           SELECT rowid,
                  ROW_NUMBER() OVER (
                    ORDER BY is_snippet ASC,
                             COALESCE(last_hit_at_epoch, 0) ASC,
                             last_hit_at ASC,
                             rowid ASC
                  ) AS rn,
                  SUM(length(CAST(key AS BLOB)) + length(CAST(clean_text AS BLOB))) OVER (
                    ORDER BY is_snippet ASC,
                             COALESCE(last_hit_at_epoch, 0) ASC,
                             last_hit_at ASC,
                             rowid ASC
                    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                  ) AS cumulative_bytes
             FROM cleanup_cache
         ), cutoff AS (
           SELECT MAX(
                    ?1,
                    CASE WHEN ?2 > 0
                         THEN COALESCE((SELECT MIN(rn) FROM ordered WHERE cumulative_bytes >= ?2), 0)
                         ELSE 0 END
                  ) AS rn
         )
         DELETE FROM cleanup_cache
          WHERE rowid IN (SELECT ordered.rowid FROM ordered, cutoff WHERE ordered.rn <= cutoff.rn)",
        params![rows_to_free, bytes_to_free],
    )?;
    Ok(removed)
}

pub fn cleanup_cache_clear_all(db: &Db) -> Result<usize> {
    let conn = lock_conn(db)?;
    let changed = conn.execute("DELETE FROM cleanup_cache", [])?;
    Ok(changed)
}

pub fn cleanup_cache_count(db: &Db) -> Result<i64> {
    let conn = lock_conn(db)?;
    conn.query_row("SELECT COUNT(*) FROM cleanup_cache", [], |r| r.get(0))
        .map_err(Into::into)
}

/// Logical text payload, not SQLite file allocation or whole-disk free space.
pub fn cleanup_cache_payload_bytes(db: &Db) -> Result<i64> {
    let conn = lock_conn(db)?;
    conn.query_row("SELECT COALESCE(SUM(length(CAST(key AS BLOB)) + length(CAST(clean_text AS BLOB))), 0) FROM cleanup_cache",
        [], |row| row.get(0)).map_err(Into::into)
}

pub fn cleanup_cache_delete_by_key(db: &Db, key: &str) -> Result<()> {
    let conn = lock_conn(db)?;
    conn.execute("DELETE FROM cleanup_cache WHERE key = ?1", params![key])?;
    Ok(())
}
