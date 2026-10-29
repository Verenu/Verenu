use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};

use super::{lock_conn, Db};

// The audit intentionally stores no supplied text, setting values, or tokens.
pub fn begin_agent_change(db: &Db, command: &str, target_id: Option<i64>) -> Result<i64> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM agent_change_audit WHERE started_at < datetime('now', '-30 days')", [])?;
    tx.execute("INSERT INTO agent_change_audit(command, target_id, status) VALUES (?1, ?2, 'pending')", params![command, target_id])?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(id)
}

pub fn finish_agent_change(db: &Db, id: i64, succeeded: bool) -> Result<()> {
    let conn = lock_conn(db)?;
    conn.execute("UPDATE agent_change_audit SET status = ?2, finished_at = datetime('now') WHERE id = ?1",
        params![id, if succeeded { "succeeded" } else { "failed" }])?;
    Ok(())
}

pub fn query_agent_changes(db: &Db, limit: usize) -> Result<Value> {
    anyhow::ensure!((1..=500).contains(&limit), "limit must be between 1 and 500");
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare("SELECT id, command, target_id, status, started_at, finished_at FROM agent_change_audit WHERE started_at >= datetime('now', '-30 days') ORDER BY id DESC LIMIT ?1")?;
    let entries = stmt.query_map([limit], |row| Ok(json!({
        "id": row.get::<_, i64>(0)?, "command": row.get::<_, String>(1)?,
        "target_id": row.get::<_, Option<i64>>(2)?, "status": row.get::<_, String>(3)?,
        "started_at": row.get::<_, String>(4)?, "finished_at": row.get::<_, Option<String>>(5)?
    })))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"entries": entries, "retention_days": 30,
        "actor": "shared local session grant", "atomic_with_change": false,
        "pending_means": "The outcome is unknown; read the affected state before retrying."}))
}
