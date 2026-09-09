//! Tests for the sync engine: change capture, LWW merging, deletes,
//! natural-key conflicts, snapshots, counter/settings merge, and a full
//! protocol session over an in-memory duplex stream.

use anyhow::Result;
use serde_json::json;
use std::net::IpAddr;

use super::engine::{self, SyncHost, SYNCABLE_SETTINGS};
use super::protocol::Message;
use super::store as sync_store;
use crate::data::db;
use crate::DbHandle;

// ---- helpers ----

#[test]
fn listener_port_is_stable_and_device_specific() {
    let first = super::manager::listener_port_for_uuid("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    let repeated = super::manager::listener_port_for_uuid("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    let second = super::manager::listener_port_for_uuid("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");

    assert_eq!(first, repeated);
    assert_ne!(first, second);
    assert!((49_152..=u16::MAX).contains(&first));
    assert!((49_152..=u16::MAX).contains(&second));
}

#[test]
fn connection_candidates_try_stable_port_before_stale_advertisement() {
    let uuid = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    let candidates =
        super::manager::connection_candidates(&["192.168.0.86:58546".to_string()], 58546, uuid);
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0].port(),
        super::manager::listener_port_for_uuid(uuid)
    );
    assert_eq!(candidates[1].port(), 58546);
}

#[test]
fn automatic_sync_has_exactly_one_initiator() {
    let a = "a734d68e-0000-0000-0000-000000000000";
    let b = "be256d68-0000-0000-0000-000000000000";
    assert!(super::manager::should_auto_initiate(a, b));
    assert!(!super::manager::should_auto_initiate(b, a));
}

#[tokio::test]
async fn incoming_connection_admission_is_bounded() {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(
        super::manager::MAX_INCOMING_CONNECTIONS,
    ));
    let mut permits = Vec::new();
    for _ in 0..super::manager::MAX_INCOMING_CONNECTIONS {
        permits.push(slots.clone().try_acquire_owned().expect("slot available"));
    }
    assert!(
        slots.clone().try_acquire_owned().is_err(),
        "the listener must reject excess concurrent connections"
    );
    drop(permits);
    assert!(
        slots.try_acquire().is_ok(),
        "a released slot must be reusable"
    );
}

#[test]
fn discovery_excludes_tunnels_virtual_and_non_lan_interfaces() {
    assert!(!super::manager::is_discovery_interface_allowed(
        "Tailscale",
        "100.99.57.54".parse::<IpAddr>().unwrap(),
        true,
    ));
    assert!(!super::manager::is_discovery_interface_allowed(
        "Ethernet",
        "100.99.57.54".parse::<IpAddr>().unwrap(),
        false,
    ));
    assert!(!super::manager::is_discovery_interface_allowed(
        "vEthernet (Default Switch)",
        "172.20.0.1".parse::<IpAddr>().unwrap(),
        false,
    ));
    assert!(!super::manager::is_discovery_interface_allowed(
        "Ethernet",
        "169.254.61.34".parse::<IpAddr>().unwrap(),
        false,
    ));
    assert!(super::manager::is_discovery_interface_allowed(
        "Ethernet 2",
        "192.168.0.187".parse::<IpAddr>().unwrap(),
        false,
    ));
}

fn test_db(device_uuid: &str) -> DbHandle {
    let conn = db::open(":memory:").expect("test db");
    {
        let guard = conn.lock().expect("lock");
        sync_store::ensure_self_identity(&guard, device_uuid, "Test Device").expect("identity");
    }
    conn
}

fn uuid(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        crate::sync::identity::fingerprint_of(prefix.as_bytes())
            .get(..8)
            .expect("len")
            .to_string()
    )
}

