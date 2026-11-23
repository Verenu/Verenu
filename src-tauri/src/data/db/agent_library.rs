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
    #[serde(default)]
    replacement_confidence: Option<String>,
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
                "SELECT id,uuid,term,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at,replacement_confidence FROM dictionary WHERE uuid=?1",
                [&item.uuid], |r| Ok(Entry { kind: item.kind.clone(), id:r.get(0)?, uuid:r.get(1)?, name:r.get(2)?, mistake:r.get(3)?, auto_learned:r.get(4)?, correction_count:r.get(5)?, confidence_tier:Some(r.get(6)?), last_seen_at:r.get(7)?, created_at:r.get(8)?, replacement_confidence:Some(r.get(9)?), expansion:None, instructions:None, use_count:0, assignments:vec![], corrections:vec![], local_matches:0, local_last_matched_at:None }),
            ).optional()?,
            "snippet" => conn.query_row(
                "SELECT id,uuid,trigger,expansion,instructions,use_count,created_at FROM snippets WHERE uuid=?1",
                [&item.uuid], |r| Ok(Entry { kind:item.kind.clone(), id:r.get(0)?, uuid:r.get(1)?, name:r.get(2)?, expansion:Some(r.get(3)?), instructions:Some(r.get(4)?), use_count:r.get(5)?, created_at:r.get(6)?, replacement_confidence:None, mistake:None, auto_learned:0, correction_count:0, confidence_tier:None, last_seen_at:None, assignments:vec![], corrections:vec![], local_matches:0, local_last_matched_at:None }),
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
    let mut stmt = conn.prepare("SELECT 'dictionary',d.id,d.uuid,d.term FROM dictionary d WHERE NOT EXISTS (SELECT 1 FROM vocabulary_expiration_marks m LEFT JOIN vocabulary_expiration_usage u ON u.item_uuid=m.item_uuid WHERE m.item_uuid=d.uuid AND MAX(d.created_at,COALESCE(u.last_used_at,'')) <= m.cutoff) UNION ALL SELECT 'snippet',id,uuid,trigger FROM snippets ORDER BY 1,4")?;
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
    restore_entries(&tx, &entries, log_seq)?;
    tx.execute("UPDATE agent_library_changes SET undone=1 WHERE request_id=?1", [&request_id])?;
    tx.commit()?;
    invalidate_snippet_cache();
    Ok(json!({"request_id":request_id,"restored":entries.len(),"undone":true,"replayed":false,"learning_evidence_restored":false}))
}

fn restore_entries(tx: &Connection, entries: &[Entry], log_seq: i64) -> Result<()> {
    for e in entries {
        let table = if e.kind == "dictionary" { "dictionary" } else { "snippets" };
        let changed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_log WHERE seq > ?1 AND table_name=?2 AND row_uuid=?3)", params![log_seq,table,e.uuid], |r| r.get(0))?;
        ensure!(!changed, "Cannot restore an item that changed after deletion");
        for c in &e.corrections {
            let changed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_log WHERE seq > ?1 AND table_name='dictionary_corrections' AND row_uuid=?2)", params![log_seq,c.uuid], |r| r.get(0))?;
            ensure!(!changed, "Cannot restore a correction that changed after deletion");
        }
        if e.kind == "dictionary" {
            tx.execute("INSERT INTO dictionary(uuid,term,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at,replacement_confidence) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![e.uuid,e.name,e.mistake,e.auto_learned,e.correction_count,e.confidence_tier,e.last_seen_at,e.created_at,e.replacement_confidence.as_deref().unwrap_or("default")]).context("Cannot restore a dictionary item; it may have been recreated or changed")?;
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
            super::dictionary::check_dictionary_mistake_conflicts(tx, context_id, Some(id), Some(&c.mistake))
                .context("Cannot restore a correction because its spelling is already assigned")?;
            tx.execute("INSERT INTO dictionary_corrections(uuid,context_id,dictionary_id,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![c.uuid,context_id,id,c.mistake,c.auto_learned,c.correction_count,c.confidence_tier,c.last_seen_at,c.created_at]).context("Cannot restore a correction because its spelling is already assigned")?;
        }
    }
    Ok(())
}

