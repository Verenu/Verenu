use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{delete_dictionary_entry_conn, delete_snippet_conn, invalidate_snippet_cache, lock_conn, Db};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct LibraryItem {
    pub kind: String,
    pub uuid: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Assignment { uuid: String, name: String, everywhere: bool }

#[derive(Debug, Serialize, Deserialize)]
struct Correction {
    uuid: String, context_uuid: String, mistake: String, auto_learned: i64,
    correction_count: i64, confidence_tier: String, last_seen_at: Option<String>, created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    kind: String, uuid: String, id: i64, name: String,
    mistake: Option<String>, auto_learned: i64, correction_count: i64,
    confidence_tier: Option<String>, last_seen_at: Option<String>,
    expansion: Option<String>, instructions: Option<String>, use_count: i64,
    created_at: String, assignments: Vec<Assignment>, corrections: Vec<Correction>,
    #[serde(default)]
    local_matches: i64,
    #[serde(default)]
    local_last_matched_at: Option<String>,
}

fn normalize_items(mut items: Vec<LibraryItem>) -> Result<Vec<LibraryItem>> {
    ensure!(!items.is_empty() && items.len() <= 100, "Select between 1 and 100 items");
    for item in &mut items {
        ensure!(matches!(item.kind.as_str(), "dictionary" | "snippet"), "Unknown library item kind");
        item.uuid = uuid::Uuid::parse_str(&item.uuid).context("Invalid item UUID")?.to_string();
    }
    items.sort();
    ensure!(items.windows(2).all(|pair| pair[0] != pair[1]), "Duplicate library item");
    Ok(items)
}

fn read_entries(conn: &Connection, items: &[LibraryItem]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in items {
        let mut entry = match item.kind.as_str() {
            "dictionary" => conn.query_row(
                "SELECT id,uuid,term,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at FROM dictionary WHERE uuid=?1",
                [&item.uuid], |r| Ok(Entry { kind: item.kind.clone(), id:r.get(0)?, uuid:r.get(1)?, name:r.get(2)?, mistake:r.get(3)?, auto_learned:r.get(4)?, correction_count:r.get(5)?, confidence_tier:Some(r.get(6)?), last_seen_at:r.get(7)?, created_at:r.get(8)?, expansion:None, instructions:None, use_count:0, assignments:vec![], corrections:vec![], local_matches:0, local_last_matched_at:None }),
            ).optional()?,
            "snippet" => conn.query_row(
                "SELECT id,uuid,trigger,expansion,instructions,use_count,created_at FROM snippets WHERE uuid=?1",
                [&item.uuid], |r| Ok(Entry { kind:item.kind.clone(), id:r.get(0)?, uuid:r.get(1)?, name:r.get(2)?, expansion:Some(r.get(3)?), instructions:Some(r.get(4)?), use_count:r.get(5)?, created_at:r.get(6)?, mistake:None, auto_learned:0, correction_count:0, confidence_tier:None, last_seen_at:None, assignments:vec![], corrections:vec![], local_matches:0, local_last_matched_at:None }),
            ).optional()?,
            _ => unreachable!(),
        }.context("A selected item no longer exists")?;
        (entry.local_matches, entry.local_last_matched_at) = conn.query_row(
            "SELECT COALESCE(SUM(hits), 0), MAX(last_matched_at) FROM library_usage_daily WHERE kind=?1 AND item_uuid=?2",
            params![item.kind, item.uuid], |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (table, column) = if item.kind == "dictionary" { ("dictionary_contexts", "dictionary_id") } else { ("snippet_contexts", "snippet_id") };
        entry.assignments = conn.prepare(&format!("SELECT c.uuid,c.name,c.is_everywhere FROM contexts c JOIN {table} a ON a.context_id=c.id WHERE a.{column}=?1 ORDER BY c.uuid"))?
            .query_map([entry.id], |r| Ok(Assignment { uuid:r.get(0)?, name:r.get(1)?, everywhere:r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        if item.kind == "dictionary" {
            entry.corrections = conn.prepare("SELECT dc.uuid,c.uuid,dc.mistake,dc.auto_learned,dc.correction_count,dc.confidence_tier,dc.last_seen_at,dc.created_at FROM dictionary_corrections dc JOIN contexts c ON c.id=dc.context_id WHERE dc.dictionary_id=?1 ORDER BY dc.uuid")?
                .query_map([entry.id], |r| Ok(Correction { uuid:r.get(0)?, context_uuid:r.get(1)?, mistake:r.get(2)?, auto_learned:r.get(3)?, correction_count:r.get(4)?, confidence_tier:r.get(5)?, last_seen_at:r.get(6)?, created_at:r.get(7)? }))?
                .collect::<rusqlite::Result<_>>()?;
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn basis(entries: &[Entry]) -> Result<String> {
    // Counters are intentionally included: any changed state invalidates a preview.
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(entries)?)))
}

pub fn list_library_items(db: &Db) -> Result<Value> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare("SELECT 'dictionary',id,uuid,term FROM dictionary UNION ALL SELECT 'snippet',id,uuid,trigger FROM snippets ORDER BY 1,4")?;
    let items = stmt.query_map([], |r| Ok(json!({"kind":r.get::<_,String>(0)?,"id":r.get::<_,i64>(1)?,"uuid":r.get::<_,String>(2)?,"name":r.get::<_,String>(3)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items}))
}

pub fn preview_library_delete(db: &Db, items: Vec<LibraryItem>) -> Result<Value> {
    let items = normalize_items(items)?;
    let conn = lock_conn(db)?;
    let entries = read_entries(&conn, &items)?;
    Ok(json!({"basis": basis(&entries)?, "items": entries,
        "effect": "Delete the selected global entries, all explicit Context assignments and their corrections; Everywhere entries are inherited by other Contexts.",
        "usage_caveat": "A lack of recorded matches does not prove an item is unused.",
        "undo_caveat": "Restoration is retained locally for 30 days and cannot restore purged learning evidence. Conflicts prevent restoration."}))
}

pub fn apply_library_delete(db: &Db, items: Vec<LibraryItem>, expected_basis: &str, request_id: &str) -> Result<Value> {
    let items = normalize_items(items)?;
    let request_id = uuid::Uuid::parse_str(request_id).context("Invalid request ID")?.to_string();
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM agent_library_changes WHERE created_at < datetime('now','-30 days')", [])?;
    if let Some((old_basis, snapshot, undone)) = tx.query_row("SELECT basis,snapshot,undone FROM agent_library_changes WHERE request_id=?1", [&request_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,bool>(2)?))).optional()? {
        ensure!(old_basis == expected_basis, "Request ID already used for another preview");
        let original: Vec<Entry> = serde_json::from_str(&snapshot)?;
        let old_items = original.into_iter().map(|e| LibraryItem { kind:e.kind, uuid:e.uuid }).collect::<Vec<_>>();
        ensure!(items == old_items, "Request ID already used for different items");
        return Ok(json!({"request_id":request_id,"undone":undone,"replayed":true}));
    }
    let entries = read_entries(&tx, &items)?;
    ensure!(basis(&entries)? == expected_basis, "Library changed since preview; preview again");
    let snapshot = serde_json::to_string(&entries)?;
    for entry in &entries {
        if entry.kind == "dictionary" { delete_dictionary_entry_conn(&tx, entry.id)?; }
        else { delete_snippet_conn(&tx, entry.id)?; }
    }
    let log_seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0) FROM sync_log", [], |r| r.get(0))?;
    tx.execute("INSERT INTO agent_library_changes(request_id,basis,snapshot,log_seq) VALUES (?1,?2,?3,?4)", params![request_id, expected_basis, snapshot, log_seq])?;
    tx.commit()?;
    invalidate_snippet_cache();
    Ok(json!({"request_id":request_id,"deleted":entries.len(),"undone":false,"replayed":false}))
}

pub fn undo_library_delete(db: &Db, request_id: &str) -> Result<Value> {
    let request_id = uuid::Uuid::parse_str(request_id).context("Invalid request ID")?.to_string();
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let (snapshot, undone, log_seq): (String, bool, i64) = tx.query_row("SELECT snapshot,undone,log_seq FROM agent_library_changes WHERE request_id=?1 AND created_at >= datetime('now','-30 days')", [&request_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.context("No recoverable change for this request")?;
    if undone { return Ok(json!({"request_id":request_id,"undone":true,"replayed":true})); }
    let entries: Vec<Entry> = serde_json::from_str(&snapshot)?;
    for e in &entries {
        let table = if e.kind == "dictionary" { "dictionary" } else { "snippets" };
        let changed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_log WHERE seq > ?1 AND table_name=?2 AND row_uuid=?3)", params![log_seq,table,e.uuid], |r| r.get(0))?;
        ensure!(!changed, "Cannot restore an item that changed after deletion");
        for c in &e.corrections {
            let changed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_log WHERE seq > ?1 AND table_name='dictionary_corrections' AND row_uuid=?2)", params![log_seq,c.uuid], |r| r.get(0))?;
            ensure!(!changed, "Cannot restore a correction that changed after deletion");
        }
        if e.kind == "dictionary" {
            tx.execute("INSERT INTO dictionary(uuid,term,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![e.uuid,e.name,e.mistake,e.auto_learned,e.correction_count,e.confidence_tier,e.last_seen_at,e.created_at]).context("Cannot restore a dictionary item; it may have been recreated or changed")?;
        } else {
            tx.execute("INSERT INTO snippets(uuid,trigger,expansion,instructions,use_count,created_at) VALUES (?1,?2,?3,?4,?5,?6)", params![e.uuid,e.name,e.expansion,e.instructions,e.use_count,e.created_at]).context("Cannot restore a snippet; it may have been recreated or changed")?;
        }
        let id = tx.last_insert_rowid();
        for a in &e.assignments {
            let context_id: i64 = tx.query_row("SELECT id FROM contexts WHERE uuid=?1", [&a.uuid], |r| r.get(0)).optional()?.context("Cannot restore an assignment because its Context no longer exists")?;
            if e.kind == "dictionary" { tx.execute("INSERT INTO dictionary_contexts(context_id,dictionary_id) VALUES (?1,?2)", params![context_id,id])?; }
            else { tx.execute("INSERT INTO snippet_contexts(context_id,snippet_id) VALUES (?1,?2)", params![context_id,id])?; }
        }
        for c in &e.corrections {
            let context_id: i64 = tx.query_row("SELECT id FROM contexts WHERE uuid=?1", [&c.context_uuid], |r| r.get(0)).optional()?.context("Cannot restore a correction because its Context no longer exists")?;
            tx.execute("INSERT INTO dictionary_corrections(uuid,context_id,dictionary_id,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![c.uuid,context_id,id,c.mistake,c.auto_learned,c.correction_count,c.confidence_tier,c.last_seen_at,c.created_at]).context("Cannot restore a correction because its spelling is already assigned")?;
        }
    }
    tx.execute("UPDATE agent_library_changes SET undone=1 WHERE request_id=?1", [&request_id])?;
    tx.commit()?;
    invalidate_snippet_cache();
    Ok(json!({"request_id":request_id,"restored":entries.len(),"undone":true,"replayed":false,"learning_evidence_restored":false}))
}