fn row_uuid(conn: &rusqlite::Connection, table: &str, id: i64) -> String {
    conn.query_row(
        &format!("SELECT uuid FROM {table} WHERE id = ?1"),
        rusqlite::params![id],
        |r| r.get(0),
    )
    .expect("row uuid")
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// Exchanges the full collapsed change sets between two databases until both
/// converge (two rounds is always enough for these tests).
fn exchange(a: &DbHandle, b: &DbHandle) {
    for _ in 0..2 {
        for (from, to) in [(a, b), (b, a)] {
            let ops = {
                let conn = from.lock().expect("lock");
                let mut progress = engine::SnapshotProgress::default();
                let (ops, _cursor, _done) =
                    engine::collect_ops(&conn, 0, false, 10_000, &mut progress).expect("collect");
                ops
            };
            let summary = {
                let conn = to.lock().expect("lock");
                engine::apply_ops(&conn, &ops).expect("apply")
            };
            let _ = summary;
        }
    }
}

struct TestHost {
    uuid: String,
    db_probe: Option<DbHandle>,
    settings: std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>,
    stamps: std::sync::Mutex<std::collections::HashMap<String, (i64, String)>>,
    setting_batch_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fail_setting_batch: bool,
}

impl TestHost {
    fn new(uuid: &str) -> Self {
        Self {
            uuid: uuid.to_string(),
            db_probe: None,
            settings: Default::default(),
            stamps: Default::default(),
            setting_batch_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            fail_setting_batch: false,
        }
    }

    fn with_db_probe(mut self, db: &DbHandle) -> Self {
        self.db_probe = Some(db.clone());
        self
    }

    fn with_failing_setting_batch(mut self) -> Self {
        self.fail_setting_batch = true;
        self
    }
}

impl SyncHost for TestHost {
    fn device_uuid(&self) -> String {
        self.uuid.clone()
    }

    fn device_name(&self) -> String {
        "Test".to_string()
    }

    fn app_version(&self) -> String {
        "test".to_string()
    }

    fn settings_payload(&self) -> Result<Vec<super::protocol::SettingRecord>> {
        if let Some(db) = &self.db_probe {
            let conn = db.lock().expect("settings payload database lock");
            let _: i64 = conn.query_row("SELECT 1", [], |row| row.get(0))?;
        }
        let settings = self.settings.lock().expect("settings");
        let stamps = self.stamps.lock().expect("stamps");
        Ok(settings
            .iter()
            .map(|(key, value)| super::protocol::SettingRecord {
                key: key.clone(),
                value: value.clone(),
                ts_ms: stamps.get(key).map(|s| s.0).unwrap_or(0),
                origin: stamps.get(key).map(|s| s.1.clone()).unwrap_or_default(),
            })
            .collect())
    }

    fn apply_remote_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        self.settings
            .lock()
            .expect("settings")
            .insert(key.to_string(), value.clone());
        Ok(())
    }

    fn apply_remote_settings(
        &self,
        settings: &[(String, serde_json::Value)],
    ) -> Result<(), String> {
        self.setting_batch_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail_setting_batch {
            return Err("simulated settings persistence failure".to_string());
        }
        let mut values = self.settings.lock().expect("settings");
        for (key, value) in settings {
            values.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}

fn peer_of(device_uuid: &str) -> sync_store::SyncPeer {
    sync_store::SyncPeer {
        device_uuid: device_uuid.to_string(),
        name: "Peer".to_string(),
        cert_fp: String::new(),
        added_at: String::new(),
        last_sync_at: None,
        send_cursor: 0,
        needs_snapshot: true,
        last_error: None,
    }
}

// ---- change capture ----

#[test]
fn triggers_capture_content_changes() {
    let a_uuid = uuid("aaaa");
    let db = test_db(&a_uuid);
    let dict =
        db::insert_dictionary_entry_returning(&db, "Groq", Some("Grock"), None).expect("insert");
    let snippet =
        db::insert_snippet_returning(&db, "addr", "123 Main St", "", None).expect("snippet");
    let context =
        db::insert_context_returning(&db, "Work", None, None, None, None, false).expect("context");
    db::assign_context_target(&db, context.id, "code.exe").expect("target");

    let conn = db.lock().expect("lock");
    let log_count = count(&conn, "SELECT COUNT(*) FROM sync_log");
    assert!(log_count >= 4, "expected >= 4 log entries, got {log_count}");
    let dict_uuid = row_uuid(&conn, "dictionary", dict.id);
    let stamped = sync_store::latest_op_stamp(&conn, "dictionary", &dict_uuid)
        .expect("stamp")
        .expect("stamp exists");
    assert_eq!(stamped.1, a_uuid, "origin should be this device");
    let snippet_uuid = row_uuid(&conn, "snippets", snippet.id);
    assert!(
        uuid::Uuid::parse_str(&dict_uuid).is_ok(),
        "trigger-generated dictionary UUID should be canonical"
    );
    assert!(
        uuid::Uuid::parse_str(&snippet_uuid).is_ok(),
        "trigger-generated snippet UUID should be canonical"
    );
    assert!(
        sync_store::latest_op_stamp(&conn, "snippets", &snippet_uuid)
            .expect("stamp")
            .is_some(),
        "snippet change captured"
    );
}

#[test]
fn engine_applied_ops_are_not_recaptured() {
    let db = test_db(&uuid("bbbb"));
    let op = test_dictionary_op("term-a", 100);
    {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[op.clone()]).expect("apply");
    }
    let conn = db.lock().expect("lock");
    // Exactly one log entry: the manually logged remote op (with its original
    // stamp), not a re-captured local echo.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_log"), 1);
    let stamp = sync_store::latest_op_stamp(&conn, "dictionary", &op.row_uuid)
        .expect("stamp")
        .expect("stamp");
    assert_eq!(stamp.0, 100, "remote stamp must be preserved");
}

fn test_dictionary_op(term: &str, ts_ms: i64) -> super::protocol::SyncOp {
    super::protocol::SyncOp {
        table: "dictionary".to_string(),
        row_uuid: uuid(term),
        op: "upsert".to_string(),
        ts_ms,
        origin: uuid("origin"),
        origin_seq: ts_ms,
        payload: Some(json!({
            "term": term,
            "mistake": null,
            "auto_learned": false,
            "correction_count": 0,
            "confidence_tier": "manual",
            "last_seen_at": null,
            "created_at": "2026-01-01 00:00:00",
        })),
    }
}

fn correction_row_uuid(conn: &rusqlite::Connection, context_id: i64, dictionary_id: i64) -> String {
    conn.query_row(
        "SELECT uuid FROM dictionary_corrections
          WHERE context_id = ?1 AND dictionary_id = ?2
          ORDER BY id LIMIT 1",
        rusqlite::params![context_id, dictionary_id],
        |r| r.get(0),
    )
    .expect("correction uuid")
}

fn context_uuid(conn: &rusqlite::Connection, context_id: i64) -> String {
    conn.query_row(
        "SELECT uuid FROM contexts WHERE id = ?1",
        rusqlite::params![context_id],
        |r| r.get(0),
    )
    .expect("context uuid")
}

fn dictionary_op_with_uuid(row_uuid: &str, term: &str, ts_ms: i64) -> super::protocol::SyncOp {
    super::protocol::SyncOp {
        table: "dictionary".to_string(),
        row_uuid: row_uuid.to_string(),
        op: "upsert".to_string(),
        ts_ms,
        origin: uuid("remote-dictionary"),
        origin_seq: ts_ms,
        payload: Some(json!({
            "term": term,
            "mistake": null,
            "auto_learned": false,
            "correction_count": 0,
            "confidence_tier": "manual",
            "last_seen_at": null,
            "created_at": "2026-01-01 00:00:00",
        })),
    }
}

fn dictionary_correction_op(
    row_uuid: &str,
    context_uuid: &str,
    dictionary_uuid: &str,
    dictionary_term: &str,
    mistake: &str,
    ts_ms: i64,
) -> super::protocol::SyncOp {
    super::protocol::SyncOp {
        table: "dictionary_corrections".to_string(),
        row_uuid: row_uuid.to_string(),
        op: "upsert".to_string(),
        ts_ms,
        origin: uuid("remote-correction"),
        origin_seq: ts_ms,
        payload: Some(json!({
            "context_uuid": context_uuid,
            "dictionary_uuid": dictionary_uuid,
            "dictionary_term": dictionary_term,
            "mistake": mistake,
            "auto_learned": true,
            "correction_count": 1,
            "confidence_tier": "high",
            "last_seen_at": null,
            "created_at": "2026-01-01 00:00:00",
        })),
    }
}

// ---- bidirectional merge ----

#[test]
fn context_correction_delta_roundtrip_preserves_scope_and_identity() {
    let a = test_db(&uuid("correction-a"));
    let b = test_db(&uuid("correction-b"));
    let context = db::insert_context_returning(&a, "Development", None, None, None, None, false)
        .expect("context");
    let entry = db::insert_dictionary_entry_returning(
        &a,
        "Kubernetes",
        Some("Koobernetes"),
        Some(context.id),
    )
    .expect("dictionary entry");
    let (source_context_uuid, source_correction_uuid) = {
        let conn = a.lock().expect("lock");
        (
            context_uuid(&conn, context.id),
            correction_row_uuid(&conn, context.id, entry.id),
        )
    };

    exchange(&a, &b);

    let conn = b.lock().expect("lock");
    let target_context_id: i64 = conn
        .query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            rusqlite::params![source_context_uuid],
            |r| r.get(0),
        )
        .expect("synced context");
    let targeted_mapping_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary_corrections WHERE context_id = ?1",
            rusqlite::params![target_context_id],
            |r| r.get(0),
        )
        .expect("targeted mapping count");
    assert_eq!(targeted_mapping_count, 1);
    let mapping: (String, String, String, i64) = conn
        .query_row(
            "SELECT c.uuid, d.term, c.mistake, c.auto_learned
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.context_id = ?1",
            rusqlite::params![target_context_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("synced correction");
    assert_eq!(mapping.0, source_correction_uuid);
    assert_eq!(mapping.1, "Kubernetes");
    assert_eq!(mapping.2, "Koobernetes");
    assert_eq!(mapping.3, 0, "manual mapping stays manual over sync");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM dictionary_corrections
              WHERE context_id = (SELECT id FROM contexts WHERE is_everywhere = 1)"
        ),
        0,
        "a targeted correction must not be copied to Everywhere"
    );
}

#[test]
fn correction_snapshot_is_dependency_ordered_and_refreshes_dictionary() {
    let a = test_db(&uuid("correction-snapshot-a"));
    let b = test_db(&uuid("correction-snapshot-b"));
    let context = db::insert_context_returning(&a, "Writing", None, None, None, None, false)
        .expect("context");
    let entry = db::insert_dictionary_entry_returning(
        &a,
        "PostgreSQL",
        Some("PostgresQL"),
        Some(context.id),
    )
    .expect("dictionary entry");
    let conn_a = a.lock().expect("lock");
    let mut progress = engine::SnapshotProgress::default();
    let (ops, cursor, done) =
        engine::collect_ops(&conn_a, 0, true, 10_000, &mut progress).expect("snapshot");
    assert!(done);
    assert!(cursor > 0);
    let correction_index = ops
        .iter()
        .position(|op| op.table == "dictionary_corrections")
        .expect("correction snapshot op");
    let dictionary_index = ops
        .iter()
        .position(|op| op.table == "dictionary")
        .expect("dictionary snapshot op");
    let context_index = ops
        .iter()
        .position(|op| op.table == "contexts" && op.row_uuid == context_uuid(&conn_a, context.id))
        .expect("context snapshot op");
    assert!(dictionary_index < correction_index);
    assert!(context_index < correction_index);
    drop(conn_a);

    let summary = {
        let conn_b = b.lock().expect("lock");
        engine::apply_ops(&conn_b, &ops).expect("apply snapshot")
    };
    assert!(summary.dictionary);
    assert!(summary.dictionary_corrections);
    assert!(summary.touched_tables().contains(&"dictionary_corrections"));
    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 1);
    assert_eq!(
        count(&conn_b, "SELECT COUNT(*) FROM dictionary_corrections"),
        1
    );
    assert_eq!(
        count(&conn_b, "SELECT COUNT(*) FROM dictionary_contexts"),
        1
    );
    let _ = entry;
}

#[test]
fn correction_apply_waits_for_both_parents_even_when_batch_is_reversed() {
    let db = test_db(&uuid("correction-dependencies"));
    let context_name = "Scoped Context";
    let context_uuid = uuid(context_name);
    let dictionary_uuid = uuid("scoped-canonical");
    let correction_uuid = uuid("scoped-correction");
    let context_op = test_context_op(context_name, 100, "editor.exe", None);
    let dictionary_op = dictionary_op_with_uuid(&dictionary_uuid, "ScopedTerm", 101);
    let correction_op = dictionary_correction_op(
        &correction_uuid,
        &context_uuid,
        &dictionary_uuid,
        "ScopedTerm",
        "ScopedMistake",
        102,
    );

    let summary = {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[correction_op, context_op, dictionary_op])
            .expect("apply reversed dependency batch")
    };
    assert_eq!(summary.applied, 3);
    let conn = db.lock().expect("lock");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"),
        1
    );
    let stored: (String, String) = conn
        .query_row(
            "SELECT d.term, c.mistake
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("stored correction");
    assert_eq!(
        stored,
        ("ScopedTerm".to_string(), "ScopedMistake".to_string())
    );
}

