use super::super::{engine, identity, pairing, protocol, store, transport};
use super::{count, TestHost};
use crate::{data::db, DbHandle};
use anyhow::{anyhow, Result};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

struct Device {
    dir: PathBuf,
    db: DbHandle,
    host: TestHost,
    cert: CertificateDer<'static>,
    key: Vec<u8>,
}

#[test]
fn vocabulary_membership_records_add_and_observed_remove() {
    let device = Device::new();
    let entry = db::insert_dictionary_entry_returning(&device.db, "DotFixture", None, None).unwrap();
    {
        let conn = device.db.lock().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_dots WHERE removed=0 AND dictionary_term='DotFixture'"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_log WHERE table_name='vocabulary_membership_dots'"), 1);
    }
    db::set_dictionary_context_assignment(&device.db, db::EVERYWHERE_CONTEXT_ID, entry.id, false).unwrap();
    let conn = device.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_dots WHERE removed=1 AND dictionary_term='DotFixture'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_log WHERE table_name='vocabulary_membership_dots'"), 2);
}

#[test]
fn correction_versions_retain_deleted_mapping_history() {
    let device = Device::new();
    let entry = db::insert_dictionary_entry_returning(&device.db, "VersionedFixture", Some("versioned typo"), None).unwrap();
    {
        let conn = device.db.lock().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_versions WHERE retired=0 AND mistake='versioned typo'"), 1);
    }
    db::delete_dictionary_entry(&device.db, entry.id).unwrap();
    let conn = device.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_versions WHERE retired=1 AND mistake='versioned typo'"), 1);
}

#[test]
fn historical_correction_decision_survives_later_dictionary_alias() {
    let source = Device::new();
    let first = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
    let second = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1";
    {
        let conn = source.db.lock().unwrap();
        for (version, correction, dictionary, created) in [
            ("11111111-1111-4111-8111-111111111111", "33333333-3333-4333-8333-333333333333", first, "2020-01-01"),
            ("22222222-2222-4222-8222-222222222222", "44444444-4444-4444-8444-444444444444", second, "2020-01-02"),
        ] {
            conn.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at) VALUES (?1,?2,'everywhere-0000-0000-0000-000000000001',?3,'Alias Fixture','alias typo',0,?4)", rusqlite::params![version,correction,dictionary,created]).unwrap();
        }
        db::record_correction_history_decisions(&conn).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
        conn.execute("INSERT INTO vocabulary_identity_links(link_id,left_uuid,right_uuid) VALUES (?1,?2,?3)", rusqlite::params![format!("{first}:{second}"),first,second]).unwrap();
        conn.execute("UPDATE sync_state SET applying=1", []).unwrap();
        conn.execute("INSERT INTO dictionary(uuid,term) VALUES (?1,'Alias Fixture')", [first]).unwrap();
        let id = conn.last_insert_rowid();
        conn.execute("INSERT INTO dictionary_contexts(context_id,dictionary_id) VALUES (?1,?2)", rusqlite::params![db::EVERYWHERE_CONTEXT_ID,id]).unwrap();
        conn.execute("INSERT INTO dictionary_corrections(uuid,context_id,dictionary_id,mistake) VALUES ('33333333-3333-4333-8333-333333333333',?1,?2,'alias typo')", rusqlite::params![db::EVERYWHERE_CONTEXT_ID,id]).unwrap();
        conn.execute("UPDATE sync_state SET applying=0", []).unwrap();
    }
    assert!(db::query_effective_dictionary_for_context(&source.db, db::EVERYWHERE_CONTEXT_ID).unwrap().iter().any(|entry| entry.term == "Alias Fixture" && entry.mistake.as_deref() == Some("alias typo")), "linking two identities must not hide their remaining correction");
    let ops = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let target = Device::new();
    engine::apply_ops(&target.db.lock().unwrap(), &ops).expect("a later alias must not invalidate a historical decision");
    assert_eq!(count(&target.db.lock().unwrap(), "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
    let other = Device::new();
    let before_link = ops.iter().filter(|op| op.table != "vocabulary_identity_links").cloned().collect::<Vec<_>>();
    let links = ops.iter().filter(|op| op.table == "vocabulary_identity_links").cloned().collect::<Vec<_>>();
    engine::apply_ops(&other.db.lock().unwrap(), &before_link).unwrap();
    engine::apply_ops(&other.db.lock().unwrap(), &links).unwrap();
    engine::apply_ops(&other.db.lock().unwrap(), &ops).unwrap();
    assert_eq!(count(&other.db.lock().unwrap(), "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
}

#[test]
fn correction_conflicts_recognize_everywhere_aliases() {
    let device = Device::new();
    let conn = device.db.lock().unwrap();
    let alias = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2";
    conn.execute("INSERT INTO sync_everywhere_aliases(uuid) VALUES (?1)", [alias]).unwrap();
    for (version, correction, context, dictionary, created) in [
        ("11111111-1111-4111-8111-111111111111", "33333333-3333-4333-8333-333333333333", "everywhere-0000-0000-0000-000000000001", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1", "2020-01-01"),
        ("22222222-2222-4222-8222-222222222222", "44444444-4444-4444-8444-444444444444", alias, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1", "2020-01-02"),
    ] {
        conn.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at) VALUES (?1,?2,?3,?4,'Alias Fixture','alias typo',0,?5)", rusqlite::params![version,correction,context,dictionary,created]).unwrap();
    }
    db::record_correction_history_decisions(&conn).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
}

#[test]
fn correction_decision_accepts_backup_alias_retargeted_to_everywhere() {
    let source = Device::new();
    let target = Device::new();
    let local_everywhere = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa3";
    let alias = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb3";
    for device in [&source, &target] {
        let conn = device.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid=?1 WHERE is_everywhere=1", [local_everywhere]).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid) VALUES (?1,?2)", rusqlite::params![alias,local_everywhere]).unwrap();
    }
    {
        let conn = source.db.lock().unwrap();
        for (version, correction, context, dictionary, created) in [
            ("11111111-1111-4111-8111-111111111111", "33333333-3333-4333-8333-333333333333", "everywhere-0000-0000-0000-000000000001", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1", "2020-01-01"),
            ("22222222-2222-4222-8222-222222222222", "44444444-4444-4444-8444-444444444444", alias, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1", "2020-01-02"),
        ] {
            conn.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at) VALUES (?1,?2,?3,?4,'Backup Alias Fixture','backup alias typo',0,?5)", rusqlite::params![version,correction,context,dictionary,created]).unwrap();
        }
        db::record_correction_history_decisions(&conn).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
    }
    let ops = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &ops).expect("a backup alias retargeted to Everywhere must validate");
    assert_eq!(count(&target.db.lock().unwrap(), "SELECT COUNT(*) FROM vocabulary_correction_decisions"), 1);
}

#[test]
fn stale_offline_dictionary_edit_is_retained_and_flagged_after_deletion() {
    let source = Device::new();
    let target = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "OfflineReviewFixture", Some("offline review typo"), None).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    let mut offline_edit = snapshot.iter().find(|op| op.table == "dictionary").unwrap().clone();
    offline_edit.origin = "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into();
    offline_edit.origin_seq = 1;
    offline_edit.payload.as_mut().unwrap()["replacement_confidence"] = json!("high");
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let changes = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &changes).unwrap();
    engine::apply_ops(&target.db.lock().unwrap(), &[offline_edit]).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 1);
    let payload: String = conn.query_row("SELECT payload FROM vocabulary_review_payloads WHERE kind='dictionary'", [], |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&payload).unwrap()["replacement_confidence"], "high");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='OfflineReviewFixture'"), 1);
    let (context_uuid, dictionary_uuid): (String, String) = conn.query_row("SELECT context_uuid,dictionary_uuid FROM vocabulary_membership_reviews WHERE dictionary_term='OfflineReviewFixture'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    drop(conn);
    assert!(db::acknowledge_vocabulary_review(&target.db, &context_uuid, &dictionary_uuid).is_err(), "an archived edit must not be dismissed before it is resolved");
}

#[test]
fn clock_skewed_offline_edit_after_deletion_is_reviewed_before_activation() {
    let source = Device::new();
    let target = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "SkewReviewFixture", None, None).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    let mut offline = snapshot.iter().find(|op| op.table == "dictionary").unwrap().clone();
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let changes = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &changes).unwrap();
    offline.origin = "dddddddd-dddd-4ddd-8ddd-dddddddddddd".into();
    offline.origin_seq = 1;
    offline.ts_ms = store::now_ms() + 60_000;
    offline.payload.as_mut().unwrap()["replacement_confidence"] = json!("high");
    engine::apply_ops(&target.db.lock().unwrap(), &[offline]).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary WHERE term='SkewReviewFixture'"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='SkewReviewFixture'"), 1);
}

#[test]
fn explicit_restore_survives_deletion_delivery_in_either_order() {
    let source = Device::new();
    let delete_first = Device::new();
    let restore_first = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "RestoreBasisFixture", None, None).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    engine::apply_ops(&delete_first.db.lock().unwrap(), &snapshot).unwrap();
    engine::apply_ops(&restore_first.db.lock().unwrap(), &snapshot).unwrap();
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let (deletions, delete_cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    let deletion = deletions.iter().find(|op| op.table == "dictionary" && op.is_delete()).unwrap().clone();
    {
        let conn = source.db.lock().unwrap();
        conn.execute("INSERT INTO dictionary(uuid,term) VALUES (?1,'RestoreBasisFixture')", [&deletion.row_uuid]).unwrap();
    }
    let restored = engine::collect_ops(&source.db.lock().unwrap(), delete_cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let restore = restored.iter().find(|op| op.table == "dictionary" && !op.is_delete()).unwrap().clone();
    assert_eq!(restore.payload.as_ref().unwrap()["restore_basis"]["origin"], deletion.origin);
    engine::apply_ops(&delete_first.db.lock().unwrap(), &[deletion.clone(), restore.clone()]).unwrap();
    engine::apply_ops(&restore_first.db.lock().unwrap(), &[restore]).unwrap();
    let mut delayed_delete = deletion;
    delayed_delete.ts_ms = store::now_ms() + 60_000;
    engine::apply_ops(&restore_first.db.lock().unwrap(), &[delayed_delete]).unwrap();
    for device in [&delete_first, &restore_first] {
        let conn = device.db.lock().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary WHERE term='RestoreBasisFixture'"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='RestoreBasisFixture'"), 0);
    }
    let mut offline = snapshot.iter().find(|op| op.table == "dictionary").unwrap().clone();
    offline.origin = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".into();
    offline.origin_seq = 1;
    offline.ts_ms = store::now_ms() + 120_000;
    offline.payload.as_mut().unwrap()["replacement_confidence"] = json!("high");
    let conn = delete_first.db.lock().unwrap();
    engine::apply_ops(&conn, &[offline]).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary WHERE term='RestoreBasisFixture' AND replacement_confidence='default'"), 1, "an offline edit must not silently replace an explicit restore");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='RestoreBasisFixture'"), 1);
}

