use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::protocol::SyncOp;

pub const PEER_STALE_AFTER_DAYS: i64 = 30;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncPeer {
    pub device_uuid: String,
    pub name: String,
    pub cert_fp: String,
    pub added_at: String,
    pub last_sync_at: Option<String>,
    pub send_cursor: i64,
    pub needs_snapshot: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceStats {
    pub device_id: String,
    pub total_words: i64,
    pub dictionary_fixes: i64,
}

pub fn set_sync_applying(conn: &Connection, applying: bool) -> Result<()> {
    conn.execute(
        "UPDATE sync_state SET applying = ?1",
        params![if applying { 1 } else { 0 }],
    )?;
    Ok(())
}

pub fn self_uuid(conn: &Connection) -> Result<Option<String>> {
    let uuid: Option<String> = conn
        .query_row("SELECT uuid FROM sync_identity LIMIT 1", [], |r| r.get(0))
        .optional()?;
    Ok(uuid)
}

pub fn ensure_self_identity(conn: &Connection, uuid: &str, name: &str) -> Result<()> {
    let existing = self_uuid(conn)?;
    match existing {
        Some(existing) if existing == uuid => {
            conn.execute("UPDATE sync_identity SET name = ?1", params![name])?;
        }
        _ => {
            conn.execute("DELETE FROM sync_identity", [])?;
            conn.execute(
                "INSERT INTO sync_identity (uuid, name) VALUES (?1, ?2)",
                params![uuid, name],
            )?;
        }
    }
    Ok(())
}

pub fn list_peers(conn: &Connection) -> Result<Vec<SyncPeer>> {
    let mut stmt = conn.prepare(
        "SELECT device_uuid, name, cert_fp, added_at, last_sync_at, send_cursor, needs_snapshot, last_error
         FROM sync_peers ORDER BY added_at ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SyncPeer {
                device_uuid: r.get(0)?,
                name: r.get(1)?,
                cert_fp: r.get(2)?,
                added_at: r.get(3)?,
                last_sync_at: r.get(4)?,
                send_cursor: r.get(5)?,
                needs_snapshot: r.get::<_, i64>(6)? != 0,
                last_error: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_peer(conn: &Connection, device_uuid: &str) -> Result<Option<SyncPeer>> {
    let mut stmt = conn.prepare(
        "SELECT device_uuid, name, cert_fp, added_at, last_sync_at, send_cursor, needs_snapshot, last_error
         FROM sync_peers WHERE device_uuid = ?1",
    )?;
    let row = stmt
        .query_row(params![device_uuid], |r| {
            Ok(SyncPeer {
                device_uuid: r.get(0)?,
                name: r.get(1)?,
                cert_fp: r.get(2)?,
                added_at: r.get(3)?,
                last_sync_at: r.get(4)?,
                send_cursor: r.get(5)?,
                needs_snapshot: r.get::<_, i64>(6)? != 0,
                last_error: r.get(7)?,
            })
        })
        .optional()?;
    Ok(row)
}

pub fn upsert_peer(conn: &Connection, device_uuid: &str, name: &str, cert_fp: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_peers (device_uuid, name, cert_fp, needs_snapshot)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT(device_uuid) DO UPDATE SET
           name = excluded.name,
           cert_fp = excluded.cert_fp,
           needs_snapshot = 1,
           send_cursor = 0,
           recv_cursor = 0,
           last_error = NULL",
        params![device_uuid, name, cert_fp],
    )?;
    Ok(())
}

pub fn remove_peer(conn: &Connection, device_uuid: &str) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM sync_peers WHERE device_uuid = ?1",
        params![device_uuid],
    )?;
    Ok(n > 0)
}

pub fn mark_peer_synced(conn: &Connection, device_uuid: &str, send_cursor: i64) -> Result<()> {
    conn.execute(
        "UPDATE sync_peers
         SET last_sync_at = datetime('now'), send_cursor = MAX(send_cursor, ?2), last_error = NULL
         WHERE device_uuid = ?1",
        params![device_uuid, send_cursor],
    )?;
    Ok(())
}

pub fn mark_peer_error(conn: &Connection, device_uuid: &str, error: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_peers SET last_error = ?2 WHERE device_uuid = ?1",
        params![device_uuid, truncate_error(error)],
    )?;
    Ok(())
}