#[test]
fn correction_dependency_is_retried_when_parent_arrives_in_a_later_batch() {
    let db = test_db(&uuid("correction-deferred"));
    let context =
        db::insert_context_returning(&db, "Deferred Context", None, None, None, None, false)
            .expect("context");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let dictionary_uuid = uuid("deferred-canonical");
    let correction_uuid = uuid("deferred-correction");
    let correction = dictionary_correction_op(
        &correction_uuid,
        &context_uuid,
        &dictionary_uuid,
        "DeferredTerm",
        "DeferredMistake",
        200,
    );

    let first = engine::apply_ops(&conn, &[correction.clone()]).expect("defer correction");
    assert!(first.deferred);
    assert_eq!(first.applied, 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"),
        0
    );

    let dictionary = dictionary_op_with_uuid(&dictionary_uuid, "DeferredTerm", 201);
    let second = engine::apply_ops(&conn, &[dictionary, correction]).expect("retry correction");
    assert!(!second.deferred);
    assert_eq!(second.applied, 2);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"),
        1
    );
}

#[test]
fn stale_correction_cannot_restore_removed_context_membership() {
    let db = test_db(&uuid("correction-membership-tombstone"));
    let context =
        db::insert_context_returning(&db, "Removed Mapping", None, None, None, None, false)
            .expect("context");
    let entry = db::insert_dictionary_entry_returning(
        &db,
        "MembershipTerm",
        Some("MembershipMistake"),
        Some(context.id),
    )
    .expect("entry");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let dictionary_uuid = row_uuid(&conn, "dictionary", entry.id);
    let correction_uuid = correction_row_uuid(&conn, context.id, entry.id);

    let mut progress = engine::SnapshotProgress::default();
    let (mut context_ops, _, _) =
        engine::collect_ops(&conn, 0, true, 10_000, &mut progress).expect("context snapshot");
    let context_op = context_ops
        .iter_mut()
        .find(|op| op.table == "contexts" && op.row_uuid == context_uuid)
        .expect("context aggregate")
        .clone();
    let mut context_op = context_op;
    context_op.ts_ms = sync_store::now_ms() + 10_000;
    context_op.origin = uuid("membership-removal");
    context_op.origin_seq = 1;
    let payload = context_op.payload.as_mut().expect("context payload");
    payload["dictionary_uuids"] = json!([]);
    payload["dictionary_entries"] = json!([]);

    let stale = dictionary_correction_op(
        &correction_uuid,
        &context_uuid,
        &dictionary_uuid,
        "MembershipTerm",
        "MembershipMistake",
        1,
    );
    let summary = engine::apply_ops(&conn, &[stale, context_op]).expect("remove membership");
    assert!(summary.contexts);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"),
        0
    );
    let assigned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary_contexts WHERE context_id = ?1",
            rusqlite::params![context.id],
            |r| r.get(0),
        )
        .expect("membership count");
    assert_eq!(assigned, 0);
}

#[test]
fn correction_does_not_fallback_after_canonical_delete_tombstone() {
    let db = test_db(&uuid("correction-deleted-parent"));
    let context =
        db::insert_context_returning(&db, "Deleted Parent", None, None, None, None, false)
            .expect("context");
    let old =
        db::insert_dictionary_entry_returning(&db, "DeletedCanonical", None, Some(context.id))
            .expect("old canonical");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let old_dictionary_uuid = row_uuid(&conn, "dictionary", old.id);
    let delete = super::protocol::SyncOp {
        table: "dictionary".to_string(),
        row_uuid: old_dictionary_uuid.clone(),
        op: "delete".to_string(),
        ts_ms: sync_store::now_ms() + 10_000,
        origin: uuid("canonical-delete"),
        origin_seq: 1,
        payload: None,
    };
    engine::apply_ops(&conn, &[delete]).expect("delete canonical");
    drop(conn);
    let replacement =
        db::insert_dictionary_entry_returning(&db, "DeletedCanonical", None, Some(context.id))
            .expect("replacement canonical");
    let conn = db.lock().expect("lock");
    let replacement_uuid = row_uuid(&conn, "dictionary", replacement.id);
    assert_ne!(old_dictionary_uuid, replacement_uuid);

    let stale = dictionary_correction_op(
        &uuid("stale-deleted-correction"),
        &context_uuid,
        &old_dictionary_uuid,
        "DeletedCanonical",
        "StaleMistake",
        sync_store::now_ms() + 20_000,
    );
    let summary = engine::apply_ops(&conn, &[stale]).expect("ignore deleted-parent child");
    assert_eq!(summary.applied, 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"),
        0
    );
}

#[test]
fn context_membership_follows_local_canonical_natural_key_winner() {
    let db = test_db(&uuid("local-canonical-winner"));
    let context =
        db::insert_context_returning(&db, "Natural Key Context", None, None, None, None, false)
            .expect("context");
    let local = db::insert_dictionary_entry_returning(&db, "NaturalTerm", None, Some(context.id))
        .expect("local canonical");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let remote_dictionary_uuid = uuid("losing-remote-canonical");
    let remote_dictionary = dictionary_op_with_uuid(&remote_dictionary_uuid, "NaturalTerm", 1);
    engine::apply_ops(&conn, &[remote_dictionary]).expect("keep local canonical");
    let local_dictionary_uuid = row_uuid(&conn, "dictionary", local.id);
    assert_ne!(local_dictionary_uuid, remote_dictionary_uuid);

    let context_op = super::protocol::SyncOp {
        table: "contexts".to_string(),
        row_uuid: context_uuid.clone(),
        op: "upsert".to_string(),
        ts_ms: sync_store::now_ms() + 20_000,
        origin: uuid("remote-context-membership"),
        origin_seq: 1,
        payload: Some(json!({
            "name": "Natural Key Context",
            "is_everywhere": false,
            "icon": null,
            "tone": null,
            "cleanup_intensity": null,
            "color": null,
            "custom_instructions": null,
            "contextual_formatting_disabled": false,
            "pinned_at": null,
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
            "targets": [],
            "websites": [],
            "dictionary_uuids": [remote_dictionary_uuid],
            "dictionary_entries": [{"uuid": remote_dictionary_uuid, "term": "NaturalTerm"}],
            "snippet_uuids": [],
        })),
    };
    engine::apply_ops(&conn, &[context_op]).expect("resolve membership natural key");
    let assigned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary_contexts WHERE context_id = ?1 AND dictionary_id = ?2",
            rusqlite::params![context.id, local.id],
            |r| r.get(0),
        )
        .expect("membership count");
    assert_eq!(assigned, 1);
}

#[test]
fn correction_follows_natural_key_loser_tombstone() {
    let db = test_db(&uuid("natural-key-child-follow"));
    let context =
        db::insert_context_returning(&db, "Natural Child Context", None, None, None, None, false)
            .expect("context");
    let local =
        db::insert_dictionary_entry_returning(&db, "NaturalChildTerm", None, Some(context.id))
            .expect("local canonical");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let remote_dictionary_uuid = uuid("natural-child-loser");
    engine::apply_ops(
        &conn,
        &[dictionary_op_with_uuid(
            &remote_dictionary_uuid,
            "NaturalChildTerm",
            1,
        )],
    )
    .expect("record natural-key loser");
    let correction = dictionary_correction_op(
        &uuid("natural-child-correction"),
        &context_uuid,
        &remote_dictionary_uuid,
        "NaturalChildTerm",
        "NaturalChildMistake",
        sync_store::now_ms() + 1,
    );
    let summary = engine::apply_ops(&conn, &[correction]).expect("follow natural-key loser");
    assert_eq!(summary.applied, 1);
    let stored_dictionary_id: i64 = conn
        .query_row(
            "SELECT dictionary_id FROM dictionary_corrections WHERE context_id = ?1",
            rusqlite::params![context.id],
            |r| r.get(0),
        )
        .expect("mapping dictionary id");
    assert_eq!(stored_dictionary_id, local.id);
}