#[test]
fn deletion_preserves_preupgrade_correction_for_review() {
    let source = Device::new();
    let target = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "LegacyReviewFixture", Some("legacy review typo"), None).unwrap();
    source.db.lock().unwrap().execute("UPDATE vocabulary_correction_versions SET legacy=1 WHERE dictionary_term='LegacyReviewFixture'", []).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    {
        let conn = target.db.lock().unwrap();
        conn.execute("UPDATE dictionary_corrections SET correction_count=7, confidence_tier='high' WHERE mistake='legacy review typo'", []).unwrap();
    }
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let changes = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &changes).unwrap();
    let extra = db::insert_context_returning(&target.db, "Unrelated Review Context", None, None, None, None, false).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='LegacyReviewFixture'"), 1);
    let payload: String = conn.query_row("SELECT payload FROM vocabulary_review_payloads WHERE kind='correction'", [], |row| row.get(0)).unwrap();
    let correction: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(correction["correction_count"], 7);
    assert_eq!(correction["confidence_tier"], "high");
    let context_uuid: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [db::EVERYWHERE_CONTEXT_ID], |r| r.get(0)).unwrap();
    let dictionary_uuid: String = conn.query_row("SELECT dictionary_uuid FROM vocabulary_membership_reviews WHERE dictionary_term='LegacyReviewFixture' LIMIT 1", [], |r| r.get(0)).unwrap();
    conn.execute("INSERT INTO vocabulary_membership_reviews(context_uuid,dictionary_uuid,dictionary_term,reason) SELECT uuid,?1,'LegacyReviewFixture','offline_deletion' FROM contexts WHERE id=?2", rusqlite::params![dictionary_uuid,extra.id]).unwrap();
    let candidates = conn.prepare("SELECT kind,row_uuid,origin,origin_seq FROM vocabulary_review_payloads ORDER BY kind DESC").unwrap().query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    drop(conn);
    for (kind, row_uuid, origin, origin_seq) in candidates {
        db::restore_vocabulary_review_payload(&target.db, &context_uuid, &kind, &row_uuid, &origin, origin_seq).unwrap();
    }
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary WHERE term='LegacyReviewFixture'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary_corrections WHERE mistake='legacy review typo' AND correction_count=7 AND confidence_tier='high'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_review_payloads"), 2, "restoration must retain the saved copies until the user discards them");
    let extra_assignments: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_contexts dc JOIN dictionary d ON d.id=dc.dictionary_id WHERE dc.context_id=?1 AND d.term='LegacyReviewFixture'", [extra.id], |r| r.get(0)).unwrap();
    assert_eq!(extra_assignments, 0, "restoring one Context must not activate another review Context");
    drop(conn);
    let effective = db::query_effective_dictionary_for_context(&target.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert!(effective.iter().any(|entry| entry.term == "LegacyReviewFixture" && entry.mistake.as_deref() == Some("legacy review typo")), "the restored correction must be available to the pipeline");
}

#[test]
fn review_restores_correction_after_dictionary_identity_merge() {
    let device = Device::new();
    let entry = db::insert_dictionary_entry_returning(&device.db, "AliasReviewFixture", None, None).unwrap();
    let old_uuid = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
    let correction_uuid = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1";
    {
        let conn = device.db.lock().unwrap();
        let current: String = conn.query_row("SELECT uuid FROM dictionary WHERE id=?1", [entry.id], |r| r.get(0)).unwrap();
        let context: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [db::EVERYWHERE_CONTEXT_ID], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO vocabulary_identity_links(link_id,left_uuid,right_uuid) VALUES ('alias-review',?1,?2)", rusqlite::params![old_uuid,current]).unwrap();
        conn.execute("INSERT INTO vocabulary_membership_reviews(context_uuid,dictionary_uuid,dictionary_term,reason) VALUES (?1,?2,'AliasReviewFixture','offline_deletion')", rusqlite::params![context,old_uuid]).unwrap();
        let payload = json!({"context_uuid":context,"dictionary_uuid":old_uuid,"dictionary_term":"AliasReviewFixture","mistake":"alias review typo","auto_learned":false,"correction_count":3,"confidence_tier":"high","created_at":"2020-01-01 00:00:00"}).to_string();
        conn.execute("INSERT INTO vocabulary_review_payloads(kind,row_uuid,dictionary_uuid,payload,ts_ms,origin,origin_seq) VALUES ('correction',?1,?2,?3,1,'fixture',1)", rusqlite::params![correction_uuid,old_uuid,payload]).unwrap();
    }
    let context: String = device.db.lock().unwrap().query_row("SELECT uuid FROM contexts WHERE id=?1", [db::EVERYWHERE_CONTEXT_ID], |r| r.get(0)).unwrap();
    db::restore_vocabulary_review_payload(&device.db, &context, "correction", correction_uuid, "fixture", 1).unwrap();
    let entries = db::query_effective_dictionary_for_context(&device.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert!(entries.iter().any(|row| row.term == "AliasReviewFixture" && row.mistake.as_deref() == Some("alias review typo")));
}

#[test]
fn concurrent_offline_edits_are_each_retained_for_review() {
    let source = Device::new();
    let target = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "MultipleReviewFixture", None, None).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let changes = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &changes).unwrap();
    let mut first = snapshot.iter().find(|op| op.table == "dictionary").unwrap().clone();
    first.origin = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into();
    first.origin_seq = 1;
    first.payload.as_mut().unwrap()["replacement_confidence"] = json!("low");
    let mut second = first.clone();
    second.origin = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into();
    second.payload.as_mut().unwrap()["replacement_confidence"] = json!("high");
    engine::apply_ops(&target.db.lock().unwrap(), &[first, second]).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 2);
    let (row_uuid, origin, origin_seq): (String,String,i64) = conn.query_row("SELECT row_uuid,origin,origin_seq FROM vocabulary_review_payloads WHERE kind='dictionary' ORDER BY origin LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let (relayed, cursor, _) = engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    drop(conn);
    let third = Device::new();
    engine::apply_ops(&third.db.lock().unwrap(), &relayed).unwrap();
    assert_eq!(count(&third.db.lock().unwrap(), "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 2, "a snapshot must retain all ambiguous edits for another device");
    db::discard_vocabulary_review_payload(&target.db, "dictionary", &row_uuid, &origin, origin_seq).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 1);
    store::compact_log(&conn).unwrap();
    let changes = engine::collect_ops(&conn, cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let compacted = engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    drop(conn);
    let third_conn = third.db.lock().unwrap();
    engine::apply_ops(&third_conn, &changes).unwrap();
    engine::apply_ops(&third_conn, &relayed).unwrap();
    assert_eq!(count(&third_conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 1, "discard must survive delayed snapshot replay");
    let fourth = Device::new();
    let fourth_conn = fourth.db.lock().unwrap();
    engine::apply_ops(&fourth_conn, &compacted).unwrap();
    engine::apply_ops(&fourth_conn, &relayed).unwrap();
    assert_eq!(count(&fourth_conn, "SELECT COUNT(*) FROM vocabulary_review_payloads WHERE kind='dictionary'"), 1, "a compacted snapshot must retain the discard");
}

#[test]
fn moving_a_correction_creates_a_new_context_version() {
    let device = Device::new();
    let context = db::insert_context_returning(&device.db, "Version Move Fixture", None, None, None, None, false).unwrap();
    let entry = db::insert_dictionary_entry_returning(&device.db, "MoveVersionFixture", Some("move version typo"), None).unwrap();
    db::move_dictionary_entry_to_context(&device.db, entry.id, db::EVERYWHERE_CONTEXT_ID, context.id).unwrap();
    let conn = device.db.lock().unwrap();
    let context_uuid: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [context.id], |row| row.get(0)).unwrap();
    let active: i64 = conn.query_row("SELECT COUNT(*) FROM vocabulary_correction_versions WHERE context_uuid=?1 AND retired=0 AND mistake='move version typo'", [context_uuid], |row| row.get(0)).unwrap();
    assert_eq!(active, 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_versions WHERE retired=1 AND mistake='move version typo'"), 1);
}

#[test]
fn correction_version_snapshot_and_retirement_survive_replay() {
    let source = Device::new();
    let target = Device::new();
    let entry = db::insert_dictionary_entry_returning(&source.db, "VersionReplayFixture", Some("replay typo"), None).unwrap();
    let (snapshot, cursor, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
    assert!(snapshot.iter().any(|op| op.table == "vocabulary_correction_versions"));
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    db::delete_dictionary_entry(&source.db, entry.id).unwrap();
    let changes = engine::collect_ops(&source.db.lock().unwrap(), cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &changes).unwrap();
    engine::apply_ops(&target.db.lock().unwrap(), &snapshot).unwrap();
    let conn = target.db.lock().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_versions WHERE mistake='replay typo' AND retired=1"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_correction_versions WHERE mistake='replay typo' AND retired=0"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews"), 0, "a replay of an already observed edit must not create an ambiguous review");
}

#[test]
fn defeated_offline_correction_does_not_block_post_expiration_replacement() {
    let device = Device::new();
    let first = db::insert_dictionary_entry_returning(&device.db, "FirstHistoryFixture", None, None).unwrap();
    let second = db::insert_dictionary_entry_returning(&device.db, "SecondHistoryFixture", None, None).unwrap();
    let third = db::insert_dictionary_entry_returning(&device.db, "ThirdHistoryFixture", None, None).unwrap();
    let conn = device.db.lock().unwrap();
    let first_uuid: String = conn.query_row("SELECT uuid FROM dictionary WHERE id=?1", [first.id], |row| row.get(0)).unwrap();
    let request = "11111111-1111-4111-8111-111111111111";
    conn.execute("UPDATE dictionary SET created_at='2020-01-01' WHERE id=?1", [first.id]).unwrap();
    conn.execute("INSERT INTO vocabulary_expiration_marks(item_uuid,cutoff,deleted_at,request_id,policy_ts,policy_origin) VALUES (?1,'2021-01-01','2021-01-02',?2,0,'')", rusqlite::params![first_uuid,request]).unwrap();
    conn.execute("UPDATE sync_state SET applying=1", []).unwrap();
    for (id, correction, version, created, seen) in [
        (first.id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1", "2020-03-01", "[]".to_string()),
        (second.id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb2", "2020-01-01", serde_json::json!([format!("{first_uuid}:{request}")]).to_string()),
        (third.id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa3", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb3", "2020-02-01", "[]".to_string()),
    ] {
        conn.execute("INSERT INTO dictionary_corrections(uuid,context_id,dictionary_id,mistake,created_at) VALUES (?1,1,?2,'history typo',?3)", rusqlite::params![correction,id,created]).unwrap();
        conn.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at,seen_expired) SELECT ?1,?2,'everywhere-0000-0000-0000-000000000001',uuid,term,'history typo',0,?3,?4 FROM dictionary WHERE id=?5", rusqlite::params![version,correction,created,seen,id]).unwrap();
    }
    conn.execute("UPDATE sync_state SET applying=0", []).unwrap();
    drop(conn);
    let entries = db::query_effective_dictionary_for_context(&device.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert!(entries.iter().any(|entry| entry.term == "SecondHistoryFixture" && entry.mistake.as_deref() == Some("history typo")), "a defeated concurrent correction must not block the valid replacement");
}

#[test]
fn replacement_waits_for_observed_expiration_versions() {
    let device = Device::new();
    db::insert_dictionary_entry_returning(&device.db, "ReplacementFixture", Some("replacement typo"), None).unwrap();
    let item = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1";
    let request = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb1";
    let prior_version = "cccccccc-cccc-4ccc-8ccc-ccccccccccc1";
    {
        let conn = device.db.lock().unwrap();
        let seen = serde_json::json!([format!("{item}:{request}")]).to_string();
        conn.execute("UPDATE vocabulary_correction_versions SET seen_expired=?1 WHERE dictionary_term='ReplacementFixture'", [&seen]).unwrap();
        conn.execute("INSERT INTO vocabulary_expiration_events(request_id,item_uuid,cutoff,deleted_at,policy_ts,policy_origin,version_ids) VALUES (?1,?2,'2020-01-01 00:00:00','2020-01-02 00:00:00',0,'',?3)", rusqlite::params![request,item,serde_json::json!([prior_version]).to_string()]).unwrap();
    }
    let before = db::query_dictionary(&device.db).unwrap();
    assert_eq!(before[0].mistake, None, "a missing observed version must not be projected");
    {
        let conn = device.db.lock().unwrap();
        conn.execute("INSERT INTO vocabulary_correction_versions(version_id,correction_uuid,context_uuid,dictionary_uuid,dictionary_term,mistake,auto_learned,created_at) VALUES (?1,'dddddddd-dddd-4ddd-8ddd-ddddddddddd1','everywhere-0000-0000-0000-000000000001',?2,'PriorFixture','replacement typo',0,'2020-01-01')", rusqlite::params![prior_version,item]).unwrap();
    }
    let after = db::query_dictionary(&device.db).unwrap();
    assert_eq!(after[0].mistake.as_deref(), Some("replacement typo"));
}

#[test]
fn vocabulary_membership_removal_before_addition_does_not_resurrect() {
    let source = Device::new();
    let target = Device::new();
    db::insert_dictionary_entry_returning(&source.db, "DotOrderFixture", None, None).unwrap();
    let ops = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let parents = ops.iter().filter(|op| op.table == "dictionary" || op.table == "contexts").cloned().collect::<Vec<_>>();
    let add = ops.iter().find(|op| op.table == "vocabulary_membership_dots").unwrap().clone();
    let mut removal = add.clone();
    removal.payload.as_mut().unwrap()["removed"] = serde_json::json!(true);
    let conn = target.db.lock().unwrap();
    engine::apply_ops(&conn, &parents).unwrap();
    engine::apply_ops(&conn, &[removal]).unwrap();
    engine::apply_ops(&conn, &[add.clone(), add]).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary_contexts"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_dots WHERE removed=1"), 1);
}

#[tokio::test]
async fn removing_a_merged_vocabulary_identity_removes_all_observed_dots() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "IdentityDotFixture", None, None).unwrap();
    db::insert_dictionary_entry_returning(&b.db, "IdentityDotFixture", None, None).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let entries = db::query_effective_dictionary_for_context(&a.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(count(&a.db.lock().unwrap(), "SELECT COUNT(DISTINCT dictionary_uuid) FROM vocabulary_membership_dots WHERE removed=0 AND dictionary_term='IdentityDotFixture'"), 2);
    db::set_dictionary_context_assignment(&a.db, db::EVERYWHERE_CONTEXT_ID, entries[0].id, false).unwrap();
    assert_eq!(count(&a.db.lock().unwrap(), "SELECT COUNT(*) FROM vocabulary_membership_dots WHERE removed=0 AND dictionary_term='IdentityDotFixture'"), 0);
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert!(db::query_effective_dictionary_for_context(&device.db, db::EVERYWHERE_CONTEXT_ID).unwrap().is_empty());
    }
}

#[test]
fn active_membership_for_deleted_parent_is_recorded_without_deferring_forever() {
    let source = Device::new();
    let target = Device::new();
    let source_entry = db::insert_dictionary_entry_returning(&source.db, "DeletedDotFixture", None, None).unwrap();
    let ops = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&target.db.lock().unwrap(), &ops).unwrap();
    let target_entry = db::query_dictionary(&target.db).unwrap().into_iter().find(|entry| entry.term == "DeletedDotFixture").unwrap();
    db::delete_dictionary_entry(&target.db, target_entry.id).unwrap();
    db::set_dictionary_context_assignment(&source.db, db::EVERYWHERE_CONTEXT_ID, source_entry.id, false).unwrap();
    db::set_dictionary_context_assignment(&source.db, db::EVERYWHERE_CONTEXT_ID, source_entry.id, true).unwrap();
    let later = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let active = later.into_iter().find(|op| op.table == "vocabulary_membership_dots" && op.payload.as_ref().is_some_and(|payload| payload["removed"] == false)).unwrap();
    let conn = target.db.lock().unwrap();
    assert!(!engine::apply_ops(&conn, &[active.clone()]).unwrap().deferred);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary WHERE term='DeletedDotFixture'"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM vocabulary_membership_reviews WHERE dictionary_term='DeletedDotFixture'"), 1);
    let mut restored_parent = ops.iter().find(|op| op.table == "dictionary").unwrap().clone();
    let (delete_origin, delete_seq): (String,i64) = conn.query_row("SELECT origin,origin_seq FROM sync_log WHERE table_name='dictionary' AND row_uuid=?1 AND op='delete' ORDER BY seq DESC LIMIT 1", [&restored_parent.row_uuid], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    restored_parent.payload.as_mut().unwrap()["restore_basis"] = json!({"origin":delete_origin,"origin_seq":delete_seq});
    restored_parent.ts_ms += 1_000_000;
    assert!(!engine::apply_ops(&conn, &[restored_parent]).unwrap().deferred);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary_contexts"), 1, "a retained active dot must materialize when its parent returns");
    drop(conn);
    let status = db::vocabulary_expiration_status(&target.db).unwrap();
    let review = &status["reviews"][0];
    assert_eq!(review["term"], "DeletedDotFixture");
    db::acknowledge_vocabulary_review(&target.db, review["context_uuid"].as_str().unwrap(), review["dictionary_uuid"].as_str().unwrap()).unwrap();
    engine::apply_ops(&target.db.lock().unwrap(), &[active]).unwrap();
    assert!(db::vocabulary_expiration_status(&target.db).unwrap()["reviews"].as_array().unwrap().is_empty());
}

