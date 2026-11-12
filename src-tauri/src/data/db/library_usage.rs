use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::{lock_conn, Db};

// This ledger is deliberately local: it contains no text and is not synced or backed up.
pub(super) fn record_library_usage_conn(conn: &Connection, kind: &str, id: i64, hits: i64) -> Result<()> {
    let table = match kind {
        "dictionary" => "dictionary",
        "snippet" => "snippets",
        _ => anyhow::bail!("Unknown library usage kind"),
    };
    if hits <= 0 {
        return Ok(());
    }
    conn.execute(
        &format!("INSERT INTO library_usage_daily(kind, item_uuid, day, hits, last_matched_at)
                  SELECT ?1, uuid, date('now'), ?3, datetime('now') FROM {table} WHERE id = ?2 AND uuid IS NOT NULL
                  ON CONFLICT(kind, item_uuid, day) DO UPDATE SET
                    hits = hits + excluded.hits, last_matched_at = excluded.last_matched_at"),
        params![kind, id, hits],
    )?;
    Ok(())
}

pub fn record_dictionary_usage(db: &Db, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    for id in ids {
        record_library_usage_conn(&tx, "dictionary", *id, 1)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn record_vocabulary_activity(db: &Db, ids: &[i64]) -> Result<()> {
    if ids.is_empty() { return Ok(()); }
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let tracking_needed: bool = tx.query_row(
        "SELECT days IS NOT NULL FROM vocabulary_expiration_settings WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    if !tracking_needed { return Ok(()); }
    for id in ids {
        let uuid: Option<String> = tx.query_row("SELECT uuid FROM dictionary WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten();
        if let Some(uuid) = uuid {
            tx.execute("INSERT INTO vocabulary_expiration_usage(item_uuid,last_used_at) VALUES (?1,datetime('now')) ON CONFLICT(item_uuid) DO UPDATE SET last_used_at=MAX(last_used_at,excluded.last_used_at)", [&uuid])?;
            crate::sync::engine::append_self_log(&tx, "vocabulary_expiration_usage", &uuid, "upsert")?;
            crate::sync::engine::reconcile_vocabulary_identities(&tx, &uuid)?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn query_library_usage(db: &Db) -> Result<Value> {
    let conn = lock_conn(db)?;
    let started_at: String = conn.query_row("SELECT started_at FROM library_usage_coverage WHERE id = 1", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT u.kind, u.item_uuid, SUM(u.hits), MAX(u.last_matched_at), COALESCE(d.id, s.id)
           FROM library_usage_daily u
           LEFT JOIN dictionary d ON u.kind = 'dictionary' AND d.uuid = u.item_uuid
           LEFT JOIN snippets s ON u.kind = 'snippet' AND s.uuid = u.item_uuid
          GROUP BY u.kind, u.item_uuid ORDER BY u.kind, u.item_uuid",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(json!({"kind": row.get::<_, String>(0)?, "uuid": row.get::<_, String>(1)?,
                  "matches": row.get::<_, i64>(2)?, "last_matched_at": row.get::<_, String>(3)?,
                  "id": row.get::<_, Option<i64>>(4)?}))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({
        "coverage": {
            "started_at": started_at,
            "scope": "this device; retained tracking window",
            "dictionary": "dictations with a mechanical substitution, counted once per term",
            "snippet": "mechanical expansions, including attempts that may not complete",
            "excludes": ["history before tracking started", "other devices", "model prompt influence", "speech recognition influence"],
            "proves_unused": false,
            "limitations": "Recording is best effort; absent matches do not prove an item was unused."
        },
        "items": rows
    }))
}