#[test]
fn dictionary_natural_key_winner_reparents_child_corrections() {
    let db = test_db(&uuid("reparent-target"));
    let context = db::insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    let local = db::insert_dictionary_entry_returning(
        &db,
        "SharedTerm",
        Some("SharedMistake"),
        Some(context.id),
    )
    .expect("local dictionary entry");
    let conn = db.lock().expect("lock");
    let local_correction_uuid = correction_row_uuid(&conn, context.id, local.id);
    let remote_uuid = uuid("remote-shared-term");
    let remote = dictionary_op_with_uuid(&remote_uuid, "SharedTerm", sync_store::now_ms() + 10_000);
    let summary = engine::apply_ops(&conn, &[remote]).expect("apply remote winner");
    assert_eq!(summary.applied, 1);
    let (dictionary_id, stored_uuid): (i64, String) = conn
        .query_row(
            "SELECT id, uuid FROM dictionary WHERE term = 'SharedTerm'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("winning dictionary row");
    assert_eq!(stored_uuid, remote_uuid);
    let mapping: (String, String) = conn
        .query_row(
            "SELECT c.uuid, c.mistake FROM dictionary_corrections c
              WHERE c.context_id = ?1 AND c.dictionary_id = ?2",
            rusqlite::params![context.id, dictionary_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("reparented correction");
    assert_eq!(mapping.0, local_correction_uuid);
    assert_eq!(mapping.1, "SharedMistake");
}

#[test]
fn correction_delta_falls_back_to_canonical_term_after_uuid_conflict() {
    let db = test_db(&uuid("correction-natural-key"));
    let context = db::insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    let entry = db::insert_dictionary_entry_returning(&db, "SharedTerm", None, Some(context.id))
        .expect("canonical entry");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let local_dictionary_uuid = row_uuid(&conn, "dictionary", entry.id);
    let remote_dictionary_uuid = uuid("losing-canonical");
    let correction = dictionary_correction_op(
        &uuid("in-flight-correction"),
        &context_uuid,
        &remote_dictionary_uuid,
        "SharedTerm",
        "DifferentMistake",
        sync_store::now_ms() + 10_000,
    );
    assert_ne!(local_dictionary_uuid, remote_dictionary_uuid);
    let summary = engine::apply_ops(&conn, &[correction]).expect("apply natural-key fallback");
    assert_eq!(summary.applied, 1);
    let stored: (String, String) = conn
        .query_row(
            "SELECT d.uuid, c.mistake
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.context_id = ?1",
            rusqlite::params![context.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("fallback correction");
    assert_eq!(stored.0, local_dictionary_uuid);
    assert_eq!(stored.1, "DifferentMistake");
}

#[test]
fn correction_delete_is_scoped_and_leaves_canonical_dictionary_row() {
    let db = test_db(&uuid("correction-delete"));
    let context =
        db::insert_context_returning(&db, "Work", None, None, None, None, false).expect("context");
    let entry = db::insert_dictionary_entry_returning(
        &db,
        "TechnicalTerm",
        Some("TechnikalTerm"),
        Some(context.id),
    )
    .expect("entry");
    let conn = db.lock().expect("lock");
    let correction_uuid = correction_row_uuid(&conn, context.id, entry.id);
    let op = super::protocol::SyncOp {
        table: "dictionary_corrections".to_string(),
        row_uuid: correction_uuid.clone(),
        op: "delete".to_string(),
        ts_ms: sync_store::now_ms() + 10_000,
        origin: uuid("remote-delete"),
        origin_seq: 1,
        payload: None,
    };
    let summary = engine::apply_ops(&conn, &[op]).expect("delete correction");
    assert!(summary.dictionary);
    assert!(summary.dictionary_corrections);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 1);
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary_corrections WHERE uuid = ?1",
            rusqlite::params![correction_uuid],
            |r| r.get(0),
        )
        .expect("remaining correction count");
    assert_eq!(
        remaining, 0,
        "the row is removed without deleting the canonical term"
    );
}

#[test]
fn remote_context_delete_moves_corrections_to_everywhere() {
    let db = test_db(&uuid("context-delete"));
    let context = db::insert_context_returning(&db, "Temporary", None, None, None, None, false)
        .expect("context");
    let entry = db::insert_dictionary_entry_returning(
        &db,
        "RetainedTerm",
        Some("RetainedMistake"),
        Some(context.id),
    )
    .expect("entry");
    let conn = db.lock().expect("lock");
    let context_uuid = context_uuid(&conn, context.id);
    let correction_uuid = correction_row_uuid(&conn, context.id, entry.id);
    let delete = super::protocol::SyncOp {
        table: "contexts".to_string(),
        row_uuid: context_uuid,
        op: "delete".to_string(),
        ts_ms: sync_store::now_ms() + 10_000,
        origin: uuid("remote-context-delete"),
        origin_seq: 1,
        payload: None,
    };
    engine::apply_ops(&conn, &[delete]).expect("delete Context");
    let everywhere_id: i64 = conn
        .query_row("SELECT id FROM contexts WHERE is_everywhere = 1", [], |r| {
            r.get(0)
        })
        .expect("Everywhere");
    let moved_context_id: i64 = conn
        .query_row(
            "SELECT context_id FROM dictionary_corrections WHERE uuid = ?1",
            rusqlite::params![correction_uuid],
            |r| r.get(0),
        )
        .expect("moved correction");
    assert_eq!(moved_context_id, everywhere_id);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM contexts WHERE name = 'Temporary'"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM dictionary_contexts
              WHERE context_id = (SELECT id FROM contexts WHERE is_everywhere = 1)"
        ),
        1
    );
}

#[test]
fn content_syncs_both_directions_without_duplicates() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));

    let entry_a =
        db::insert_dictionary_entry_returning(&a, "Groq", Some("Grock"), None).expect("a entry");
    let _snippet_b =
        db::insert_snippet_returning(&b, "addr", "123 Main St", "", None).expect("b snippet");
    let ctx_a =
        db::insert_context_returning(&a, "Work", None, None, None, None, false).expect("ctx");
    db::assign_context_target(&a, ctx_a.id, "code.exe").expect("target");
    // Scope A's dictionary entry to A's context.
    db::set_dictionary_context_assignment(&a, ctx_a.id, entry_a.id, true).expect("assign");

    exchange(&a, &b);

    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 1);
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM snippets"), 1);
    assert_eq!(
        count(
            &conn_b,
            "SELECT COUNT(*) FROM contexts WHERE is_everywhere = 0"
        ),
        1
    );
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM context_targets"), 1);
    assert_eq!(
        count(&conn_b, "SELECT COUNT(*) FROM dictionary_contexts"),
        2,
        "the canonical row remains assigned to Everywhere and is additionally shared with Work"
    );
    drop(conn_b);

    // And back: B's snippet reaches A.
    let conn_a = a.lock().expect("lock");
    assert_eq!(count(&conn_a, "SELECT COUNT(*) FROM snippets"), 1);
    let snippet_on_a: String = conn_a
        .query_row("SELECT expansion FROM snippets", [], |r| r.get(0))
        .expect("snippet");
    assert_eq!(snippet_on_a, "123 Main St");
    drop(conn_a);

    // Idempotence: exchanging again must not create duplicates.
    exchange(&a, &b);
    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 1);
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM snippets"), 1);
}

#[test]
fn edits_and_deletes_propagate() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));

    // A creates an entry; it reaches B.
    let entry =
        db::insert_dictionary_entry_returning(&a, "Groq", Some("Grock"), None).expect("entry");
    exchange(&a, &b);
    {
        let conn = b.lock().expect("lock");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 1);
    }

    // B edits it offline; the edit reaches A.
    let local_id = {
        let conn = b.lock().expect("lock");
        conn.query_row("SELECT id FROM dictionary", [], |r| r.get(0))
            .expect("id")
    };
    db::update_dictionary_entry(&b, local_id, "Groq", Some("Groqck")).expect("edit");
    exchange(&a, &b);
    {
        let conn = a.lock().expect("lock");
        let mistake: String = conn
            .query_row(
                "SELECT c.mistake
                   FROM dictionary_corrections c
                  WHERE c.dictionary_id = ?1
                    AND c.context_id = (SELECT id FROM contexts WHERE is_everywhere = 1)",
                rusqlite::params![entry.id],
                |r| r.get(0),
            )
            .expect("mistake");
        assert_eq!(mistake, "Groqck", "B's edit reached A");
    }

    // B deletes it; the delete reaches A and nothing resurrects.
    db::delete_dictionary_entry(&b, local_id).expect("delete on b");
    exchange(&a, &b);
    let conn_a = a.lock().expect("lock");
    assert_eq!(
        count(&conn_a, "SELECT COUNT(*) FROM dictionary"),
        0,
        "delete propagated"
    );
    drop(conn_a);
    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 0);
}

#[test]
fn delete_loses_to_a_newer_edit() {
    let db = test_db(&uuid("aaaa"));
    let upsert = test_dictionary_op("term-x", 5_000);
    {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[upsert.clone()]).expect("apply");
        let delete = super::protocol::SyncOp {
            table: "dictionary".to_string(),
            row_uuid: upsert.row_uuid.clone(),
            op: "delete".to_string(),
            ts_ms: 1_000, // older than the upsert
            origin: uuid("origin"),
            origin_seq: 1_000,
            payload: None,
        };
        let summary = engine::apply_ops(&conn, &[delete]).expect("apply delete");
        assert_eq!(
            summary.skipped, 1,
            "older delete must lose to the newer edit"
        );
    }
    let conn = db.lock().expect("lock");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 1);
}