impl Device {
    fn new() -> Self {
        crate::legal_consent::enable_test_consent();
        let uuid = uuid::Uuid::new_v4().to_string();
        let dir = std::env::temp_dir().join(format!("verenu-sync-test-{uuid}"));
        std::fs::create_dir(&dir).unwrap();
        let db = db::open(dir.join("fixture.sqlite").to_str().unwrap()).unwrap();
        store::ensure_self_identity(&db.lock().unwrap(), &uuid, "Fixture device").unwrap();
        let certificate = rcgen::generate_simple_self_signed(vec![uuid.clone()]).unwrap();
        Self {
            dir,
            db,
            host: TestHost::new(&uuid),
            cert: certificate.cert.der().clone(),
            key: certificate.signing_key.serialize_der(),
        }
    }

    fn identity(&self) -> pairing::IdentityExchange {
        pairing::IdentityExchange {
            device_uuid: self.host.uuid.clone(),
            device_name: "Fixture device".into(),
            cert_der: self.cert.to_vec(),
            app_version: Some(self.host.version.clone()),
            sync_port: None,
            pairing_id: None,
        }
    }

    fn restart_database(&mut self) {
        self.db = db::open(self.dir.join("fixture.sqlite").to_str().unwrap()).unwrap();
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // Close the file-backed connection before cleanup on Windows.
        self.db = db::open(":memory:").unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

type Client = tokio_rustls::client::TlsStream<TcpStream>;
type Server = tokio_rustls::server::TlsStream<TcpStream>;

async fn connect(a: &Device, b: &Device) -> Result<(Client, Server)> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let connector = transport::tls_connector(transport::client_config(
        a.cert.clone(),
        PrivatePkcs8KeyDer::from(a.key.clone()),
    )?);
    let acceptor = tokio_rustls::TlsAcceptor::from(transport::server_config(
        b.cert.clone(),
        PrivatePkcs8KeyDer::from(b.key.clone()),
    )?);
    tokio::time::timeout(Duration::from_secs(10), async {
        let (client, server) = tokio::join!(
            async {
                let tcp = TcpStream::connect(address).await?;
                Ok::<_, anyhow::Error>(
                    connector
                        .connect(transport::server_name_for(&b.host.uuid), tcp)
                        .await?,
                )
            },
            async {
                let (tcp, _) = listener.accept().await?;
                transport::accept_with_timeout(&acceptor, tcp, Duration::from_secs(5)).await
            },
        );
        Ok((client?, server?))
    })
    .await
    .map_err(|_| anyhow!("fixture connection timed out"))?
}

async fn pair(a: &Device, b: &Device, correct_code: bool) -> Result<()> {
    pair_with_storage(a, b, correct_code, true).await
}

async fn pair_with_storage(
    a: &Device,
    b: &Device,
    correct_code: bool,
    save_trust: bool,
) -> Result<()> {
    let (mut client, mut server) = connect(a, b).await?;
    let (state, spake_msg) = pairing::initiator_start("123456");
    let (left, right) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            async {
                protocol::send_message(
                    &mut client,
                    protocol::Message::PairRequest {
                        device_uuid: a.host.uuid.clone(),
                        device_name: "Fixture A".into(),
                        protocol: protocol::PROTOCOL_VERSION,
                        spake_msg,
                    },
                )
                .await?;
                let protocol::Message::PairAccept { spake_msg } =
                    protocol::read_message(&mut client).await?
                else {
                    return Err(anyhow!("expected pairing acceptance"));
                };
                let cipher = pairing::initiator_cipher(state, &spake_msg)?;
                pairing::initiator_exchange(&mut client, &cipher, &a.identity(), &b.host.uuid).await
            },
            async {
                let protocol::Message::PairRequest {
                    spake_msg,
                    device_uuid,
                    ..
                } = protocol::read_message(&mut server).await?
                else {
                    return Err(anyhow!("expected pairing request"));
                };
                let code = if correct_code { "123456" } else { "654321" };
                let (message, cipher) = pairing::responder_start(code, &spake_msg)?;
                let result = pairing::responder_exchange(
                    &mut server,
                    &cipher,
                    message,
                    &b.identity(),
                    &device_uuid,
                )
                .await;
                if result.is_err() {
                    let _ = protocol::send_message(
                        &mut server,
                        protocol::Message::PairReject {
                            reason: "code mismatch".into(),
                        },
                    )
                    .await;
                }
                let remote = result?;
                if !save_trust {
                    protocol::send_message(
                        &mut server,
                        protocol::Message::Error {
                            message: "simulated trust persistence failure".into(),
                        },
                    )
                    .await?;
                    return Err(anyhow!("simulated trust persistence failure"));
                }
                store::upsert_peer(
                    &b.db.lock().unwrap(),
                    &remote.device_uuid,
                    &remote.device_name,
                    &identity::fingerprint_of(&remote.cert_der),
                )?;
                protocol::send_message(&mut server, protocol::Message::PairComplete).await?;
                Ok(remote)
            },
        )
    })
    .await
    .map_err(|_| anyhow!("fixture pairing timed out"))?;
    if !save_trust {
        assert!(
            left.is_err(),
            "the initiator must not accept pairing before trust is saved"
        );
        assert!(right.is_err());
    }
    let left = left?;
    let right = right?;
    assert_eq!(left.cert_der, b.cert.as_ref());
    assert_eq!(right.cert_der, a.cert.as_ref());
    store::upsert_peer(
        &a.db.lock().unwrap(),
        &left.device_uuid,
        &left.device_name,
        &identity::fingerprint_of(&left.cert_der),
    )?;
    Ok(())
}

