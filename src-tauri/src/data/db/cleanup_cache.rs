use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

pub const CLEANUP_CACHE_MAX_ROWS: i64 = 2_000;
pub const CLEANUP_CACHE_MAX_BYTES: i64 = 16 * 1024 * 1024;
pub const CLEANUP_CACHE_IDLE_DAYS: i64 = 2;
pub const CLEANUP_CACHE_MAX_AGE_DAYS: i64 = 7;

pub(crate) fn shareable_cache_key(key: &str) -> bool {
    key.strip_prefix("cleanup-v3:")
        .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

pub fn cleanup_cache_result_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub(crate) fn cleanup_cache_rejection_id(key: &str, hash: &str) -> Option<String> {
    if shareable_cache_key(key) && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Some(format!("{key}:{hash}"))
    } else {
        None
    }
}

pub(crate) fn cleanup_cache_enabled_conn(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(conn.query_row("SELECT enabled FROM cleanup_cache_gate WHERE id=1", [], |r| r.get(0))?)
}

pub fn cleanup_cache_set_enabled(db: &Db, enabled: bool) -> Result<()> {
    let conn = lock_conn(db)?;
    let tx = conn.unchecked_transaction()?;
    let was_enabled = cleanup_cache_enabled_conn(&tx)?;
    tx.execute("UPDATE cleanup_cache_gate SET enabled=?1 WHERE id=1", [enabled])?;
    if !enabled {
        tx.execute("DELETE FROM cleanup_cache", [])?;
    } else if !was_enabled {
        tx.execute("UPDATE sync_peers SET needs_snapshot=1, recv_cursor=0", [])?;
    }
    tx.commit()?;
    Ok(())
}

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

pub fn cleanup_cache_context_identity(db: &Db, context_id: i64) -> Result<Option<String>> {
    let conn = lock_conn(db)?;
    conn.query_row(
        "SELECT CASE WHEN is_everywhere = 1 THEN 'everywhere' ELSE uuid END FROM contexts WHERE id = ?1",
        [context_id],
        |row| row.get(0),
    )
    .optional()
    .map(|identity| identity.flatten())
    .map_err(Into::into)
}

pub fn cleanup_cache_get_active(db: &Db, key: &str) -> Result<Option<CleanupCacheEntry>> {
    let conn = lock_conn(db)?;
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

pub fn cleanup_cache_insert_if(
    db: &Db,
    key: &str,
    clean_text: &str,
    expires_at: &str,
    is_snippet: bool,
    enabled: impl FnOnce() -> bool,
) -> Result<()> {
    let conn = lock_conn(db)?;
    if !enabled() || !cleanup_cache_enabled_conn(&conn)? {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    if let Some(id) = cleanup_cache_rejection_id(key, &cleanup_cache_result_hash(clean_text)) {
        let rejected: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_log WHERE table_name='cleanup_cache_rejection' AND row_uuid=?1)", [id], |r| r.get(0))?;
        if rejected { return Ok(()); }
    }
    tx.execute(
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
    cleanup_cache_enforce_budget_conn(&tx)?;
    if shareable_cache_key(key) && tx.query_row("SELECT EXISTS(SELECT 1 FROM cleanup_cache WHERE key=?1)", [key], |r| r.get::<_, bool>(0))? {
        crate::sync::engine::append_self_log(&tx, "cleanup_cache", key, "upsert")?;
    }
    tx.commit()?;
    Ok(())
}

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
    conn.execute("DELETE FROM sync_log WHERE table_name='cleanup_cache' AND seq NOT IN (SELECT MAX(seq) FROM sync_log WHERE table_name='cleanup_cache' GROUP BY row_uuid)", [])?;
    conn.execute("DELETE FROM sync_log WHERE table_name='cleanup_cache_rejection' AND seq NOT IN (SELECT MAX(seq) FROM sync_log WHERE table_name='cleanup_cache_rejection' GROUP BY row_uuid)", [])?;
    conn.execute("DELETE FROM sync_log WHERE table_name='cleanup_cache' AND ts_ms < (CAST(strftime('%s','now','-8 days') AS INTEGER) * 1000) AND NOT EXISTS (SELECT 1 FROM cleanup_cache WHERE key=sync_log.row_uuid)", [])?;
    conn.execute("DELETE FROM sync_log WHERE table_name='cleanup_cache_rejection' AND ts_ms < (CAST(strftime('%s','now','-8 days') AS INTEGER) * 1000)", [])?;
    Ok(changed_epoch + changed_fallback + cleanup_cache_enforce_budget_conn(&conn)?)
}

pub(crate) fn cleanup_cache_enforce_budget_conn(conn: &rusqlite::Connection) -> Result<usize> {
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

pub fn cleanup_cache_payload_bytes(db: &Db) -> Result<i64> {
    let conn = lock_conn(db)?;
    conn.query_row("SELECT COALESCE(SUM(length(CAST(key AS BLOB)) + length(CAST(clean_text AS BLOB))), 0) FROM cleanup_cache",
        [], |row| row.get(0)).map_err(Into::into)
}

pub fn cleanup_cache_delete_by_key(db: &Db, key: &str) -> Result<()> {
    let entry = cleanup_cache_get_active(db, key)?;
    if let Some(entry) = entry {
        return cleanup_cache_reject(db, key, &cleanup_cache_result_hash(&entry.clean_text));
    }
    let conn = lock_conn(db)?;
    conn.execute("DELETE FROM cleanup_cache WHERE key = ?1", params![key])?;
    Ok(())
}

pub fn cleanup_cache_reject(db: &Db, key: &str, result_hash: &str) -> Result<()> {
    let conn = lock_conn(db)?;
    let tx = conn.unchecked_transaction()?;
    if let Some(id) = cleanup_cache_rejection_id(key, result_hash) {
        crate::sync::engine::append_self_log(&tx, "cleanup_cache_rejection", &id, "delete")?;
    }
    let current: Option<String> = tx.query_row("SELECT clean_text FROM cleanup_cache WHERE key=?1", [key], |r| r.get(0)).optional()?;
    if current.as_deref().is_some_and(|text| cleanup_cache_result_hash(text) == result_hash) {
        tx.execute("DELETE FROM cleanup_cache WHERE key=?1", [key])?;
    }
    tx.commit()?;
    Ok(())
}