#[test]
fn natural_key_conflict_converges_to_one_row() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));

    // Both devices independently create the same dictionary term.
    db::insert_dictionary_entry_returning(&a, "Verenu", None, None).expect("a row");
    db::insert_dictionary_entry_returning(&b, "Verenu", None, None).expect("b row");
    exchange(&a, &b);

    for (name, db_handle) in [("a", &a), ("b", &b)] {
        let conn = db_handle.lock().expect("lock");
        let n = count(&conn, "SELECT COUNT(*) FROM dictionary");
        assert_eq!(n, 1, "device {name} should have exactly one Verenu entry");
        let uuids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT uuid FROM dictionary").expect("q");
            let rows = stmt
                .query_map([], |r| r.get(0))
                .expect("map")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("rows");
            rows
        };
        drop(conn);
        // Same surviving uuid on both devices.
        let other = if name == "a" { &b } else { &a };
        let conn_other = other.lock().expect("lock");
        let other_uuids: Vec<String> = {
            let mut stmt = conn_other
                .prepare("SELECT uuid FROM dictionary")
                .expect("q");
            stmt.query_map([], |r| r.get(0))
                .expect("map")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("rows")
        };
        assert_eq!(uuids, other_uuids, "same row must win on both devices");
    }
}

#[test]
fn older_op_never_overwrites_newer() {
    let db = test_db(&uuid("aaaa"));
    let newer = test_dictionary_op("term-new", 5_000);
    {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[newer.clone()]).expect("apply newer");
        // The same row re-delivered with an older stamp must be skipped.
        let mut stale = newer.clone();
        stale.ts_ms = 999;
        stale.origin_seq = 999;
        let summary = engine::apply_ops(&conn, &[stale]).expect("apply stale");
        assert_eq!(summary.skipped, 1, "stale op should be skipped");
        assert_eq!(summary.applied, 0);
    }
    let conn = db.lock().expect("lock");
    let term: String = conn
        .query_row("SELECT term FROM dictionary", [], |r| r.get(0))
        .expect("term");
    assert_eq!(term, "term-new");
}

// ---- full session over an in-memory stream ----

#[tokio::test]
async fn session_snapshot_seeds_a_new_device() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));

    for i in 0..5 {
        db::insert_dictionary_entry_returning(&a, &format!("term{i}"), None, None).expect("entry");
    }
    db::insert_snippet_returning(&a, "sig", "Best regards", "", None).expect("snippet");
    let ctx =
        db::insert_context_returning(&a, "Meetings", None, None, None, None, false).expect("ctx");
    db::assign_context_website(&a, ctx.id, "meet.example.com").expect("site");

    let host_a = TestHost::new(&uuid("aaaa"));
    let host_b = TestHost::new(&uuid("bbbb"));
    let (mut side_a, mut side_b) = tokio::io::duplex(1024 * 1024);

    pair_test_dbs(&a, &b, &host_a.uuid, &host_b.uuid);
    let peer_a = peer_of(&host_b.uuid);
    let peer_b = peer_of(&host_a.uuid);
    let (session_a, session_b) = tokio::join!(
        engine::run_session(&a, &host_a, &mut side_a, true, &peer_a),
        engine::run_session(&b, &host_b, &mut side_b, false, &peer_b),
    );
    session_a.expect("session a");
    session_b.expect("session b");

    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 5);
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM snippets"), 1);
    assert_eq!(
        count(&conn_b, "SELECT COUNT(*) FROM context_website_targets"),
        1
    );
    // B recorded the snapshot position, and A no longer owes B a snapshot.
    let recv_cursor = sync_store::peer_recv_cursor(&conn_b, &host_a.uuid).expect("pos");
    assert!(recv_cursor > 0, "B should have recorded A's log position");
    drop(conn_b);
    let conn_a = a.lock().expect("lock");
    let (_, needs_snapshot) = sync_store::peer_send_position(&conn_a, &host_b.uuid).expect("pos");
    assert!(!needs_snapshot, "A should have cleared the snapshot flag");
}

#[tokio::test]
async fn responder_continues_after_dispatcher_consumes_hello() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let host_a = TestHost::new(&uuid("aaaa"));
    let host_b = TestHost::new(&uuid("bbbb"));
    pair_test_dbs(&a, &b, &host_a.uuid, &host_b.uuid);

    let (mut side_a, mut side_b) = tokio::io::duplex(1024 * 1024);
    let peer_a = peer_of(&host_b.uuid);
    let peer_b = peer_of(&host_a.uuid);
    let (initiator, responder) = tokio::join!(
        engine::run_session(&a, &host_a, &mut side_a, true, &peer_a),
        async {
            let hello = match super::protocol::read_message(&mut side_b)
                .await
                .expect("dispatcher reads hello")
            {
                Message::Hello(hello) => hello,
                other => panic!("expected hello, got {other:?}"),
            };
            engine::run_session_after_hello(&b, &host_b, &mut side_b, false, &peer_b, Some(hello))
                .await
        }
    );
    initiator.expect("initiator completes");
    responder.expect("responder completes");
}

#[tokio::test]
async fn session_does_not_hold_database_lock_while_building_settings() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let host_a = TestHost::new(&uuid("aaaa")).with_db_probe(&a);
    let host_b = TestHost::new(&uuid("bbbb")).with_db_probe(&b);

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        run_two_sessions(&a, &b, &host_a, &host_b),
    )
    .await
    .expect("production-style settings lookup must not deadlock");
}

#[test]
fn stale_snapshot_does_not_overwrite_newer_local_row() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let old = test_dictionary_op("snapshot-old", 1_000);
    let mut newer = old.clone();
    newer.ts_ms = 5_000;
    newer.origin_seq = 5_000;
    newer.payload = Some(json!({
        "term": "local-newer",
        "mistake": null,
        "auto_learned": false,
        "correction_count": 0,
        "confidence_tier": "manual",
        "last_seen_at": null,
        "created_at": "2026-01-01 00:00:00",
    }));

    {
        let conn = a.lock().expect("lock");
        engine::apply_ops(&conn, &[old]).expect("old row");
    }
    {
        let conn = b.lock().expect("lock");
        engine::apply_ops(&conn, &[newer]).expect("newer row");
    }

    let snapshot = {
        let conn = a.lock().expect("lock");
        let mut progress = engine::SnapshotProgress::default();
        let mut snapshot = Vec::new();
        loop {
            let (ops, _cursor, done) =
                engine::collect_ops(&conn, 0, true, 1, &mut progress).expect("collect snapshot");
            snapshot.extend(ops);
            if done {
                break;
            }
        }
        snapshot
    };
    {
        let conn = b.lock().expect("lock");
        engine::apply_ops(&conn, &snapshot).expect("apply snapshot");
        let term: String = conn
            .query_row("SELECT term FROM dictionary", [], |r| r.get(0))
            .expect("term");
        assert_eq!(term, "local-newer");
    }
}

#[test]
fn snapshot_contains_current_tombstones_for_rejoining_peers() {
    let db = test_db(&uuid("tombstone-source"));
    let entry = db::insert_dictionary_entry_returning(&db, "gone", None, None).expect("entry");
    db::delete_dictionary_entry(&db, entry.id).expect("delete");

    let conn = db.lock().expect("lock");
    let mut progress = engine::SnapshotProgress::default();
    let mut ops = Vec::new();
    loop {
        let (batch, _cursor, done) =
            engine::collect_ops(&conn, 0, true, 10, &mut progress).expect("snapshot");
        ops.extend(batch);
        if done {
            break;
        }
    }
    assert!(
        ops.iter()
            .any(|op| op.is_delete() && op.table == "dictionary"),
        "a resnapshot must carry retained deletes, not only live rows"
    );
}

#[test]
fn re_pairing_an_old_snapshot_does_not_restore_a_deleted_dictionary_entry() {
    let source_uuid = uuid("re-pair-source");
    let peer_uuid = uuid("re-pair-stale");
    let source = test_db(&source_uuid);
    let stale = test_db(&peer_uuid);
    let entry =
        db::insert_dictionary_entry_returning(&source, "synthetic-deleted-term", None, None)
            .expect("entry");
    exchange(&source, &stale);
    db::delete_dictionary_entry(&source, entry.id).expect("delete while peer is offline");

    // Advance the log timestamps beyond the previous pruning window while
    // preserving the original upsert-before-delete ordering on both devices.
    let year = 365 * 24 * 60 * 60 * 1000i64;
    for db in [&source, &stale] {
        db.lock()
            .expect("lock")
            .execute("UPDATE sync_log SET ts_ms = ts_ms - ?1", [year])
            .expect("age operations");
    }
    {
        let conn = source.lock().expect("source lock");
        sync_store::compact_log(&conn).expect("compact with no connected peer");
        sync_store::upsert_peer(
            &conn,
            &peer_uuid,
            "Repaired device",
            "synthetic-fingerprint",
        )
        .expect("re-pair stale device");
    }

    // Exercise the actual snapshot producer and apply path, stale side first.
    for (from, to) in [(&stale, &source), (&source, &stale)] {
        let mut progress = engine::SnapshotProgress::default();
        loop {
            let (ops, _, done) =
                engine::collect_ops(&from.lock().expect("sender"), 0, true, 10, &mut progress)
                    .expect("snapshot");
            engine::apply_ops(&to.lock().expect("receiver"), &ops).expect("apply snapshot");
            if done {
                break;
            }
        }
    }
    for db in [&source, &stale] {
        assert_eq!(
            count(&db.lock().expect("lock"), "SELECT COUNT(*) FROM dictionary"),
            0
        );
    }
}