async fn session(a: &Device, b: &Device) -> Result<()> {
    let (mut client, mut server) = connect(a, b).await?;
    let peer_a = store::get_peer(&a.db.lock().unwrap(), &b.host.uuid)?
        .ok_or_else(|| anyhow!("not paired"))?;
    let peer_b = store::get_peer(&b.db.lock().unwrap(), &a.host.uuid)?
        .ok_or_else(|| anyhow!("not paired"))?;
    assert_eq!(
        transport::peer_fingerprint(client.get_ref().1.peer_certificates().unwrap())?,
        peer_a.cert_fp
    );
    assert_eq!(
        transport::peer_fingerprint(server.get_ref().1.peer_certificates().unwrap())?,
        peer_b.cert_fp
    );
    let (left, right) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(
            engine::run_session(&a.db, &a.host, &mut client, true, &peer_a),
            engine::run_session(&b.db, &b.host, &mut server, false, &peer_b),
        )
    })
    .await
    .map_err(|_| anyhow!("fixture session timed out"))?;
    left?;
    right?;
    Ok(())
}

#[tokio::test]
async fn pairing_and_sync_require_matching_app_versions() {
    let a = Device::new();
    let mut b = Device::new();
    b.host.version = "other".into();
    let error = pair(&a, &b, true).await.unwrap_err();
    assert!(error.to_string().contains("same Verenu version"));
    assert!(store::get_peer(&a.db.lock().unwrap(), &b.host.uuid).unwrap().is_none());
    assert!(store::get_peer(&b.db.lock().unwrap(), &a.host.uuid).unwrap().is_none());

    b.host.version = "test".into();
    pair(&a, &b, true).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "VersionFixture", None, None).unwrap();
    b.host.version = "other".into();
    let error = session(&a, &b).await.unwrap_err();
    assert!(error.to_string().contains("same Verenu version"));
    let error = session(&b, &a).await.unwrap_err();
    assert!(error.to_string().contains("same Verenu version"));
    assert!(store::get_peer(&a.db.lock().unwrap(), &b.host.uuid).unwrap().is_some());
    assert!(store::get_peer(&b.db.lock().unwrap(), &a.host.uuid).unwrap().is_some());
    assert!(db::query_effective_dictionary_for_context(&b.db, db::EVERYWHERE_CONTEXT_ID).unwrap().is_empty());

    b.host.version = "test".into();
    session(&a, &b).await.unwrap();
    assert_eq!(db::query_effective_dictionary_for_context(&b.db, db::EVERYWHERE_CONTEXT_ID).unwrap().len(), 1);
}

#[tokio::test]
async fn pristine_peer_snapshot_does_not_remove_existing_context_membership() {
    for distinct_alias in [false, true] {
    let source = Device::new();
    let new_peer = Device::new();
    if distinct_alias {
        let conn = new_peer.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    }
    db::insert_dictionary_entry_returning(&source.db, "SnapshotFixture", Some("snapshot typo"), None).unwrap();
    source.db.lock().unwrap().execute("UPDATE contexts SET custom_instructions='synthetic instruction' WHERE is_everywhere=1", []).unwrap();
    pair(&source, &new_peer, true).await.unwrap();
    session(&source, &new_peer).await.unwrap();
    session(&new_peer, &source).await.unwrap();
    for device in [&source, &new_peer] {
        let entries = db::query_effective_dictionary_for_context(&device.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mistake.as_deref(), Some("snapshot typo"));
        let instructions: Option<String> = device.db.lock().unwrap().query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
        assert_eq!(instructions.as_deref(), Some("synthetic instruction"));
    }
    }
}

#[test]
fn pristine_everywhere_alias_does_not_overwrite_context_settings() {
    let source = Device::new();
    let new_peer = Device::new();
    source.db.lock().unwrap().execute("UPDATE contexts SET custom_instructions='synthetic instruction' WHERE is_everywhere=1", []).unwrap();
    let ops = {
        let conn = new_peer.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
        let (ops, _, _) = engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
        ops.into_iter().filter(|op| op.table == "contexts").collect::<Vec<_>>()
    };
    let conn = source.db.lock().unwrap();
    engine::apply_ops(&conn, &ops).unwrap();
    let instructions: Option<String> = conn.query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
    assert_eq!(instructions.as_deref(), Some("synthetic instruction"));
}

#[test]
fn pristine_alias_does_not_create_a_newer_context_change() {
    let empty_peer = Device::new();
    let receiver = Device::new();
    let customized_peer = Device::new();
    customized_peer.db.lock().unwrap().execute("UPDATE contexts SET custom_instructions='earlier customization' WHERE is_everywhere=1", []).unwrap();
    customized_peer.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=1 WHERE table_name='contexts'", []).unwrap();
    let context_ops = |device: &Device| {
        let conn = device.db.lock().unwrap();
        let (ops, _, _) = engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
        ops.into_iter().filter(|op| op.table == "contexts").collect::<Vec<_>>()
    };
    {
        let conn = empty_peer.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    }
    let empty_ops = context_ops(&empty_peer);
    let customized_ops = context_ops(&customized_peer);
    let conn = receiver.db.lock().unwrap();
    engine::apply_ops(&conn, &empty_ops).unwrap();
    engine::apply_ops(&conn, &customized_ops).unwrap();
    let instructions: Option<String> = conn.query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
    assert_eq!(instructions.as_deref(), Some("earlier customization"));
}

#[test]
fn unstamped_vocabulary_does_not_erase_customized_everywhere_settings() {
    let source = Device::new();
    let receiver = Device::new();
    db::insert_dictionary_entry_returning(&source.db, "PreSyncFixture", Some("fixture typo"), None).unwrap();
    {
        let conn = source.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    }
    {
        let conn = receiver.db.lock().unwrap();
        conn.execute("UPDATE contexts SET custom_instructions='preserve instructions' WHERE is_everywhere=1", []).unwrap();
        conn.execute("UPDATE sync_log SET ts_ms=1 WHERE table_name='contexts'", []).unwrap();
    }
    let ops = {
        let conn = source.db.lock().unwrap();
        engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0
    };
    let conn = receiver.db.lock().unwrap();
    engine::apply_ops(&conn, &ops).unwrap();
    let instructions: Option<String> = conn.query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
    assert_eq!(instructions.as_deref(), Some("preserve instructions"));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary_corrections"), 1);
}

#[test]
fn older_membership_operation_can_relay_newer_context_metadata() {
    let origin = Device::new();
    let relay = Device::new();
    let recipient = Device::new();
    {
        let conn = origin.db.lock().unwrap();
        conn.execute("UPDATE contexts SET custom_instructions='relay fixture' WHERE is_everywhere=1", []).unwrap();
        conn.execute("UPDATE sync_log SET ts_ms=10 WHERE table_name='contexts'", []).unwrap();
        conn.execute("UPDATE sync_setting_meta SET ts_ms=10 WHERE key LIKE 'context_metadata:%'", []).unwrap();
    }
    db::insert_dictionary_entry_returning(&relay.db, "RelayFixture", None, None).unwrap();
    relay.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=20 WHERE table_name='contexts'", []).unwrap();
    let snapshot = |device: &Device| {
        let conn = device.db.lock().unwrap();
        engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0
    };
    engine::apply_ops(&recipient.db.lock().unwrap(), &snapshot(&relay)).unwrap();
    let cursor = store::max_log_seq(&relay.db.lock().unwrap()).unwrap();
    engine::apply_ops(&relay.db.lock().unwrap(), &snapshot(&origin)).unwrap();
    let changes = {
        let conn = relay.db.lock().unwrap();
        engine::collect_ops(&conn, cursor, false, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0
    };
    engine::apply_ops(&recipient.db.lock().unwrap(), &changes).unwrap();
    let instructions: Option<String> = recipient.db.lock().unwrap().query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
    assert_eq!(instructions.as_deref(), Some("relay fixture"));
}

#[tokio::test]
async fn distinct_everywhere_alias_relay_keeps_incremental_metadata() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    {
        let conn = a.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    }
    pair(&a, &b, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    db::insert_dictionary_entry_returning(&b.db, "IncrementalFixture", None, None).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    assert!(store::peer_recv_cursor(&c.db.lock().unwrap(), &b.host.uuid).unwrap() > 0);
    a.db.lock().unwrap().execute("UPDATE contexts SET custom_instructions='relay instructions' WHERE is_everywhere=1", []).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    for device in [&a, &b, &c] {
        let instructions: Option<String> = device.db.lock().unwrap().query_row("SELECT custom_instructions FROM contexts WHERE is_everywhere=1", [], |row| row.get(0)).unwrap();
        assert_eq!(instructions.as_deref(), Some("relay instructions"));
    }
}

#[test]
fn older_everywhere_alias_cannot_prune_newer_local_membership() {
    let a = Device::new();
    let b = Device::new();
    {
        let conn = a.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
        let baseline = engine::collect_ops(&conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
        engine::apply_ops(&b.db.lock().unwrap(), &baseline).unwrap();
    }
    db::insert_dictionary_entry_returning(&a.db, "OlderFixture", None, None).unwrap();
    db::insert_dictionary_entry_returning(&b.db, "NewerFixture", Some("newer typo"), None).unwrap();
    a.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=10 WHERE table_name='contexts'", []).unwrap();
    b.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=20 WHERE table_name='contexts'", []).unwrap();
    let ops = engine::collect_ops(&a.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    engine::apply_ops(&b.db.lock().unwrap(), &ops).unwrap();
    let entries = db::query_effective_dictionary_for_context(&b.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert!(entries.iter().any(|entry| entry.term == "NewerFixture" && entry.mistake.as_deref() == Some("newer typo")));
    assert!(entries.iter().any(|entry| entry.term == "OlderFixture"), "concurrent vocabulary additions from both devices must be retained");
}

#[test]
fn deferred_everywhere_alias_merges_members_on_first_application() {
    let a = Device::new();
    let b = Device::new();
    {
        let conn = a.db.lock().unwrap();
        conn.execute("UPDATE contexts SET uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' WHERE is_everywhere=1", []).unwrap();
        conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    }
    db::insert_dictionary_entry_returning(&a.db, "RemoteFixture", None, None).unwrap();
    db::insert_dictionary_entry_returning(&b.db, "LocalFixture", Some("local typo"), None).unwrap();
    let ops = engine::collect_ops(&a.db.lock().unwrap(), 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap().0;
    let context = ops.iter().filter(|op| op.table == "contexts").cloned().collect::<Vec<_>>();
    assert!(engine::apply_ops(&b.db.lock().unwrap(), &context).unwrap().deferred);
    assert!(!engine::apply_ops(&b.db.lock().unwrap(), &ops).unwrap().deferred);
    let entries = db::query_effective_dictionary_for_context(&b.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
    assert!(entries.iter().any(|entry| entry.term == "LocalFixture" && entry.mistake.as_deref() == Some("local typo")));
    assert!(entries.iter().any(|entry| entry.term == "RemoteFixture"));
}

#[test]
fn only_pristine_unstamped_everywhere_uses_the_baseline_snapshot_stamp() {
    let device = Device::new();
    let conn = device.db.lock().unwrap();
    let snapshot_stamp = |conn: &rusqlite::Connection| {
        let (ops, _, _) = engine::collect_ops(conn, 0, true, 10_000, &mut engine::SnapshotProgress::default()).unwrap();
        let op = ops.into_iter().find(|op| op.table == "contexts").unwrap();
        (op.ts_ms, op.origin, op.origin_seq)
    };
    assert_eq!(snapshot_stamp(&conn), (0, String::new(), 0));
    conn.execute("UPDATE contexts SET custom_instructions='fixture' WHERE is_everywhere=1", []).unwrap();
    conn.execute("DELETE FROM sync_log WHERE table_name='contexts'", []).unwrap();
    assert!(snapshot_stamp(&conn).0 > 0, "existing unstamped Context customization is not a pristine baseline");
}

#[tokio::test]
async fn first_pairing_preserves_existing_everywhere_corrections() {
    let a = Device::new();
    let b = Device::new();
    db::insert_dictionary_entry_returning(&a.db, "ExistingTerm", Some("existing mistake"), None).unwrap();
    db::insert_snippet_returning(&b.db, "existing-shortcut", "fixture", "", None).unwrap();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        let entries = db::query_effective_dictionary_for_context(&device.db, db::EVERYWHERE_CONTEXT_ID).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mistake.as_deref(), Some("existing mistake"));
        assert_eq!(db::query_snippets(&device.db).unwrap().len(), 1);
    }
}

#[tokio::test]
async fn expiration_waits_for_policy_on_transitive_offline_device() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    session(&a, &b).await.unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    assert_eq!(db::vocabulary_expiration_status(&a.db).unwrap()["paused_for_sync"], true);
    session(&b, &c).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    assert_eq!(db::vocabulary_expiration_status(&a.db).unwrap()["paused_for_sync"], false);
}

#[tokio::test]
async fn vocabulary_activity_survives_duplicate_term_identity_resolution() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    let a_entry = db::insert_dictionary_entry_returning(&a.db, "DuplicateTerm", None, None).unwrap();
    let b_entry = db::insert_dictionary_entry_returning(&b.db, "DuplicateTerm", None, None).unwrap();
    for (device, peer) in [(&a, &b), (&b, &a)] {
        store::mark_peer_synced(&device.db.lock().unwrap(), &peer.host.uuid, 0).unwrap();
        device.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    }
    db::record_vocabulary_activity(&b.db, &[b_entry.id]).unwrap();
    a.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=ts_ms+10000 WHERE table_name='dictionary' AND row_uuid=(SELECT uuid FROM dictionary WHERE id=?1)", [a_entry.id]).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert_eq!(db::query_dictionary(&device.db).unwrap().len(), 1, "activity on the losing identity must keep the canonical term active");
    }
}

#[tokio::test]
async fn vocabulary_activity_from_offline_third_device_survives_identity_resolution() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&a, &b).await.unwrap();
    db::insert_dictionary_entry_returning(&b.db, "SharedDuplicate", None, None).unwrap();
    session(&b, &c).await.unwrap();
    let c_entry = db::query_dictionary(&c.db).unwrap()[0].id;
    for device in [&b, &c] {
        device.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days');").unwrap();
    }
    let a_entry = db::insert_dictionary_entry_returning(&a.db, "SharedDuplicate", None, None).unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    a.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    // C uses its original identity while disconnected; A later wins the collision.
    db::record_vocabulary_activity(&c.db, &[c_entry]).unwrap();
    a.db.lock().unwrap().execute("UPDATE sync_log SET ts_ms=ts_ms+10000 WHERE table_name='dictionary' AND row_uuid=(SELECT uuid FROM dictionary WHERE id=?1)", [a_entry.id]).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    assert!(db::query_dictionary(&a.db).unwrap().is_empty(), "the term must remain expired until C's activity arrives");
    pair(&a, &c, true).await.unwrap();
    session(&a, &c).await.unwrap();
    session(&c, &a).await.unwrap();
    for device in [&a, &c] {
        assert_eq!(db::query_dictionary(&device.db).unwrap().len(), 1, "offline activity must survive a third-device snapshot");
        let links: i64 = device.db.lock().unwrap().query_row("SELECT COUNT(*) FROM vocabulary_identity_links", [], |row| row.get(0)).unwrap();
        assert!(links > 0, "identity links must reach the third device");
    }
}