fn truncate_error(error: &str) -> String {
    error.chars().take(300).collect()
}

pub fn peer_send_position(conn: &Connection, device_uuid: &str) -> Result<(i64, bool)> {
    let row = conn
        .query_row(
            "SELECT send_cursor, needs_snapshot FROM sync_peers WHERE device_uuid = ?1",
            params![device_uuid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or((0, 1));
    Ok((row.0, row.1 != 0))
}

pub fn needs_snapshot_for(conn: &Connection, device_uuid: &str, since_seq: i64) -> Result<bool> {
    let (_, peer_flag) = peer_send_position(conn, device_uuid)?;
    if peer_flag {
        return Ok(true);
    }
    match oldest_log_seq(conn)? {
        Some(oldest) => Ok(since_seq < oldest.saturating_sub(1)),
        None => Ok(false),
    }
}

pub fn set_peer_send_position(
    conn: &Connection,
    device_uuid: &str,
    send_cursor: i64,
    needs_snapshot: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE sync_peers SET send_cursor = ?2, needs_snapshot = ?3 WHERE device_uuid = ?1",
        params![device_uuid, send_cursor, if needs_snapshot { 1 } else { 0 }],
    )?;
    Ok(())
}

pub fn peer_recv_cursor(conn: &Connection, device_uuid: &str) -> Result<i64> {
    let cursor: i64 = conn
        .query_row(
            "SELECT recv_cursor FROM sync_peers WHERE device_uuid = ?1",
            params![device_uuid],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(cursor)
}

pub fn set_peer_recv_cursor(conn: &Connection, device_uuid: &str, cursor: i64) -> Result<()> {
    conn.execute(
        "UPDATE sync_peers SET recv_cursor = MAX(recv_cursor, ?2) WHERE device_uuid = ?1",
        params![device_uuid, cursor],
    )?;
    Ok(())
}

pub fn maintain_peer_lifecycle(conn: &Connection) -> Result<(usize, usize)> {
    let marked = conn.execute(
        "UPDATE sync_peers
         SET needs_snapshot = 1,
             send_cursor = 0,
             last_error = 'peer stale; full resnapshot required'
         WHERE COALESCE(last_sync_at, added_at) < datetime('now', ?1)
           AND needs_snapshot = 0",
        params![format!("-{} days", PEER_STALE_AFTER_DAYS)],
    )?;
    Ok((marked, 0))
}

fn oldest_log_seq(conn: &Connection) -> Result<Option<i64>> {
    let seq: Option<i64> = conn
        .query_row("SELECT MIN(seq) FROM sync_log", [], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(seq)
}

pub fn latest_op_stamp(
    conn: &Connection,
    table_name: &str,
    row_uuid: &str,
) -> Result<Option<(i64, String, i64)>> {
    let row = conn
        .prepare_cached(
            "SELECT ts_ms, origin, origin_seq FROM sync_log
             WHERE table_name = ?1 AND row_uuid = ?2
             ORDER BY seq DESC LIMIT 1",
        )?
        .query_row(params![table_name, row_uuid], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    Ok(row)
}

pub fn append_op(
    conn: &Connection,
    op: &SyncOp,
    ts_ms: i64,
    origin: &str,
    origin_seq: i64,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?
    .execute(
        params![
            op.table_name(),
            op.row_uuid,
            op.op_name(),
            ts_ms,
            origin,
            origin_seq
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    #[allow(dead_code)]
    pub seq: i64,
    pub table_name: String,
    pub row_uuid: String,
    pub op: String,
    pub ts_ms: i64,
    pub origin: String,
    pub origin_seq: i64,
}

pub fn changes_since(conn: &Connection, after_seq: i64, limit: i64) -> Result<Vec<LogEntry>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, table_name, row_uuid, op, ts_ms, origin, origin_seq
         FROM (SELECT seq, table_name, row_uuid, op, ts_ms, origin, origin_seq,
                      MAX(seq) OVER (PARTITION BY table_name, row_uuid) AS max_seq
               FROM sync_log WHERE seq > ?1)
         WHERE seq = max_seq
         ORDER BY seq ASC
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![after_seq, limit], |r| {
            Ok(LogEntry {
                seq: r.get(0)?,
                table_name: r.get(1)?,
                row_uuid: r.get(2)?,
                op: r.get(3)?,
                ts_ms: r.get(4)?,
                origin: r.get(5)?,
                origin_seq: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn max_log_seq(conn: &Connection) -> Result<i64> {
    let seq: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM sync_log", [], |r| {
        r.get(0)
    })?;
    Ok(seq)
}

pub fn compact_log(conn: &Connection) -> Result<usize> {
    let _ = maintain_peer_lifecycle(conn)?;
    let min_cursor: Option<i64> = conn
        .query_row(
            "SELECT MIN(send_cursor) FROM sync_peers WHERE needs_snapshot = 0",
            [],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let n = if let Some(min_cursor) = min_cursor {
        conn.execute(
            "DELETE FROM sync_log
             WHERE seq <= ?1
               AND seq NOT IN (SELECT MAX(seq) FROM sync_log GROUP BY table_name, row_uuid)",
            params![min_cursor],
        )?
    } else {
        conn.execute(
            "DELETE FROM sync_log
             WHERE seq NOT IN (SELECT MAX(seq) FROM sync_log GROUP BY table_name, row_uuid)",
            [],
        )?
    };

    Ok(n)
}

pub fn list_remote_stats(conn: &Connection) -> Result<Vec<DeviceStats>> {
    let mut stmt =
        conn.prepare("SELECT device_id, total_words, dictionary_fixes FROM combined_device_counters")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DeviceStats {
                device_id: r.get(0)?,
                total_words: r.get(1)?,
                dictionary_fixes: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn upsert_remote_stats(conn: &Connection, stats: &DeviceStats) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_remote_stats (device_id, total_words, dictionary_fixes, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(device_id) DO UPDATE SET
           total_words = MAX(total_words, excluded.total_words),
           dictionary_fixes = MAX(dictionary_fixes, excluded.dictionary_fixes),
           updated_at = excluded.updated_at",
        params![stats.device_id, stats.total_words, stats.dictionary_fixes],
    )?;
    Ok(())
}

pub fn remove_remote_stats(conn: &Connection, device_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM sync_remote_stats WHERE device_id = ?1",
        params![device_id],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn effective_lifetime_totals(conn: &Connection) -> Result<(i64, i64)> {
    let row: (i64, i64) = conn.query_row(
        "SELECT COALESCE((SELECT total_words FROM lifetime_stats WHERE id = 1), 0)
              + COALESCE((SELECT SUM(total_words) FROM combined_device_counters), 0),
                COALESCE((SELECT dictionary_fixes FROM lifetime_stats WHERE id = 1), 0)
              + COALESCE((SELECT SUM(dictionary_fixes) FROM combined_device_counters), 0)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct SettingStamp {
    pub ts_ms: i64,
    pub origin: String,
}

pub fn list_setting_stamps(conn: &Connection) -> Result<Vec<(String, SettingStamp)>> {
    let mut stmt = conn.prepare("SELECT key, ts_ms, origin FROM sync_setting_meta")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                SettingStamp {
                    ts_ms: r.get(1)?,
                    origin: r.get(2)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_setting_stamp(conn: &Connection, key: &str) -> Result<Option<SettingStamp>> {
    let row = conn
        .query_row(
            "SELECT ts_ms, origin FROM sync_setting_meta WHERE key = ?1",
            params![key],
            |r| {
                Ok(SettingStamp {
                    ts_ms: r.get(0)?,
                    origin: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn set_setting_stamp(conn: &Connection, key: &str, ts_ms: i64, origin: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_setting_meta (key, ts_ms, origin) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET ts_ms = excluded.ts_ms, origin = excluded.origin",
        params![key, ts_ms, origin],
    )?;
    Ok(())
}

pub fn seed_setting_stamps(conn: &Connection, origin: &str, keys: &[String]) -> Result<()> {
    let now_ms = now_ms();
    for key in keys {
        conn.execute(
            "INSERT OR IGNORE INTO sync_setting_meta (key, ts_ms, origin) VALUES (?1, ?2, ?3)",
            params![key, now_ms, origin],
        )?;
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