#[test]
fn tombstones_survive_compaction_after_the_peer_safety_window() {
    let db = test_db(&uuid("tombstone-expiry"));
    let entry = db::insert_dictionary_entry_returning(&db, "gone", None, None).expect("entry");
    db::delete_dictionary_entry(&db, entry.id).expect("delete");

    let conn = db.lock().expect("lock");
    let row_uuid: String = conn
        .query_row(
            "SELECT row_uuid FROM sync_log
             WHERE table_name = 'dictionary' AND op = 'delete'
             ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("tombstone uuid");
    let old_ms = sync_store::now_ms().saturating_sub(365 * 24 * 60 * 60 * 1000);
    conn.execute(
        "UPDATE sync_log SET ts_ms = ?1 WHERE table_name = 'dictionary' AND row_uuid = ?2 AND op = 'delete'",
        rusqlite::params![old_ms, row_uuid],
    )
    .expect("backdate tombstone");

    sync_store::compact_log(&conn).expect("compact");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sync_log WHERE table_name = 'dictionary' AND row_uuid = ?1 AND op = 'delete'",
            rusqlite::params![row_uuid],
            |row| row.get::<_, i64>(0),
        ).expect("tombstone count"),
        1
    );
}

#[test]
fn stale_peers_are_resnapshotted_without_forgetting_their_deletes() {
    let db = test_db(&uuid("stale-source"));
    let peer = uuid("stale-peer");
    {
        let conn = db.lock().expect("lock");
        sync_store::upsert_peer(&conn, &peer, "Stale", "fp").expect("peer");
        conn.execute(
            "UPDATE sync_peers
             SET last_sync_at = datetime('now', '-31 days'),
                 needs_snapshot = 0,
                 send_cursor = 1",
            [],
        )
        .expect("backdate stale peer");
        let result = sync_store::maintain_peer_lifecycle(&conn).expect("lifecycle");
        assert_eq!(result, (1, 0));
        let (_, needs_snapshot) = sync_store::peer_send_position(&conn, &peer).expect("position");
        assert!(needs_snapshot);

        conn.execute(
            "UPDATE sync_peers SET last_sync_at = datetime('now', '-181 days')",
            [],
        )
        .expect("backdate expired peer");
        let result = sync_store::maintain_peer_lifecycle(&conn).expect("lifecycle repeat");
        assert_eq!(result, (0, 0));
        assert!(sync_store::get_peer(&conn, &peer)
            .expect("peer query")
            .is_some());
    }
}

#[test]
fn compaction_is_safe_without_a_successful_sync_session() {
    let db = test_db(&uuid("compact-source"));
    let entry = db::insert_dictionary_entry_returning(&db, "first", None, None).expect("entry");
    db::update_dictionary_entry(&db, entry.id, "second", None).expect("update");
    let conn = db.lock().expect("lock");
    let before = count(&conn, "SELECT COUNT(*) FROM sync_log");
    let removed = sync_store::compact_log(&conn).expect("compact");
    let after = count(&conn, "SELECT COUNT(*) FROM sync_log");
    assert!(before > after);
    assert_eq!(removed, before as usize - after as usize);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 1);
}

#[tokio::test]
async fn session_exchanges_changes_incrementally() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let host_a = TestHost::new(&uuid("aaaa"));
    let host_b = TestHost::new(&uuid("bbbb"));

    // First session: seed.
    db::insert_dictionary_entry_returning(&a, "one", None, None).expect("entry");
    run_two_sessions(&a, &b, &host_a, &host_b).await;
    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 1);
    drop(conn_b);

    // Offline edits on both sides, then reconnect: both changes must land.
    db::insert_dictionary_entry_returning(&a, "from-a", None, None).expect("a entry");
    db::insert_snippet_returning(&b, "from-b", "B", "", None).expect("b snippet");
    run_two_sessions(&a, &b, &host_a, &host_b).await;

    let conn_a = a.lock().expect("lock");
    assert_eq!(count(&conn_a, "SELECT COUNT(*) FROM dictionary"), 2);
    assert_eq!(count(&conn_a, "SELECT COUNT(*) FROM snippets"), 1);
    drop(conn_a);
    let conn_b = b.lock().expect("lock");
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM dictionary"), 2);
    assert_eq!(count(&conn_b, "SELECT COUNT(*) FROM snippets"), 1);
}

async fn run_two_sessions(a: &DbHandle, b: &DbHandle, host_a: &TestHost, host_b: &TestHost) {
    pair_test_dbs(a, b, &host_a.uuid, &host_b.uuid);
    let (mut side_a, mut side_b) = tokio::io::duplex(1024 * 1024);
    let peer_a = peer_of(&host_b.uuid);
    let peer_b = peer_of(&host_a.uuid);
    let (r1, r2) = tokio::join!(
        engine::run_session(a, host_a, &mut side_a, true, &peer_a),
        engine::run_session(b, host_b, &mut side_b, false, &peer_b),
    );
    r1.expect("session a");
    r2.expect("session b");
}

// ---- counters + settings ----

#[tokio::test]
async fn lifetime_counters_merge_without_double_counting() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let host_a = TestHost::new(&uuid("aaaa"));
    let host_b = TestHost::new(&uuid("bbbb"));

    // Each device dictated: A 100 words, B 40 words.
    db::insert_transcription_returning(
        &a,
        "one two three",
        "one two three",
        100,
        6_000,
        "",
        None,
        None,
    )
    .expect("transcribe a");
    db::insert_transcription_returning(&b, "hello world", "hello world", 40, 4_000, "", None, None)
        .expect("transcribe b");

    run_two_sessions(&a, &b, &host_a, &host_b).await;

    for (name, handle) in [("a", &a), ("b", &b)] {
        let conn = handle.lock().expect("lock");
        let (words, _fixes) = sync_store::effective_lifetime_totals(&conn).expect("totals");
        assert_eq!(words, 140, "device {name} merged word count");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM transcriptions"), 2);
        // History rows arrived with their original timestamps and text.
        let raw: String = conn
            .query_row(
                "SELECT raw_text FROM transcriptions WHERE words = 100",
                [],
                |r| r.get(0),
            )
            .expect("row");
        assert_eq!(raw, "one two three");
    }
}

#[tokio::test]
async fn settings_lww_applies_newer_remote_value() {
    let a = test_db(&uuid("aaaa"));
    let b = test_db(&uuid("bbbb"));
    let host_a = TestHost::new(&uuid("aaaa"));
    let host_b = TestHost::new(&uuid("bbbb"));

    // Realistic wall-clock stamps so the seeding baseline (now_ms) compares
    // the way production stamps do.
    let base = sync_store::now_ms();
    host_a
        .settings
        .lock()
        .expect("s")
        .insert("default_tone".to_string(), json!("formal"));
    host_a.stamps.lock().expect("s").insert(
        "default_tone".to_string(),
        (base + 5_000, host_a.uuid.clone()),
    );

    run_two_sessions(&a, &b, &host_a, &host_b).await;
    let applied = host_b
        .settings
        .lock()
        .expect("s")
        .get("default_tone")
        .cloned();
    assert_eq!(applied, Some(json!("formal")), "newer remote setting wins");

    // B's older local change (stamped before A's) must not overwrite it.
    host_b
        .settings
        .lock()
        .expect("s")
        .insert("default_tone".to_string(), json!("casual"));
    host_b.stamps.lock().expect("s").insert(
        "default_tone".to_string(),
        (base + 1_000, host_b.uuid.clone()),
    );
    // Production stamps the DB when a setting changes locally (save_setting
    // hook); mirror that here so B's LWW stamp matches its value.
    {
        let conn = b.lock().expect("lock");
        sync_store::set_setting_stamp(&conn, "default_tone", base + 1_000, &host_b.uuid)
            .expect("stamp");
    }
    run_two_sessions(&a, &b, &host_a, &host_b).await;
    let applied = host_b
        .settings
        .lock()
        .expect("s")
        .get("default_tone")
        .cloned();
    assert_eq!(applied, Some(json!("formal")), "older local value loses");
}

#[test]
fn newer_remote_settings_are_applied_in_one_batch() {
    let db = test_db(&uuid("batch"));
    let host = TestHost::new(&uuid("batch-host"));
    let now = sync_store::now_ms();
    let records = vec![
        super::protocol::SettingRecord {
            key: "default_tone".to_string(),
            value: json!("formal"),
            ts_ms: now + 1,
            origin: uuid("remote-a"),
        },
        super::protocol::SettingRecord {
            key: "cleanup_intensity".to_string(),
            value: json!("high"),
            ts_ms: now + 2,
            origin: uuid("remote-a"),
        },
    ];
    let applied = {
        let conn = db.lock().expect("lock");
        engine::apply_settings_exchange(&conn, &host, &records).expect("settings")
    };
    assert_eq!(applied, 2);
    assert_eq!(
        host.setting_batch_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "one settings document write should cover the exchange"
    );
}