#[tokio::test]
async fn vocabulary_expiration_converges_with_offline_usage_and_recovery() {
    let mut a = Device::new();
    let mut b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    let entry = db::insert_dictionary_entry_returning(&a.db, "SyntheticTerm", Some("sinthetic"), None).unwrap();
    a.db.lock().unwrap().execute("UPDATE dictionary SET replacement_confidence='high' WHERE id=?1", [entry.id]).unwrap();
    db::insert_snippet_returning(&a.db, "synthetic-shortcut", "fixture", "", None).unwrap();
    session(&a, &b).await.unwrap();
    for device in [&a, &b] {
        assert_eq!(db::query_dictionary(&device.db).unwrap()[0].mistake.as_deref(), Some("sinthetic"), "correction after initial sync");
    }
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    for (device, peer) in [(&a, &b), (&b, &a)] {
        store::mark_peer_synced(&device.db.lock().unwrap(), &peer.host.uuid, 0).unwrap();
        device.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    }
    let b_id = db::query_dictionary(&b.db).unwrap()[0].id;
    db::record_vocabulary_activity(&b.db, &[b_id]).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    assert!(db::query_dictionary(&a.db).unwrap().is_empty());
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    for device in [&a, &b] {
        assert_eq!(db::query_dictionary(&device.db).unwrap()[0].mistake.as_deref(), Some("sinthetic"), "correction before restart");
    }
    a.restart_database();
    b.restart_database();
    for device in [&a, &b] {
        let entries = db::query_dictionary(&device.db).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mistake.as_deref(), Some("sinthetic"), "device {} corrections {:?}", device.host.uuid, entries[0].corrections);
        assert_eq!(db::query_snippets(&device.db).unwrap().len(), 1);
        assert!(db::vocabulary_expiration_status(&device.db).unwrap()["deleted"].as_array().unwrap().is_empty());
    }
    // A later expiration reaches both peers and can be restored on either one.
    for device in [&a, &b] {
        device.db.lock().unwrap().execute_batch("UPDATE vocabulary_expiration_usage SET last_used_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_marks SET cutoff=datetime('now','-30 days');").unwrap();
    }
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert!(db::query_dictionary(&device.db).unwrap().is_empty());
        assert_eq!(db::query_snippets(&device.db).unwrap().len(), 1);
    }
    let status = db::vocabulary_expiration_status(&b.db).unwrap();
    let request = status["deleted"][0]["request_id"].as_str().unwrap();
    db::restore_expired_vocabulary(&b.db, request).unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    for device in [&a, &b] {
        let entries = db::query_dictionary(&device.db).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mistake.as_deref(), Some("sinthetic"));
        assert_eq!(entries[0].replacement_confidence, "high");
        assert!(db::vocabulary_expiration_status(&device.db).unwrap()["deleted"].as_array().unwrap().is_empty());
    }
    assert_eq!(db::query_dictionary(&a.db).unwrap()[0].id, entry.id);
}

#[tokio::test]
async fn concurrent_active_correction_conflicts_have_one_convergent_projection() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "First", Some("typo"), None).unwrap();
    db::insert_dictionary_entry_returning(&b.db, "Second", Some("typo"), None).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let projection = |device: &Device| {
        let mut entries: Vec<_> = db::query_dictionary(&device.db).unwrap().into_iter()
            .map(|entry| (entry.term, entry.mistake)).collect();
        entries.sort();
        entries
    };
    let left = projection(&a);
    assert_eq!(left, projection(&b));
    assert_eq!(left.len(), 2);
    assert_eq!(left.iter().filter(|(_, mistake)| mistake.as_deref() == Some("typo")).count(), 1);
}

#[tokio::test]
async fn concurrent_edits_to_distinct_terms_project_one_active_correction() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    let first = db::insert_dictionary_entry_returning(&a.db, "First", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Second", None, None).unwrap();
    session(&a, &b).await.unwrap();
    let second = db::query_dictionary(&b.db).unwrap().into_iter().find(|entry| entry.term == "Second").unwrap();
    db::update_dictionary_entry(&a.db, first.id, "First", Some("typo")).unwrap();
    db::update_dictionary_entry(&b.db, second.id, "Second", Some("typo")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let projection = |device: &Device| {
        let mut entries: Vec<_> = db::query_dictionary(&device.db).unwrap().into_iter()
            .map(|entry| (entry.term, entry.mistake)).collect();
        entries.sort();
        entries
    };
    let left = projection(&a);
    assert_eq!(left, projection(&b));
    assert_eq!(left.iter().filter(|(_, mistake)| mistake.as_deref() == Some("typo")).count(), 1);
    assert!(db::insert_dictionary_entry_returning(&a.db, "Third", Some("typo"), None).is_err());
    let winner = left.iter().find(|(_, mistake)| mistake.as_deref() == Some("typo")).unwrap().0.clone();
    let winner_id = db::query_dictionary(&a.db).unwrap().into_iter().find(|entry| entry.term == winner).unwrap().id;
    db::delete_dictionary_entry(&a.db, winner_id).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert!(projection(device).iter().all(|(_, mistake)| mistake.as_deref() != Some("typo")), "a suppressed mapping must not activate after deleting the winner");
    }
    let survivor = db::query_dictionary(&a.db).unwrap().into_iter().find(|entry| entry.term != winner).unwrap();
    db::update_dictionary_entry(&a.db, survivor.id, &survivor.term, Some("typo")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert!(projection(device).iter().any(|(term,mistake)| term == &survivor.term && mistake.as_deref() == Some("typo")), "an explicit re-edit after deletion must be usable");
    }
}