fn expiration_sync_pending(conn: &Connection) -> Result<bool> {
    let direct: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_peers p LEFT JOIN sync_peer_capabilities c ON c.device_uuid=p.device_uuid LEFT JOIN vocabulary_policy_peer a ON a.device_uuid=p.device_uuid LEFT JOIN sync_setting_meta s ON s.key='vocabulary_expiration_policy' WHERE COALESCE(c.vocabulary_usage,0)=0 OR p.last_sync_at IS NULL OR p.needs_snapshot != 0 OR a.ts_ms IS NULL OR a.ts_ms!=COALESCE(s.ts_ms,0) OR a.origin!=COALESCE(s.origin,''))", [], |r| r.get(0))?;
    Ok(direct || crate::sync::engine::vocabulary_topology_pending(conn)?)
}

pub fn expire_vocabulary(db: &Db) -> Result<usize> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let (days, since): (Option<i64>, String) = tx.query_row(
        "SELECT days, tracking_since FROM vocabulary_expiration_settings WHERE id=1", [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let Some(days) = days.filter(|days| (7..=60).contains(days)) else { return Ok(0); };
    if expiration_sync_pending(&tx)? {
        tx.execute("UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now') WHERE id=1", [])?;
        tx.commit()?;
        return Ok(0);
    }
    let uuids = tx.prepare(
        "SELECT d.uuid FROM dictionary d LEFT JOIN vocabulary_expiration_usage u ON u.item_uuid=d.uuid
         WHERE d.uuid IS NOT NULL AND MAX(d.created_at, ?1, COALESCE(u.last_used_at, ?1)) <= datetime('now', ?2)
         AND NOT EXISTS (SELECT 1 FROM vocabulary_expiration_marks m WHERE m.item_uuid=d.uuid AND MAX(d.created_at,COALESCE(u.last_used_at,'')) <= m.cutoff)",
    )?.query_map(params![since, format!("-{days} days")], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let policy = crate::sync::engine::vocabulary_policy(&tx)?;
    for uuid in &uuids {
        tx.execute("INSERT INTO vocabulary_expiration_marks(item_uuid,cutoff,deleted_at,request_id,policy_ts,policy_origin) VALUES (?1,datetime('now',?2),datetime('now'),?3,?4,?5) ON CONFLICT(item_uuid) DO UPDATE SET cutoff=excluded.cutoff, deleted_at=excluded.deleted_at, request_id=excluded.request_id,policy_ts=excluded.policy_ts,policy_origin=excluded.policy_origin", params![uuid,format!("-{days} days"),uuid::Uuid::new_v4().to_string(),policy.ts_ms,policy.origin])?;
        crate::sync::engine::append_self_log(&tx, "vocabulary_expiration_marks", uuid, "upsert")?;
        crate::sync::engine::reconcile_vocabulary_identities(&tx, uuid)?;
    }
    tx.commit()?;
    Ok(uuids.len())
}

pub fn set_vocabulary_expiration(db: &Db, days: Option<i64>) -> Result<()> {
    ensure!(days.is_none_or(|days| (7..=60).contains(&days)), "Choose between 7 and 60 days");
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let old: Option<i64> = tx.query_row("SELECT days FROM vocabulary_expiration_settings WHERE id=1", [], |r| r.get(0))?;
    if old != days {
        tx.execute("UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now'), days=?1 WHERE id=1", [days])?;
        crate::sync::engine::stamp_vocabulary_policy(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn vocabulary_expiration_status(db: &Db) -> Result<Value> {
    let conn = lock_conn(db)?;
    let (days, since): (Option<i64>, String) = conn.query_row("SELECT days,tracking_since FROM vocabulary_expiration_settings WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?)))?;
    let paused = expiration_sync_pending(&conn)?;
    let mut stmt = conn.prepare("SELECT m.request_id,d.term,m.deleted_at FROM vocabulary_expiration_marks m JOIN dictionary d ON d.uuid=m.item_uuid LEFT JOIN vocabulary_expiration_usage u ON u.item_uuid=m.item_uuid WHERE MAX(d.created_at,COALESCE(u.last_used_at,'')) <= m.cutoff ORDER BY m.deleted_at DESC,m.request_id")?;
    let entries = stmt.query_map([], |r| Ok(json!({"request_id":r.get::<_,String>(0)?,"term":r.get::<_,String>(1)?,"deleted_at":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reviews_stmt = conn.prepare("SELECT r.context_uuid,r.dictionary_uuid,r.dictionary_term,r.reason,
        (SELECT c.name FROM contexts c WHERE c.uuid=r.context_uuid OR (c.is_everywhere=1 AND r.context_uuid='everywhere-0000-0000-0000-000000000001') LIMIT 1)
        FROM vocabulary_membership_reviews r ORDER BY r.dictionary_term,r.context_uuid")?;
    let mut reviews = reviews_stmt.query_map([], |r| Ok(json!({"context_uuid":r.get::<_,String>(0)?,"dictionary_uuid":r.get::<_,String>(1)?,"term":r.get::<_,String>(2)?,"reason":r.get::<_,String>(3)?,"context_name":r.get::<_,Option<String>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for review in &mut reviews {
        let dictionary_uuid = review["dictionary_uuid"].as_str().context("Missing review identity")?;
        let review_context = review["context_uuid"].as_str().context("Missing review Context")?;
        let mut stmt = conn.prepare("SELECT kind,row_uuid,origin,origin_seq,payload FROM vocabulary_review_payloads WHERE dictionary_uuid=?1 ORDER BY kind DESC,ts_ms,origin,origin_seq")?;
        let rows = stmt.query_map([dictionary_uuid], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut candidates = Vec::new();
        for (kind, row_uuid, origin, origin_seq, payload) in rows {
            let value: Value = serde_json::from_str(&payload)?;
            if kind == "correction" {
                let correction_context = value["context_uuid"].as_str().context("Missing correction Context")?;
                let resolve = |uuid: &str| -> rusqlite::Result<Option<i64>> { conn.query_row("SELECT id FROM contexts WHERE uuid=?1 OR (is_everywhere=1 AND ?1='everywhere-0000-0000-0000-000000000001') OR uuid=(SELECT local_uuid FROM backup_context_aliases WHERE source_uuid=?1) ORDER BY (uuid=?1) DESC LIMIT 1", [uuid], |r| r.get(0)).optional() };
                let context_id = resolve(review_context)?;
                if context_id.is_none() || context_id != resolve(correction_context)? { continue; }
            }
            candidates.push(json!({"kind":kind,"row_uuid":row_uuid,"origin":origin,"origin_seq":origin_seq,"value":value}));
        }
        review["candidates"] = json!(candidates);
    }
    Ok(json!({"days":days,"tracking_since":since,"paused_for_sync":paused,"deleted":entries,"reviews":reviews}))
}

pub fn acknowledge_vocabulary_review(db: &Db, context_uuid: &str, dictionary_uuid: &str) -> Result<()> {
    let conn = lock_conn(db)?;
    let has_archived_edit: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM vocabulary_review_payloads WHERE dictionary_uuid=?1)", [dictionary_uuid], |r| r.get(0))?;
    ensure!(!has_archived_edit, "An archived vocabulary edit must be resolved before dismissing this review");
    let changed = conn.execute("DELETE FROM vocabulary_membership_reviews WHERE context_uuid=?1 AND dictionary_uuid=?2", params![context_uuid,dictionary_uuid])?;
    anyhow::ensure!(changed == 1, "This vocabulary review is no longer pending");
    Ok(())
}

pub fn restore_vocabulary_review_payload(db: &Db, context_uuid: &str, kind: &str, row_uuid: &str, origin: &str, origin_seq: i64) -> Result<()> {
    ensure!(matches!(kind, "dictionary" | "correction"), "Unknown vocabulary review item");
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let (dictionary_uuid, payload): (String, String) = tx.query_row(
        "SELECT dictionary_uuid,payload FROM vocabulary_review_payloads WHERE kind=?1 AND row_uuid=?2 AND origin=?3 AND origin_seq=?4",
        params![kind,row_uuid,origin,origin_seq], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional()?.context("This vocabulary review item is no longer available")?;
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM vocabulary_membership_reviews WHERE context_uuid=?1 AND dictionary_uuid=?2)", params![context_uuid,dictionary_uuid], |r| r.get(0))?;
    ensure!(pending, "This vocabulary item is no longer awaiting review");
    let review_context_id: i64 = tx.query_row("SELECT id FROM contexts WHERE uuid=?1 OR (is_everywhere=1 AND ?1='everywhere-0000-0000-0000-000000000001') OR uuid=(SELECT local_uuid FROM backup_context_aliases WHERE source_uuid=?1) ORDER BY (uuid=?1) DESC LIMIT 1", [context_uuid], |r| r.get(0)).optional()?.context("Cannot restore an assignment because its Context no longer exists")?;
    if kind == "dictionary" {
        let row: crate::sync::engine::DictionaryRow = serde_json::from_str(&payload)?;
        let existing: Option<(i64,bool)> = tx.query_row("SELECT id,term=?2 AND auto_learned=?3 AND correction_count=?4 AND confidence_tier=?5 AND last_seen_at IS ?6 AND created_at=?7 AND replacement_confidence=?8 FROM dictionary WHERE uuid=?1", params![row_uuid,row.term,row.auto_learned,row.correction_count,row.confidence_tier,row.last_seen_at,row.created_at,row.replacement_confidence.as_deref().unwrap_or("default")], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let dictionary_id = if let Some((id,matches)) = existing {
            ensure!(matches, "The current vocabulary differs from this saved change");
            id
        } else {
        tx.execute("INSERT INTO dictionary(uuid,term,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at,replacement_confidence) VALUES (?1,?2,NULL,?3,?4,?5,?6,?7,?8)",
            params![row_uuid,row.term,row.auto_learned,row.correction_count,row.confidence_tier,row.last_seen_at,row.created_at,row.replacement_confidence.as_deref().unwrap_or("default")])
            .context("Cannot restore this item because its name or identity is already in use")?;
        tx.last_insert_rowid() };
        tx.execute("INSERT OR IGNORE INTO dictionary_contexts(context_id,dictionary_id) VALUES (?1,?2)", params![review_context_id,dictionary_id])?;
    } else {
        let row: crate::sync::protocol::DictionaryCorrectionRow = serde_json::from_str(&payload)?;
        let dictionary_id: i64 = tx.query_row("WITH RECURSIVE ids(uuid) AS (SELECT ?1 UNION SELECT CASE WHEN l.left_uuid=ids.uuid THEN l.right_uuid ELSE l.left_uuid END FROM vocabulary_identity_links l JOIN ids ON l.left_uuid=ids.uuid OR l.right_uuid=ids.uuid) SELECT id FROM dictionary WHERE uuid IN (SELECT uuid FROM ids) ORDER BY (uuid=?1) DESC LIMIT 1", [&dictionary_uuid], |r| r.get(0)).optional()?.context("Restore the vocabulary item before its correction")?;
        let context_id: i64 = tx.query_row("SELECT id FROM contexts WHERE uuid=?1 OR (is_everywhere=1 AND ?1='everywhere-0000-0000-0000-000000000001') OR uuid=(SELECT local_uuid FROM backup_context_aliases WHERE source_uuid=?1) ORDER BY (uuid=?1) DESC LIMIT 1", [&row.context_uuid], |r| r.get(0)).optional()?.context("Cannot restore a correction because its Context no longer exists")?;
        ensure!(context_id == review_context_id, "This correction belongs to another Context");
        super::dictionary::check_dictionary_mistake_conflicts(&tx, context_id, Some(dictionary_id), Some(&row.mistake))?;
        tx.execute("INSERT OR IGNORE INTO dictionary_contexts(context_id,dictionary_id) VALUES (?1,?2)", params![context_id,dictionary_id])?;
        tx.execute("UPDATE vocabulary_correction_versions SET retired=1 WHERE correction_uuid=?1 AND retired=0", [row_uuid])?;
        tx.execute("INSERT INTO dictionary_corrections(uuid,context_id,dictionary_id,mistake,auto_learned,correction_count,confidence_tier,last_seen_at,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![row_uuid,context_id,dictionary_id,row.mistake,row.auto_learned,row.correction_count,row.confidence_tier,row.last_seen_at,row.created_at]).context("Cannot restore this correction because it already exists")?;
    }
    tx.commit()?;
    Ok(())
}

pub fn discard_vocabulary_review_payload(db: &Db, kind: &str, row_uuid: &str, origin: &str, origin_seq: i64) -> Result<()> {
    ensure!(matches!(kind, "dictionary" | "correction"), "Unknown vocabulary review item");
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let changed = tx.execute("DELETE FROM vocabulary_review_payloads WHERE kind=?1 AND row_uuid=?2 AND origin=?3 AND origin_seq=?4", params![kind,row_uuid,origin,origin_seq])?;
    ensure!(changed == 1, "This vocabulary review item is no longer available");
    let key = crate::sync::engine::vocabulary_review_key(kind,row_uuid,origin,origin_seq)?;
    crate::sync::engine::append_self_log(&tx,"vocabulary_review_payloads",&key,"delete")?;
    tx.commit()?;
    Ok(())
}

pub(super) fn ensure_expired_corrections_available(conn: &Connection, item_uuid: &str) -> Result<()> {
    let mut stmt = conn.prepare("SELECT d.id,c.context_id,c.mistake FROM dictionary d JOIN dictionary_corrections c ON c.dictionary_id=d.id WHERE d.uuid=?1")?;
    let corrections = stmt.query_map([item_uuid], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, context_id, mistake) in corrections {
        super::dictionary::check_dictionary_mistake_conflicts(conn, context_id, Some(id), Some(&mistake))
            .map_err(|err| anyhow::anyhow!("Cannot restore: {err}"))?;
    }
    Ok(())
}

pub fn restore_expired_vocabulary(db: &Db, request_id: &str) -> Result<()> {
    let request_id = uuid::Uuid::parse_str(request_id)?.to_string();
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let item_uuid: String = tx.query_row("SELECT m.item_uuid FROM vocabulary_expiration_marks m JOIN dictionary d ON d.uuid=m.item_uuid LEFT JOIN vocabulary_expiration_usage u ON u.item_uuid=m.item_uuid WHERE m.request_id=?1 AND MAX(d.created_at,COALESCE(u.last_used_at,''))<=m.cutoff", [&request_id], |r| r.get(0)).optional()?.context("No recoverable vocabulary for this request")?;
    ensure_expired_corrections_available(&tx, &item_uuid)?;
    tx.execute("UPDATE vocabulary_correction_versions SET retired=1 WHERE retired=0 AND correction_uuid IN (SELECT c.uuid FROM dictionary_corrections c JOIN dictionary d ON d.id=c.dictionary_id WHERE d.uuid=?1)", [&item_uuid])?;
    tx.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at,seen_retired,seen_expired)
      SELECT lower(hex(randomblob(16))),dc.uuid,CASE WHEN c.is_everywhere=1 THEN 'everywhere-0000-0000-0000-000000000001' ELSE c.uuid END,
        d.uuid,d.term,dc.mistake,dc.auto_learned,datetime('now'),
        COALESCE((SELECT json_group_array(v.version_id) FROM vocabulary_correction_versions v
          WHERE v.context_uuid=CASE WHEN c.is_everywhere=1 THEN 'everywhere-0000-0000-0000-000000000001' ELSE c.uuid END
            AND (v.retired=1 OR EXISTS(SELECT 1 FROM vocabulary_correction_decisions x JOIN vocabulary_correction_versions w ON w.version_id=x.winner_version
              WHERE x.loser_version=v.version_id AND w.retired=1))),'[]'),
        COALESCE((SELECT json_group_array(m.item_uuid || ':' || m.request_id) FROM vocabulary_expiration_marks m JOIN dictionary x ON x.uuid=m.item_uuid
          LEFT JOIN vocabulary_expiration_usage u ON u.item_uuid=x.uuid WHERE MAX(x.created_at,COALESCE(u.last_used_at,''))<=m.cutoff),'[]')
      FROM dictionary_corrections dc JOIN dictionary d ON d.id=dc.dictionary_id JOIN contexts c ON c.id=dc.context_id WHERE d.uuid=?1", [&item_uuid])?;
    tx.execute("INSERT INTO vocabulary_expiration_usage(item_uuid,last_used_at) VALUES (?1,datetime('now')) ON CONFLICT(item_uuid) DO UPDATE SET last_used_at=MAX(last_used_at,excluded.last_used_at)", [&item_uuid])?;
    crate::sync::engine::reconcile_vocabulary_identities(&tx, &item_uuid)?;
    crate::sync::engine::append_self_log(&tx, "vocabulary_expiration_usage", &item_uuid, "upsert")?;
    super::dictionary::record_correction_history_decisions(&tx)?;
    tx.commit()?;
    Ok(())
}