#[test]
fn failed_remote_settings_batch_does_not_record_stamps() {
    let db = test_db(&uuid("failed-batch"));
    let host = TestHost::new(&uuid("failed-batch-host")).with_failing_setting_batch();
    let record = super::protocol::SettingRecord {
        key: "default_tone".to_string(),
        value: json!("formal"),
        ts_ms: sync_store::now_ms() + 1,
        origin: uuid("remote-failure"),
    };

    let applied = {
        let conn = db.lock().expect("lock");
        engine::apply_settings_exchange(&conn, &host, &[record]).expect("exchange remains usable")
    };
    assert_eq!(applied, 0);
    assert!(host.settings.lock().expect("settings").is_empty());
    let conn = db.lock().expect("lock");
    assert!(
        sync_store::get_setting_stamp(&conn, "default_tone")
            .expect("stamp lookup")
            .is_none(),
        "a failed persistence batch must not be stamped"
    );
}

#[test]
fn syncable_settings_exclude_device_local_keys() {
    // Device-specific and secret keys must never appear in the allowlist.
    for key in [
        crate::data::store::KEY_GROQ,
        crate::data::store::KEY_OPENAI,
        crate::data::store::KEY_GOOGLE,
        crate::data::store::KEY_ASSEMBLYAI,
        crate::data::store::MICROPHONE_DEVICE,
        crate::data::store::MIC_GAIN,
        crate::data::store::HOTKEY,
        crate::data::store::AUTOSTART_ENABLED,
        crate::data::store::SETUP_COMPLETE,
        crate::data::store::FORCE_SETUP_ON_LAUNCH,
        crate::data::store::RUIN_ACCESSIBILITY,
        crate::data::store::NOISE_REDUCTION,
        crate::data::store::MUTE_AUDIO,
        crate::data::store::EXCLUSIVE_MIC,
        crate::data::store::PAUSE_MEDIA_DURING_DICTATION,
        crate::data::store::PLAY_START_STOP_SOUNDS,
        crate::data::store::SOUND_EFFECTS_VOLUME,
        crate::data::store::APPEARANCE_MODE,
        crate::data::store::APP_MAPPINGS,
        crate::data::store::CLIPBOARD_PHRASE,
        crate::data::store::CLIPBOARD_PHRASE_ENABLED,
        crate::data::store::BETA_UPDATES_ENABLED,
        crate::data::store::LOCAL_MODEL_MEMORY_POLICY,
        crate::data::store::HISTORY_RETENTION,
    ] {
        assert!(
            !SYNCABLE_SETTINGS.contains(&key),
            "{key} must stay device-local"
        );
    }
}

#[test]
fn remote_transcription_deletes_do_not_erase_local_history() {
    let db = test_db(&uuid("cccc"));
    db::insert_transcription_returning(
        &db,
        "history text",
        "history text",
        2,
        1_000,
        "test",
        None,
        None,
    )
    .expect("transcription");
    let transcription_uuid: String = {
        let conn = db.lock().expect("lock");
        conn.query_row("SELECT uuid FROM transcriptions LIMIT 1", [], |r| r.get(0))
            .expect("uuid")
    };

    let op = super::protocol::SyncOp {
        table: "transcriptions".to_string(),
        row_uuid: transcription_uuid,
        op: "delete".to_string(),
        ts_ms: sync_store::now_ms() + 1,
        origin: uuid("peer"),
        origin_seq: 1,
        payload: None,
    };
    let summary = {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[op]).expect("apply delete")
    };
    assert_eq!(summary.applied, 0);
    let conn = db.lock().expect("lock");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM transcriptions"), 1);
}

#[test]
fn stale_peer_resnapshot_keeps_tombstones_and_rejects_its_old_row() {
    let db = test_db(&uuid("tombstone-local"));
    let row_uuid = uuid("deleted-dictionary");
    let delete_stamp = sync_store::now_ms();
    let delete = super::protocol::SyncOp {
        table: "dictionary".to_string(),
        row_uuid: row_uuid.clone(),
        op: "delete".to_string(),
        ts_ms: delete_stamp,
        origin: uuid("local-origin"),
        origin_seq: 1,
        payload: None,
    };
    {
        let conn = db.lock().expect("lock");
        engine::apply_ops(&conn, &[delete]).expect("apply deletion");
        sync_store::upsert_peer(&conn, &uuid("stale-peer"), "old device", "fp").expect("pair peer");
        conn.execute(
            "UPDATE sync_peers SET last_sync_at = datetime('now', '-365 days')",
            [],
        )
        .expect("age peer");
        sync_store::compact_log(&conn).expect("compact");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM sync_peers"),
            1,
            "an old device stays paired and is forced through a full snapshot"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sync_log WHERE table_name = 'dictionary' AND row_uuid = ?1 AND op = 'delete'",
                [&row_uuid], |row| row.get::<_, i64>(0),
            ).expect("retained deletion stamp"),
            1,
            "compaction must retain the delete stamp"
        );
    }

    let old_upsert = super::protocol::SyncOp {
        table: "dictionary".to_string(),
        row_uuid,
        op: "upsert".to_string(),
        ts_ms: delete_stamp.saturating_sub(1),
        origin: uuid("stale-peer"),
        origin_seq: 1,
        payload: Some(json!({
            "term": "resurrect-me",
            "mistake": null,
            "auto_learned": false,
            "correction_count": 0,
            "confidence_tier": "low",
            "last_seen_at": null,
            "created_at": "2026-01-01 00:00:00"
        })),
    };
    let conn = db.lock().expect("lock");
    let summary = engine::apply_ops(&conn, &[old_upsert]).expect("apply old snapshot row");
    assert_eq!(summary.applied, 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 0);
}

fn test_context_op(
    name: &str,
    ts_ms: i64,
    executable: &str,
    platform: Option<&str>,
) -> super::protocol::SyncOp {
    super::protocol::SyncOp {
        table: "contexts".to_string(),
        row_uuid: uuid(name),
        op: "upsert".to_string(),
        ts_ms,
        origin: uuid("origin"),
        origin_seq: ts_ms,
        payload: Some(json!({
            "name": name,
            "is_everywhere": false,
            "icon": null,
            "tone": null,
            "cleanup_intensity": null,
            "color": null,
            "custom_instructions": null,
            "contextual_formatting_disabled": false,
            "pinned_at": null,
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
            "targets": [{"executable": executable, "platform": platform}],
            "websites": [],
            "dictionary_uuids": [],
            "snippet_uuids": [],
        })),
    }
}