#[tokio::test]
async fn expiring_conflict_winner_does_not_activate_suppressed_correction() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    let first = db::insert_dictionary_entry_returning(&a.db, "First", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Second", None, None).unwrap();
    session(&a, &b).await.unwrap();
    let second = db::query_dictionary(&b.db).unwrap().into_iter().find(|entry| entry.term == "Second").unwrap();
    db::update_dictionary_entry(&a.db, first.id, "First", Some("typo")).unwrap();
    db::update_dictionary_entry(&b.db, second.id, "Second", Some("typo")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let winner = db::query_dictionary(&a.db).unwrap().into_iter().find(|entry| entry.mistake.as_deref() == Some("typo")).unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    {
        let conn = a.db.lock().unwrap();
        conn.execute("UPDATE dictionary SET created_at=datetime('now','-20 days') WHERE id=?1", [winner.id]).unwrap();
        conn.execute("UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days')", []).unwrap();
    }
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    assert!(db::query_dictionary(&a.db).unwrap().iter().all(|entry| entry.mistake.as_deref() != Some("typo")), "a suppressed mapping must not activate after the winner expires");
    let status = db::vocabulary_expiration_status(&a.db).unwrap();
    let request = status["deleted"][0]["request_id"].as_str().unwrap();
    db::restore_expired_vocabulary(&a.db, request).unwrap();
    let restored = db::query_dictionary(&a.db).unwrap();
    assert!(restored.iter().any(|entry| entry.term == winner.term && entry.mistake.as_deref() == Some("typo")), "restoring the winner must recover its correction");
}

#[tokio::test]
async fn late_third_device_does_not_activate_conflict_loser_after_winner_deletion() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    let first = db::insert_dictionary_entry_returning(&a.db, "First", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Second", None, None).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    let second = db::query_dictionary(&b.db).unwrap().into_iter().find(|entry| entry.term == "Second").unwrap();
    db::update_dictionary_entry(&a.db, first.id, "First", Some("typo")).unwrap();
    db::update_dictionary_entry(&b.db, second.id, "Second", Some("typo")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let winner = db::query_dictionary(&a.db).unwrap().into_iter().find(|entry| entry.mistake.as_deref() == Some("typo")).unwrap();
    db::delete_dictionary_entry(&a.db, winner.id).unwrap();
    session(&a, &c).await.unwrap();
    session(&c, &a).await.unwrap();
    assert!(db::query_dictionary(&c.db).unwrap().iter().all(|entry| entry.mistake.as_deref() != Some("typo")), "a late device must not activate a suppressed correction");
}

#[tokio::test]
async fn conflict_loser_stays_suppressed_in_snapshot_after_compaction() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    let first = db::insert_dictionary_entry_returning(&a.db, "First", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Second", None, None).unwrap();
    session(&a, &b).await.unwrap();
    let second = db::query_dictionary(&b.db).unwrap().into_iter().find(|entry| entry.term == "Second").unwrap();
    db::update_dictionary_entry(&a.db, first.id, "First", Some("typo")).unwrap();
    db::update_dictionary_entry(&b.db, second.id, "Second", Some("typo")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let winner = db::query_dictionary(&a.db).unwrap().into_iter().find(|entry| entry.mistake.as_deref() == Some("typo")).unwrap();
    let loser = if winner.term == "First" { "Second" } else { "First" };
    {
        let conn = a.db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_corrections c JOIN dictionary d ON d.id=c.dictionary_id WHERE d.term=?1 AND c.mistake='typo'", [loser], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "the suppressed correction must survive the initial sync");
    }
    db::delete_dictionary_entry(&a.db, winner.id).unwrap();
    {
        let conn = a.db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_corrections c JOIN dictionary d ON d.id=c.dictionary_id WHERE d.term=?1 AND c.mistake='typo'", [loser], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "deleting the winner must retain the suppressed correction");
    }
    store::compact_log(&a.db.lock().unwrap()).unwrap();
    let retained = || -> i64 {
        let conn = a.db.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM dictionary_corrections c JOIN dictionary d ON d.id=c.dictionary_id WHERE d.term=?1 AND c.mistake='typo'", [loser], |row| row.get(0)).unwrap()
    };
    assert_eq!(retained(), 1, "compaction must retain the suppressed correction");
    let c = Device::new();
    pair(&a, &c, true).await.unwrap();
    assert_eq!(retained(), 1, "pairing must retain the suppressed correction");
    session(&a, &c).await.unwrap();
    assert_eq!(retained(), 1, "the first snapshot exchange must retain the suppressed correction");
    session(&c, &a).await.unwrap();
    let entries = db::query_dictionary(&c.db).unwrap();
    assert!(entries.iter().any(|entry| entry.term == loser), "the surviving term must reach the new device");
    assert!(entries.iter().all(|entry| entry.mistake.as_deref() != Some("typo")), "a compacted snapshot must retain the conflict decision");
    for (label, device) in [("source", &a), ("snapshot recipient", &c)] {
        let conn = device.db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_corrections c JOIN dictionary d ON d.id=c.dictionary_id WHERE d.term=?1 AND c.mistake='typo'", [loser], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "the suppressed correction must remain recoverable on the {label}");
    }
}

#[tokio::test]
async fn concurrent_unicode_corrections_do_not_reappear_after_winner_deletion() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    let first = db::insert_dictionary_entry_returning(&a.db, "First", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Second", None, None).unwrap();
    session(&a, &b).await.unwrap();
    let second = db::query_dictionary(&b.db).unwrap().into_iter().find(|entry| entry.term == "Second").unwrap();
    db::update_dictionary_entry(&a.db, first.id, "First", Some("École")).unwrap();
    db::update_dictionary_entry(&b.db, second.id, "Second", Some("école")).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    session(&a, &b).await.unwrap();
    let entries = db::query_dictionary(&a.db).unwrap();
    let active: Vec<_> = entries.iter().filter(|entry| entry.mistake.as_deref().is_some_and(|mistake| mistake.to_lowercase() == "école")).collect();
    assert_eq!(active.len(), 1);
    db::delete_dictionary_entry(&a.db, active[0].id).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        assert!(db::query_dictionary(&device.db).unwrap().iter().all(|entry| entry.mistake.as_deref().is_none_or(|mistake| mistake.to_lowercase() != "école")), "a case-folded suppressed correction must not activate");
    }
}

#[tokio::test]
async fn expired_vocabulary_correction_conflict_survives_sync() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "First", Some("typo"), None).unwrap();
    session(&a, &b).await.unwrap();
    for device in [&a, &b] {
        device.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    }
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    a.db.lock().unwrap().execute("UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days')", []).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    db::insert_dictionary_entry_returning(&b.db, "Second", Some("typo"), None).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        let status = db::vocabulary_expiration_status(&device.db).unwrap();
        let request = status["deleted"][0]["request_id"].as_str().unwrap();
        assert!(db::restore_expired_vocabulary(&device.db, request).is_err());
        let conn = device.db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_corrections WHERE mistake='typo'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
    }
}

#[tokio::test]
async fn offline_expiration_keeps_corrections_when_sync_arrives_out_of_order() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "First", Some("typo"), None).unwrap();
    session(&a, &b).await.unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    store::mark_peer_synced(&b.db.lock().unwrap(), &a.host.uuid, 0).unwrap();
    for device in [&a, &b] {
        device.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days'); UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    }
    assert_eq!(db::expire_vocabulary(&b.db).unwrap(), 1);
    db::insert_dictionary_entry_returning(&b.db, "Second", Some("typo"), None).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        let status = db::vocabulary_expiration_status(&device.db).unwrap();
        let request = status["deleted"][0]["request_id"].as_str().unwrap();
        assert!(db::restore_expired_vocabulary(&device.db, request).is_err());
        let conn = device.db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_corrections WHERE mistake='typo'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
    }
}

#[tokio::test]
async fn vocabulary_policy_syncs_before_activity_tracking_begins() {
    let mut a = Device::new();
    let mut b = Device::new();
    pair(&a, &b, true).await.unwrap();
    let entry = db::insert_dictionary_entry_returning(&a.db, "Synthetic", None, None).unwrap();
    session(&a, &b).await.unwrap();
    let remote = db::query_dictionary(&b.db).unwrap().into_iter().find(|row| row.term == "Synthetic").unwrap();
    assert_eq!(db::vocabulary_expiration_status(&b.db).unwrap()["days"], serde_json::Value::Null);
    db::record_vocabulary_activity(&b.db, &[remote.id]).unwrap();
    assert_eq!(b.db.lock().unwrap().query_row("SELECT COUNT(*) FROM vocabulary_expiration_usage", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    assert_eq!(db::vocabulary_expiration_status(&a.db).unwrap()["paused_for_sync"], true);
    session(&a, &b).await.unwrap();
    assert_eq!(db::vocabulary_expiration_status(&b.db).unwrap()["days"], 7);
    session(&a, &b).await.unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    assert_eq!(db::vocabulary_expiration_status(&a.db).unwrap()["paused_for_sync"], false);
    db::record_vocabulary_activity(&b.db, &[remote.id]).unwrap();
    b.restart_database();
    session(&a, &b).await.unwrap();
    a.restart_database();
    let conn = a.db.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM vocabulary_expiration_usage WHERE item_uuid=(SELECT uuid FROM dictionary WHERE id=?1)", [entry.id], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
    drop(conn);
    db::set_vocabulary_expiration(&a.db, None).unwrap();
    session(&a, &b).await.unwrap();
    assert!(db::vocabulary_expiration_status(&b.db).unwrap()["days"].is_null());
    b.db.lock().unwrap().execute("UPDATE vocabulary_expiration_usage SET last_used_at='2000-01-01 00:00:00'", []).unwrap();
    db::record_vocabulary_activity(&b.db, &[remote.id]).unwrap();
    let retained: String = b.db.lock().unwrap().query_row("SELECT last_used_at FROM vocabulary_expiration_usage LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(retained, "2000-01-01 00:00:00");
}

#[tokio::test]
async fn offline_disable_rejects_later_expiration_but_keeps_prior_recovery() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b, true).await.unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Earlier", None, None).unwrap();
    db::insert_dictionary_entry_returning(&a.db, "Later", None, None).unwrap();
    session(&a, &b).await.unwrap();
    db::set_vocabulary_expiration(&a.db, Some(7)).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    store::mark_peer_synced(&a.db.lock().unwrap(), &b.host.uuid, 0).unwrap();
    a.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days') WHERE term='Earlier'; UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    b.db.lock().unwrap().execute("UPDATE dictionary SET created_at=datetime('now','-20 days') WHERE term='Earlier'", []).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    assert_eq!(db::vocabulary_expiration_status(&b.db).unwrap()["deleted"].as_array().unwrap().len(), 1, "earlier expiration must reach B before disabling");
    db::set_vocabulary_expiration(&b.db, None).unwrap();
    a.db.lock().unwrap().execute_batch("UPDATE dictionary SET created_at=datetime('now','-20 days') WHERE term='Later'; UPDATE vocabulary_expiration_settings SET tracking_since=datetime('now','-20 days');").unwrap();
    b.db.lock().unwrap().execute("UPDATE dictionary SET created_at=datetime('now','-20 days') WHERE term='Later'", []).unwrap();
    assert_eq!(db::expire_vocabulary(&a.db).unwrap(), 1);
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b] {
        let entries = db::query_dictionary(&device.db).unwrap();
        assert_eq!(entries.len(), 1, "device {}: {:?}", device.host.uuid, entries.iter().map(|entry| &entry.term).collect::<Vec<_>>());
        assert_eq!(entries[0].term, "Later");
        let status = db::vocabulary_expiration_status(&device.db).unwrap();
        assert!(status["days"].is_null());
        assert_eq!(status["deleted"].as_array().unwrap().len(), 1);
        assert_eq!(status["deleted"][0]["term"], "Earlier");
    }
}

fn logical_state(device: &Device) -> Vec<Vec<Vec<serde_json::Value>>> {
    let conn = device.db.lock().unwrap();
    [
        "SELECT uuid, term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at, created_at FROM dictionary ORDER BY uuid",
        "SELECT uuid, trigger, expansion, instructions, use_count, created_at FROM snippets ORDER BY uuid",
        "SELECT CASE WHEN is_everywhere THEN 'everywhere' ELSE uuid END, name, icon, tone, cleanup_intensity, color, custom_instructions, contextual_formatting_disabled, pinned_at FROM contexts ORDER BY is_everywhere, uuid",
        "SELECT CASE WHEN c.is_everywhere THEN 'everywhere' ELSE c.uuid END, d.uuid FROM dictionary_contexts dc JOIN contexts c ON c.id = dc.context_id JOIN dictionary d ON d.id = dc.dictionary_id ORDER BY 1, 2",
        "SELECT CASE WHEN c.is_everywhere THEN 'everywhere' ELSE c.uuid END, s.uuid FROM snippet_contexts sc JOIN contexts c ON c.id = sc.context_id JOIN snippets s ON s.id = sc.snippet_id ORDER BY 1, 2",
        "SELECT c.uuid, w.domain FROM context_website_targets w JOIN contexts c ON c.id = w.context_id ORDER BY 1, 2",
        "SELECT dc.uuid, CASE WHEN c.is_everywhere THEN 'everywhere' ELSE c.uuid END, d.uuid, dc.mistake, dc.auto_learned, dc.correction_count, dc.confidence_tier, dc.last_seen_at FROM dictionary_corrections dc JOIN contexts c ON c.id = dc.context_id JOIN dictionary d ON d.id = dc.dictionary_id ORDER BY dc.uuid",
        "SELECT t.uuid, t.raw_text, t.clean_text, t.words, t.spoken_words, t.duration_ms, t.api_used, t.app_name, CASE WHEN c.is_everywhere THEN 'everywhere' ELSE c.uuid END, t.created_at FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id ORDER BY t.uuid",
        "SELECT a.uuid, t.uuid, a.model, a.provider, a.task, a.audio_ms, a.input_chars, a.output_chars, a.created_at FROM api_calls a JOIN transcriptions t ON t.id = a.transcription_id ORDER BY a.uuid",
    ].iter().map(|sql| {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            (0..columns).map(|index| {
                Ok(match row.get_ref(index)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(value) => json!(value),
                    rusqlite::types::ValueRef::Real(value) => json!(value),
                    rusqlite::types::ValueRef::Text(value) => json!(std::str::from_utf8(value).unwrap()),
                    rusqlite::types::ValueRef::Blob(_) => panic!("fixture query must not contain binary data"),
                })
            }).collect::<rusqlite::Result<Vec<_>>>()
        }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }).collect()
}

#[tokio::test]
async fn two_devices_pair_sync_multiple_batches_restart_and_converge() {
    let mut a = Device::new();
    let mut b = Device::new();
    pair(&a, &b, true).await.unwrap();
    let ctx = db::insert_context_returning(
        &a.db,
        "Fixture Context",
        Some("chat"),
        Some("formal"),
        None,
        Some("Fixture instructions"),
        false,
    )
    .unwrap();
    db::assign_context_website(&a.db, ctx.id, "fixture.example").unwrap();
    db::insert_dictionary_entry_returning(
        &a.db,
        "SyntheticTerm",
        Some("SyntheticMistake"),
        Some(ctx.id),
    )
    .unwrap();
    db::insert_snippet_returning(&a.db, "fixture", "Synthetic expansion", "", Some(ctx.id))
        .unwrap();
    let history = db::insert_transcription_returning(
        &a.db,
        "Synthetic history fixture",
        "Synthetic history fixture",
        3,
        1200,
        "fixture-model",
        Some("Fixture App"),
        Some(ctx.id),
    )
    .unwrap();
    a.db.lock().unwrap().execute(
        "INSERT INTO api_calls (transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at)
         VALUES (?1, 'fixture-model', 'fixture-provider', 'cleanup', 1200, 25, 25, datetime('now'))",
        [history.id],
    ).unwrap();
    for index in 0..(protocol::OPS_PER_BATCH + 9) {
        db::insert_dictionary_entry_returning(&a.db, &format!("FixtureTerm{index}"), None, None)
            .unwrap();
    }
    a.host
        .settings
        .lock()
        .unwrap()
        .insert("default_tone".into(), json!("formal"));
    a.host.stamps.lock().unwrap().insert(
        "default_tone".into(),
        (store::now_ms(), a.host.uuid.clone()),
    );
    session(&a, &b).await.unwrap();
    assert_eq!(
        count(&b.db.lock().unwrap(), "SELECT COUNT(*) FROM dictionary"),
        (protocol::OPS_PER_BATCH + 10) as i64
    );
    assert_eq!(count(&b.db.lock().unwrap(), "SELECT COUNT(*) FROM snippet_contexts sc JOIN contexts c ON c.id = sc.context_id WHERE c.name = 'Fixture Context'"), 1);
    assert_eq!(
        count(
            &b.db.lock().unwrap(),
            "SELECT COUNT(*) FROM dictionary_corrections"
        ),
        1
    );
    assert_eq!(count(&b.db.lock().unwrap(), "SELECT COUNT(*) FROM transcriptions t JOIN contexts c ON c.id = t.context_id WHERE c.name = 'Fixture Context'"), 1);
    assert_eq!(
        count(&b.db.lock().unwrap(), "SELECT COUNT(*) FROM api_calls"),
        1
    );
    assert_eq!(
        store::effective_lifetime_totals(&a.db.lock().unwrap()).unwrap(),
        store::effective_lifetime_totals(&b.db.lock().unwrap()).unwrap()
    );
    assert_eq!(
        b.host.settings.lock().unwrap().get("default_tone"),
        Some(&json!("formal"))
    );
    let cursor = store::peer_recv_cursor(&b.db.lock().unwrap(), &a.host.uuid).unwrap();
    assert!(cursor > 0);
    assert!(
        !store::peer_send_position(&a.db.lock().unwrap(), &b.host.uuid)
            .unwrap()
            .1
    );
    assert_eq!(logical_state(&a), logical_state(&b));

    a.restart_database();
    b.restart_database();
    assert_eq!(
        store::peer_recv_cursor(&b.db.lock().unwrap(), &a.host.uuid).unwrap(),
        cursor
    );
    db::insert_snippet_returning(&b.db, "offline", "Offline fixture", "", None).unwrap();
    db::update_context(&a.db, ctx.id, "Renamed fixture").unwrap();
    session(&b, &a).await.unwrap();
    assert_eq!(
        count(
            &a.db.lock().unwrap(),
            "SELECT COUNT(*) FROM snippets WHERE trigger = 'offline'"
        ),
        1
    );
    assert_eq!(
        count(
            &b.db.lock().unwrap(),
            "SELECT COUNT(*) FROM contexts WHERE name = 'Renamed fixture'"
        ),
        1
    );
    let log_before = store::max_log_seq(&a.db.lock().unwrap()).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    assert_eq!(
        store::max_log_seq(&a.db.lock().unwrap()).unwrap(),
        log_before,
        "repeated sync must not amplify the log"
    );
    assert_eq!(logical_state(&a), logical_state(&b));
}

#[tokio::test]
async fn two_devices_resume_an_interrupted_snapshot_without_duplicates() {
    let a = Device::new();
    let mut b = Device::new();
    pair(&a, &b, true).await.unwrap();
    for index in 0..(protocol::OPS_PER_BATCH + 5) {
        db::insert_dictionary_entry_returning(
            &b.db,
            &format!("InterruptedFixture{index}"),
            None,
            None,
        )
        .unwrap();
    }
    let (mut client, mut server) = connect(&a, &b).await.unwrap();
    let peer = store::get_peer(&a.db.lock().unwrap(), &b.host.uuid)
        .unwrap()
        .unwrap();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            engine::run_session(&a.db, &a.host, &mut client, true, &peer),
            async {
                assert!(matches!(
                    protocol::read_message(&mut server).await.unwrap(),
                    protocol::Message::Hello(_)
                ));
                protocol::send_message(
                    &mut server,
                    protocol::Message::HelloAck(protocol::Hello {
                        device_uuid: b.host.uuid.clone(),
                        device_name: "Fixture B".into(),
                        protocol: protocol::PROTOCOL_VERSION,
                        app_version: "test".into(),
                        activity_receipts: false,
                        cleanup_cache: false,
                        vocabulary_usage: false,
                        compressed_frames: false,
                    }),
                )
                .await
                .unwrap();
                assert!(matches!(
                    protocol::read_message(&mut server).await.unwrap(),
                    protocol::Message::Meta { .. }
                ));
                let stats =
                    engine::build_stats_exchange(&b.db.lock().unwrap(), &b.host.uuid).unwrap();
                protocol::send_message(
                    &mut server,
                    protocol::Message::Meta {
                        stats,
                        settings: vec![],
                    },
                )
                .await
                .unwrap();
                assert!(matches!(
                    protocol::read_message(&mut server).await.unwrap(),
                    protocol::Message::PullRequest(_)
                ));
                let (ops, cursor, done) = engine::collect_ops(
                    &b.db.lock().unwrap(),
                    0,
                    true,
                    protocol::OPS_PER_BATCH,
                    &mut engine::SnapshotProgress::default(),
                )
                .unwrap();
                assert!(!done);
                protocol::send_message(
                    &mut server,
                    protocol::Message::Ops(protocol::OpsBatch {
                        ops,
                        cursor,
                        done,
                        snapshot: true,
                    }),
                )
                .await
                .unwrap();
                assert!(matches!(
                    protocol::read_message(&mut server).await.unwrap(),
                    protocol::Message::Ack { .. }
                ));
                drop(server);
            },
        )
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert_eq!(
        store::peer_recv_cursor(&a.db.lock().unwrap(), &b.host.uuid).unwrap(),
        0
    );
    assert_eq!(
        count(&a.db.lock().unwrap(), "SELECT COUNT(*) FROM dictionary"),
        protocol::OPS_PER_BATCH as i64
    );
    b.restart_database();
    session(&a, &b).await.unwrap();
    session(&a, &b).await.unwrap();
    for device in [&a, &b] {
        assert_eq!(
            count(
                &device.db.lock().unwrap(),
                "SELECT COUNT(*) FROM dictionary"
            ),
            (protocol::OPS_PER_BATCH + 5) as i64
        );
    }
}

#[tokio::test]
async fn two_devices_wrong_pairing_code_does_not_create_trust() {
    let a = Device::new();
    let b = Device::new();
    assert!(pair(&a, &b, false).await.is_err());
    assert!(store::list_peers(&a.db.lock().unwrap()).unwrap().is_empty());
    assert!(store::list_peers(&b.db.lock().unwrap()).unwrap().is_empty());
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
}

#[tokio::test]
async fn two_devices_do_not_confirm_pairing_when_trust_cannot_be_saved() {
    let a = Device::new();
    let b = Device::new();
    assert!(pair_with_storage(&a, &b, true, false).await.is_err());
    assert!(store::list_peers(&a.db.lock().unwrap()).unwrap().is_empty());
    assert!(store::list_peers(&b.db.lock().unwrap()).unwrap().is_empty());
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
}

#[tokio::test]
async fn three_devices_relay_data_without_regressing_lifetime_counters() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    db::insert_transcription_returning(
        &a.db,
        "First synthetic fixture",
        "First synthetic fixture",
        3,
        1200,
        "fixture-model",
        None,
        None,
    )
    .unwrap();
    db::insert_dictionary_entry_returning(&a.db, "RelayFixture", None, None).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    db::insert_transcription_returning(
        &a.db,
        "Second synthetic fixture",
        "Second synthetic fixture",
        3,
        1200,
        "fixture-model",
        None,
        None,
    )
    .unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    for device in [&a, &b, &c] {
        assert_eq!(
            store::effective_lifetime_totals(&device.db.lock().unwrap())
                .unwrap()
                .0,
            6
        );
    }
    assert_eq!(logical_state(&a), logical_state(&c));
    session(&c, &b).await.unwrap();
    assert_eq!(logical_state(&a), logical_state(&b));
}