/// A target this device already resolved for its own platform (whether by a
/// prior successful auto-match or the user manually picking the app on an
/// unresolved "?::" chip) must survive a later resync that re-derives a
/// worse or unresolved match for the same context — see the "sticky" guard
/// in `reconcile_context_children`.
#[test]
fn resolved_context_target_survives_a_later_unresolved_resync() {
    let db = test_db(&uuid("cccc"));
    let conn = db.lock().expect("lock");

    engine::apply_ops(
        &conn,
        &[test_context_op(
            "Editor Group",
            100,
            "?::antigravity ide.exe",
            None,
        )],
    )
    .expect("apply unresolved");
    let context_id: i64 = conn
        .query_row(
            "SELECT id FROM contexts WHERE name = 'Editor Group'",
            [],
            |r| r.get(0),
        )
        .expect("context id");

    // The user (or an earlier successful auto-resolve) fixes it locally.
    let my_platform = db::current_platform_tag();
    conn.execute(
        "UPDATE context_targets SET executable = 'antigravity.app', platform = ?1 WHERE context_id = ?2",
        rusqlite::params![my_platform, context_id],
    )
    .expect("simulate local fix");

    // A later resync re-derives a bad/unresolved match again for the same
    // context — e.g. an unrelated edit made on another device (rename, tone
    // change) after the fix, which still carries that device's now-stale
    // view of the target list, since a context syncs as one LWW aggregate.
    // Use a real "now" stamp so this op is unambiguously newer than the
    // manual-fix trigger's own self-logged stamp above.
    let resync_ts = sync_store::now_ms() + 1;
    let summary = engine::apply_ops(
        &conn,
        &[test_context_op(
            "Editor Group",
            resync_ts,
            "?::antigravity ide.exe",
            None,
        )],
    )
    .expect("apply resync");
    assert_eq!(
        summary.applied, 1,
        "the resync op must actually apply, not be skipped as stale"
    );

    // The sticky row must survive completely unchanged. (A stray unresolved
    // marker may additionally appear alongside it here, since this test's
    // synthetic resync payload deliberately carries a different raw source
    // string than what actually produced "antigravity.app" — in the real
    // pipeline, `resolve_context_targets_in_ops` re-derives the same source
    // deterministically and would just re-affirm the sticky row instead. The
    // one guarantee this test enforces is what the user asked for: the fix
    // is never silently deleted or overwritten by a later, worse resync.)
    let mut stmt = conn
        .prepare("SELECT executable, platform FROM context_targets WHERE context_id = ?1")
        .expect("prepare");
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params![context_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("collect");
    assert!(
        rows.contains(&(
            "antigravity.app".to_string(),
            my_platform.map(str::to_string)
        )),
        "the manually-fixed target must survive the worse resync unchanged, got {rows:?}"
    );
}

#[test]
fn context_app_matching_uses_local_name_and_rejects_weak_matches() {
    use crate::system::apps::InstalledApp;
    let apps = vec![
        InstalledApp {
            name: "Visual Studio Code".to_string(),
            exe: "code.exe".to_string(),
            developer: None,
        },
        InstalledApp {
            name: "Google Chrome".to_string(),
            exe: "chrome.exe".to_string(),
            developer: None,
        },
    ];
    let matched = super::manager::closest_installed_app("Visual Studio Code.app", &apps)
        .expect("cross-platform name match");
    assert_eq!(matched.exe, "code.exe");
    assert!(super::manager::closest_installed_app("Completely Different.app", &apps).is_none());
}

// ---- protocol framing ----

#[tokio::test]
async fn messages_roundtrip_through_the_framer() {
    use super::protocol::{read_message, send_message};
    let (mut client, mut server) = tokio::io::duplex(64 * 1024);
    let hello = Message::Hello(super::protocol::Hello {
        device_uuid: "device-1".to_string(),
        device_name: "Device One".to_string(),
        protocol: super::protocol::PROTOCOL_VERSION,
        app_version: "0.0.0".to_string(),
    });
    let (send, read) = tokio::join!(send_message(&mut client, &hello), async {
        let message: Message = read_message(&mut server).await.expect("read");
        message
    });
    send.expect("send");
    match read {
        Message::Hello(h) => {
            assert_eq!(h.device_uuid, "device-1");
            assert_eq!(h.device_name, "Device One");
        }
        other => panic!("wrong message: {other:?}"),
    }
}

#[tokio::test]
async fn oversized_and_partial_frames_are_rejected_before_decode() {
    use super::protocol::{read_message, MAX_MESSAGE_BYTES};
    use tokio::io::AsyncWriteExt;
    let (mut writer, mut reader) = tokio::io::duplex(128);
    let oversized = (MAX_MESSAGE_BYTES + 1).to_be_bytes();
    writer.write_all(&oversized).await.expect("prefix");
    drop(writer);
    assert!(read_message(&mut reader).await.is_err());

    let (mut writer, mut reader) = tokio::io::duplex(128);
    writer
        .write_all(&10u32.to_be_bytes())
        .await
        .expect("prefix");
    writer.write_all(b"short").await.expect("partial body");
    drop(writer);
    assert!(read_message(&mut reader).await.is_err());
}

#[tokio::test]
async fn stalled_tls_handshake_times_out() {
    use super::transport;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let addr = listener.local_addr().expect("addr");
    let client = tokio::spawn(async move { tokio::net::TcpStream::connect(addr).await });
    let (tcp, _) = listener.accept().await.expect("accept");
    let server_identity = super::identity::generate_for_tests();
    let acceptor = tokio_rustls::TlsAcceptor::from(
        transport::server_config(
            server_identity.cert_der().clone(),
            server_identity.tls_key(),
        )
        .expect("server config"),
    );
    let result =
        transport::accept_with_timeout(&acceptor, tcp, std::time::Duration::from_millis(20)).await;
    assert!(
        result.is_err(),
        "a client that sends no ClientHello must time out"
    );
    let _ = client.await;
}

// ---- pairing handshake ----

#[tokio::test]
async fn pairing_succeeds_with_matching_code_and_fails_with_wrong_code() {
    use super::pairing::{self, IdentityExchange};
    use super::protocol::{read_message, send_message};

    let identity_a = IdentityExchange {
        device_uuid: uuid("aaaa"),
        device_name: "A".to_string(),
        cert_der: vec![1, 2, 3],
    };
    let identity_b = IdentityExchange {
        device_uuid: uuid("bbbb"),
        device_name: "B".to_string(),
        cert_der: vec![4, 5, 6],
    };
    let identity_b_for_second = identity_b.clone();
    let code = pairing::generate_pairing_code();
    assert_eq!(code.len(), 6);

    let (mut a, mut b) = tokio::io::duplex(64 * 1024);
    let (state_a, msg_a) = pairing::initiator_start(&code);
    let code_b = code.clone();
    let expected_uuid = identity_b.device_uuid.clone();
    let expected_uuid_for_initiator = expected_uuid.clone();
    let id_a = identity_a.clone();
    let (initiator, responder) = tokio::join!(
        async move {
            send_message(
                &mut a,
                &Message::PairRequest {
                    device_uuid: id_a.device_uuid.clone(),
                    device_name: id_a.device_name.clone(),
                    protocol: super::protocol::PROTOCOL_VERSION,
                    spake_msg: msg_a,
                },
            )
            .await
            .expect("send request");
            let responder_msg = match read_message(&mut a).await.expect("accept") {
                Message::PairAccept { spake_msg } => spake_msg,
                other => panic!("expected accept, got {other:?}"),
            };
            let cipher = pairing::initiator_cipher(state_a, &responder_msg).expect("cipher");
            pairing::initiator_exchange(&mut a, &cipher, &id_a, &expected_uuid_for_initiator).await
        },
        async move {
            let request = read_message(&mut b).await.expect("request");
            let Message::PairRequest {
                spake_msg,
                device_uuid,
                ..
            } = request
            else {
                panic!("expected pair request");
            };
            let (msg_b, cipher) = pairing::responder_start(&code_b, &spake_msg).expect("start");
            pairing::responder_exchange(&mut b, &cipher, msg_b, &identity_b, &device_uuid).await
        },
    );
    let peer_from_a = initiator.expect("initiator outcome");
    responder.expect("responder outcome");
    assert_eq!(peer_from_a.device_uuid, expected_uuid);
    assert_eq!(peer_from_a.cert_der, vec![4, 5, 6]);

    // Wrong code: both sides must fail with a friendly error, not panic.
    let identity_a2 = IdentityExchange {
        cert_der: vec![7],
        ..identity_a
    };
    let identity_b2 = IdentityExchange {
        cert_der: vec![8],
        ..identity_b_for_second
    };
    let expected_uuid2 = identity_b2.device_uuid.clone();
    let (mut a2, mut b2) = tokio::io::duplex(64 * 1024);
    let (state_a2, msg_a2) = pairing::initiator_start("111111");
    let (initiator, responder) = tokio::join!(
        async move {
            send_message(
                &mut a2,
                &Message::PairRequest {
                    device_uuid: identity_a2.device_uuid.clone(),
                    device_name: identity_a2.device_name.clone(),
                    protocol: super::protocol::PROTOCOL_VERSION,
                    spake_msg: msg_a2,
                },
            )
            .await
            .expect("send");
            let responder_msg = match read_message(&mut a2).await.expect("accept") {
                Message::PairAccept { spake_msg } => spake_msg,
                other => panic!("expected accept, got {other:?}"),
            };
            let cipher = pairing::initiator_cipher(state_a2, &responder_msg).expect("cipher");
            pairing::initiator_exchange(&mut a2, &cipher, &identity_a2, &expected_uuid2).await
        },
        async move {
            let request = read_message(&mut b2).await.expect("request");
            let Message::PairRequest {
                spake_msg,
                device_uuid,
                ..
            } = request
            else {
                panic!("expected pair request");
            };
            // Responder uses a DIFFERENT code.
            let (msg_b, cipher) = pairing::responder_start("999999", &spake_msg).expect("start");
            pairing::responder_exchange(&mut b2, &cipher, msg_b, &identity_b2, &device_uuid).await
        },
    );
    assert!(initiator.is_err(), "initiator must fail on code mismatch");
    assert!(responder.is_err(), "responder must fail on code mismatch");
}

/// Registers the two test databases as paired peers of each other, mirroring
/// what `complete_pairing` does in production (the session driver reads the
/// peer row for cursor state, and the manager only dials paired peers).
fn pair_test_dbs(a: &DbHandle, b: &DbHandle, a_uuid: &str, b_uuid: &str) {
    {
        let conn = a.lock().expect("lock");
        sync_store::upsert_peer(&conn, b_uuid, "B", "fp-b").expect("upsert a->b");
    }
    {
        let conn = b.lock().expect("lock");
        sync_store::upsert_peer(&conn, a_uuid, "A", "fp-a").expect("upsert b->a");
    }
}