fn seed_device_edit(device: &Device, name: &str) -> i64 {
    let context = db::insert_context_returning(
        &device.db, name, None, None, None, Some("Synthetic mesh instructions"), false,
    ).unwrap();
    db::assign_context_website(&device.db, context.id, &format!("{name}.example")).unwrap();
    db::insert_dictionary_entry_returning(&device.db, &format!("Term{name}"), None, Some(context.id)).unwrap();
    db::insert_snippet_returning(&device.db, name, "Synthetic mesh expansion", "", Some(context.id)).unwrap();
    db::insert_transcription_returning(&device.db, "Three synthetic words", name, 3, 1200, "fixture-model", None, Some(context.id)).unwrap();
    context.id
}

#[tokio::test]
async fn three_device_chain_relays_bidirectional_edits_and_deletions() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let context = seed_device_edit(&a, "chain-a");
    seed_device_edit(&c, "chain-c");
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&b, &a).await.unwrap();
    assert_eq!(logical_state(&a), logical_state(&b));
    assert_eq!(logical_state(&a), logical_state(&c));
    assert!(store::get_peer(&a.db.lock().unwrap(), &c.host.uuid).unwrap().is_none(), "relay must not grant transitive pairing trust");
    tokio::time::sleep(Duration::from_millis(10)).await;
    db::delete_context(&a.db, context).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&c, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b, &c] {
        assert_eq!(count(&device.db.lock().unwrap(), "SELECT COUNT(*) FROM contexts WHERE name='chain-a'"), 0);
        assert_eq!(store::effective_lifetime_totals(&device.db.lock().unwrap()).unwrap().0, 6);
        assert_eq!(count(&device.db.lock().unwrap(), "SELECT COUNT(*) FROM transcriptions"), 2);
    }
    assert_eq!(logical_state(&a), logical_state(&c));
}

#[tokio::test]
async fn four_device_mesh_converges_after_concurrent_edits_offline_restart_and_stale_gossip() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    let mut d = Device::new();
    let devices = [&a, &b, &c, &d];
    for left in 0..4 {
        for right in left + 1..4 { pair(devices[left], devices[right], true).await.unwrap(); }
    }
    let context = seed_device_edit(&a, "mesh-a");
    seed_device_edit(&b, "mesh-b");
    seed_device_edit(&c, "mesh-c");
    seed_device_edit(&d, "mesh-d");
    let (ab, cd) = tokio::join!(session(&a, &b), session(&c, &d));
    ab.unwrap(); cd.unwrap();
    session(&b, &c).await.unwrap();
    session(&a, &c).await.unwrap();
    session(&a, &d).await.unwrap();
    session(&b, &d).await.unwrap();
    for device in [&b, &c, &d] { assert_eq!(logical_state(&a), logical_state(device)); }
    tokio::time::sleep(Duration::from_millis(10)).await;
    db::delete_context(&a.db, context).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    seed_device_edit(&d, "mesh-offline-d");
    d.restart_database();
    session(&d, &c).await.unwrap();
    session(&c, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    let devices = [&a, &b, &c, &d];
    for left in 0..4 {
        for right in left + 1..4 { session(devices[right], devices[left]).await.unwrap(); }
    }
    let sequences: Vec<_> = devices.iter().map(|device| store::max_log_seq(&device.db.lock().unwrap()).unwrap()).collect();
    for left in 0..4 {
        for right in left + 1..4 { session(devices[left], devices[right]).await.unwrap(); }
    }
    for (index, device) in devices.iter().enumerate() {
        assert_eq!(logical_state(&a), logical_state(device));
        assert_eq!(count(&device.db.lock().unwrap(), "SELECT COUNT(*) FROM contexts WHERE name='mesh-a'"), 0);
        assert_eq!(count(&device.db.lock().unwrap(), "SELECT COUNT(*) FROM transcriptions"), 5);
        assert_eq!(store::effective_lifetime_totals(&device.db.lock().unwrap()).unwrap().0, 15);
        assert_eq!(store::max_log_seq(&device.db.lock().unwrap()).unwrap(), sequences[index], "repeat gossip must not amplify changes");
    }
}

#[tokio::test]
async fn four_device_chain_relays_cleanup_cache_and_rejections() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    let d = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    pair(&c, &d, true).await.unwrap();

    let left = format!("cleanup-v3:{}", "a".repeat(64));
    let right = format!("cleanup-v3:{}", "b".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &left, "Synthetic result A", "2999-01-01 00:00:00", false).unwrap();
    db::cleanup_cache_insert_new(&d.db, &right, "Synthetic result D", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&c, &d).await.unwrap();
    session(&c, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b, &c, &d] {
        assert_eq!(db::cleanup_cache_get_active(&device.db, &left).unwrap().unwrap().clean_text, "Synthetic result A");
        assert_eq!(db::cleanup_cache_get_active(&device.db, &right).unwrap().unwrap().clean_text, "Synthetic result D");
    }

    db::cleanup_cache_delete_by_key(&a.db, &left).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    session(&c, &d).await.unwrap();
    session(&d, &c).await.unwrap();
    session(&c, &b).await.unwrap();
    session(&b, &a).await.unwrap();
    for device in [&a, &b, &c, &d] {
        assert!(db::cleanup_cache_get_active(&device.db, &left).unwrap().is_none());
        assert!(db::cleanup_cache_get_active(&device.db, &right).unwrap().is_some());
    }
}

#[tokio::test]
async fn disabled_cache_does_not_store_or_relay_entries_and_recovers_on_enable() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let existing = format!("cleanup-v3:{}", "e".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &existing, "Existing result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &b).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &existing).unwrap().is_some());
    db::cleanup_cache_set_enabled(&b.db, false).unwrap();
    let key = format!("cleanup-v3:{}", "c".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Synthetic result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &key).unwrap().is_none());
    assert!(db::cleanup_cache_get_active(&c.db, &key).unwrap().is_none());
    db::cleanup_cache_set_enabled(&b.db, true).unwrap();
    session(&a, &b).await.unwrap();
    session(&b, &c).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &existing).unwrap().is_some());
    assert!(db::cleanup_cache_get_active(&c.db, &existing).unwrap().is_some());
    assert_eq!(db::cleanup_cache_get_active(&c.db, &key).unwrap().unwrap().clean_text, "Synthetic result");
}

#[tokio::test]
async fn cache_created_before_identity_is_in_first_snapshot() {
    let a = Device::new();
    let b = Device::new();
    a.db.lock().unwrap().execute("DELETE FROM sync_identity", []).unwrap();
    let key = format!("cleanup-v3:{}", "f".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Early result", "2999-01-01 00:00:00", false).unwrap();
    store::ensure_self_identity(&a.db.lock().unwrap(), &a.host.uuid, "Fixture device").unwrap();
    pair(&a, &b, true).await.unwrap();
    session(&a, &b).await.unwrap();
    assert_eq!(db::cleanup_cache_get_active(&b.db, &key).unwrap().unwrap().clean_text, "Early result");
}

#[tokio::test]
async fn delayed_rejection_preserves_new_result_and_blocks_stale_copy() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let key = format!("cleanup-v3:{}", "a".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Old result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    db::cleanup_cache_insert_new(&a.db, &key, "New result", "2999-01-01 00:00:00", false).unwrap();
    db::cleanup_cache_reject(&a.db, &key, &db::cleanup_cache_result_hash("Old result")).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    session(&c, &b).await.unwrap();
    for device in [&a, &b, &c] {
        assert_eq!(db::cleanup_cache_get_active(&device.db, &key).unwrap().unwrap().clean_text, "New result");
    }
    db::cleanup_cache_insert_new(&c.db, &key, "Old result", "2999-01-01 00:00:00", false).unwrap();
    assert_eq!(db::cleanup_cache_get_active(&c.db, &key).unwrap().unwrap().clean_text, "New result");
}

#[tokio::test]
async fn expired_new_result_blocks_an_older_active_result() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let key = format!("cleanup-v3:{}", "b".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Old result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &c).await.unwrap();
    db::cleanup_cache_insert_new(&a.db, &key, "New result", "2999-01-01 00:00:00", false).unwrap();
    a.db.lock().unwrap().execute("UPDATE cleanup_cache SET expires_at_epoch=0 WHERE key=?1", [&key]).unwrap();
    session(&a, &b).await.unwrap();
    session(&c, &b).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &key).unwrap().is_none());
    assert!(db::cleanup_cache_get_active(&c.db, &key).unwrap().is_none());
    let watermark_count = |device: &Device| -> i64 {
        device.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sync_log WHERE table_name='cleanup_cache' AND row_uuid=?1", [&key], |r| r.get(0)).unwrap()
    };
    let before = (watermark_count(&b), watermark_count(&c));
    for _ in 0..3 {
        for device in [&b, &c] {
            device.db.lock().unwrap().execute("UPDATE sync_peers SET recv_cursor=0, needs_snapshot=1", []).unwrap();
        }
        session(&c, &b).await.unwrap();
    }
    assert_eq!((watermark_count(&b), watermark_count(&c)), before);
}

#[tokio::test]
async fn budget_eviction_preserves_newer_version_watermark() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let key = format!("cleanup-v3:{}", "9".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Old result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &c).await.unwrap();
    db::cleanup_cache_insert_new(&a.db, &key, "New result", "2999-01-01 00:00:00", false).unwrap();
    b.db.lock().unwrap().execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2000) INSERT INTO cleanup_cache(key, clean_text, created_at, last_hit_at, expires_at, created_at_epoch, last_hit_at_epoch, expires_at_epoch, is_snippet) SELECT 'local-'||x, 'Synthetic', datetime('now'), datetime('now'), datetime('now','+1 day'), strftime('%s','now'), strftime('%s','now'), strftime('%s','now','+1 day'), 1 FROM n").unwrap();
    session(&a, &b).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &key).unwrap().is_none());
    session(&c, &b).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &key).unwrap().is_none());
    assert!(db::cleanup_cache_get_active(&c.db, &key).unwrap().is_none());
}

#[test]
fn cache_pruning_compacts_superseded_operations_for_live_keys() {
    let a = Device::new();
    let key = format!("cleanup-v3:{}", "8".repeat(64));
    for output in ["First", "Second", "Third"] {
        db::cleanup_cache_insert_new(&a.db, &key, output, "2999-01-01 00:00:00", false).unwrap();
    }
    db::cleanup_cache_prune_expired(&a.db).unwrap();
    let conn = a.db.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM sync_log WHERE table_name='cleanup_cache' AND row_uuid=?1", [&key], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn stale_peer_does_not_restore_rejected_cache_entry() {
    let a = Device::new();
    let b = Device::new();
    let c = Device::new();
    pair(&a, &b, true).await.unwrap();
    pair(&a, &c, true).await.unwrap();
    pair(&b, &c, true).await.unwrap();
    let key = format!("cleanup-v3:{}", "d".repeat(64));
    db::cleanup_cache_insert_new(&a.db, &key, "Synthetic result", "2999-01-01 00:00:00", false).unwrap();
    session(&a, &b).await.unwrap();
    session(&a, &c).await.unwrap();
    db::cleanup_cache_delete_by_key(&a.db, &key).unwrap();
    session(&a, &b).await.unwrap();
    session(&c, &b).await.unwrap();
    assert!(db::cleanup_cache_get_active(&b.db, &key).unwrap().is_none());
    assert!(db::cleanup_cache_get_active(&c.db, &key).unwrap().is_none());
}
