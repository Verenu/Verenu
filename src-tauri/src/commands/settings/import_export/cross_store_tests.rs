use super::*;
use std::cell::Cell;

struct Fixture {
    dir: std::path::PathBuf,
    settings: store::SettingsHandle,
    db: crate::DbHandle,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("verenu-cross-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = store::SettingsHandle::empty_for_test(dir.join("settings.json"));
        settings
            .save_values([
                (store::CLEANUP_ENABLED, serde_json::json!(true)),
                (store::HISTORY_RETENTION, serde_json::json!("forever")),
            ])
            .unwrap();
        let db = db::open(dir.join("library.db").to_str().unwrap()).unwrap();
        db::insert_transcription_returning(
            &db,
            "synthetic old",
            "synthetic old",
            1,
            1000,
            "test",
            None,
            None,
        )
        .unwrap();
        db.lock()
            .unwrap()
            .execute(
                "UPDATE transcriptions SET created_at = datetime('now', '-100 days')",
                [],
            )
            .unwrap();
        Self { dir, settings, db }
    }

    fn payload(version: &str) -> ExportPayload {
        serde_json::from_value(serde_json::json!({
            "version": version, "app_version": "synthetic", "exported_at": "synthetic",
            "settings": { "cleanup_enabled": false, "history_retention": "7 days", "api_key": "fake-never-store", "setup_complete": true },
            "contexts": [],
            "dictionary": [{ "term": "SyntheticNew", "auto_learned": false, "confidence_tier": "high", "correction_count": 0, "created_at": "synthetic" }],
            "snippets": [{ "trigger": "synthetic-new", "expansion": "synthetic", "instructions": "", "created_at": "synthetic" }]
        })).unwrap()
    }

    fn run(&self, version: &str, notified: &Cell<bool>) -> Result<ImportSummary, String> {
        import_payload(
            &self.settings,
            &self.db,
            Self::payload(version),
            &[],
            |_, _| Ok(()),
            |effects| {
                notified.set(true);
                if let Some(days) = effects.history_prune_days {
                    db::prune_transcriptions_for_retention(&self.db, &self.settings, days).unwrap();
                }
            },
        )
    }

    fn library(&self) -> (Vec<ExportDictionaryEntry>, Vec<ExportContext>) {
        export_contextual_library(&self.db).unwrap()
    }

    fn history_count(&self) -> i64 {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM transcriptions", [], |r| r.get(0))
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn resolving_backup_alias_does_not_duplicate_overlapping_legacy_usage() {
    let source = Fixture::new();
    let dest = Fixture::new();
    {
        let conn = source.db.lock().unwrap();
        crate::sync::store::ensure_self_identity(&conn, "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", "source").unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        conn.execute("INSERT INTO transcription_hourly_stats SELECT '2020-01-01',10,id,5 FROM contexts WHERE name='Work'", []).unwrap();
        conn.execute("INSERT INTO provider_daily_stats SELECT '2020-01-01',id,'model','provider','stt',1,1,0,0 FROM contexts WHERE name='Work'", []).unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'ffffffff-ffff-4fff-8fff-ffffffffffff','new','new',2,1000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Work'", []).unwrap();
        conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,audio_ms,input_chars,output_chars,created_at,activity_day) SELECT '99999999-9999-4999-8999-999999999999',id,'model','provider','stt',1,0,0,'2020-01-01 10:00:00','2020-01-01' FROM transcriptions WHERE uuid='ffffffff-ffff-4fff-8fff-ffffffffffff'", []).unwrap();
    }
    {
        let conn = dest.db.lock().unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0})).unwrap();
    }
    let conn = source.db.lock().unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
    drop(conn);
    let summary = with_import_payload(&serde_json::to_string(&payload).unwrap(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    assert!(summary.activity_conflicts >= 2);
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT SUM(words) FROM transcriptions WHERE activity_day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls WHERE activity_day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn actual_context_import_resolves_existing_pending_activity() {
    let dest = Fixture::new();
    crate::commands::settings::import_sync_receipt(&dest.db.lock().unwrap(), "transcription_receipts", serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
    let mut value = serde_json::to_value(Fixture::payload("2")).unwrap();
    value["settings"] = serde_json::json!({});
    value["contexts"] = serde_json::json!([{"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"Work"}]);
    with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(conn.query_row("SELECT context_uuid FROM transcription_receipts WHERE uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, String>(0)).unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
}

#[test]
fn backup_alias_resolves_pending_history_before_importing_calls() {
    let source = Fixture::new();
    let dest = Fixture::new();
    {
        let conn = source.db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc','test','test',5,1000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Work'", []).unwrap();
        conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,audio_ms,input_chars,output_chars,created_at,activity_day) SELECT 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',id,'model','provider','stt',1,0,0,'2020-01-01 10:00:00','2020-01-01' FROM transcriptions WHERE uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", []).unwrap();
    }
    {
        let conn = dest.db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','test','test',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
        conn.execute("INSERT INTO pending_transcription_contexts VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
    }
    let conn = source.db.lock().unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
    drop(conn);
    with_import_payload(&serde_json::to_string(&payload).unwrap(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls WHERE uuid='dddddddd-dddd-4ddd-8ddd-dddddddddddd'", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
}

#[test]
fn named_context_import_resolves_existing_pending_activity() {
    let dest = Fixture::new();
    let conn = dest.db.lock().unwrap();
    conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
    crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
    drop(conn);
    let mut value = serde_json::to_value(Fixture::payload("2")).unwrap();
    value["settings"] = serde_json::json!({});
    value["contexts"] = serde_json::json!([{"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"Work"}]);
    with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(conn.query_row("SELECT context_uuid FROM transcription_receipts WHERE uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, String>(0)).unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
}

#[test]
fn deleting_mapped_context_preserves_history_provenance() {
    let dest = Fixture::new();
    dest.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
    let mut value = serde_json::to_value(Fixture::payload("2")).unwrap();
    value["settings"] = serde_json::json!({});
    value["contexts"] = serde_json::json!([{"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"Work"}]);
    value["activity"] = serde_json::json!({"lifetime":{"total_words":0,"dictionary_fixes":0,"wpm_sum":0.0,"wpm_count":0},"daily":[],"transcription_receipts":[{"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000}]});
    with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    let id: i64 = dest.db.lock().unwrap().query_row("SELECT id FROM contexts WHERE name='Work'", [], |r| r.get(0)).unwrap();
    db::delete_context(&dest.db, id).unwrap();
    let conn = dest.db.lock().unwrap();
    let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE is_everywhere=1", [], |r| r.get(0)).unwrap();
    assert_eq!(conn.query_row("SELECT local_uuid FROM backup_context_event_sources WHERE transcription_uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_,String>(0)).unwrap(), everywhere);
    let call = crate::sync::protocol::SyncOp { table:"call_receipts".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0})) };
    assert!(!crate::sync::engine::apply_ops(&conn, &[call]).unwrap().deferred);
    assert_eq!(conn.query_row("SELECT context_uuid FROM call_receipts WHERE uuid='dddddddd-dddd-4ddd-8ddd-dddddddddddd'", [], |r| r.get::<_,String>(0)).unwrap(), everywhere);
}

#[test]
fn backup_preserves_current_context_alias_independent_of_event_order() {
    let source = Fixture::new();
    let dest = Fixture::new();
    let conn = source.db.lock().unwrap();
    conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work'),('cccccccc-cccc-4ccc-8ccc-cccccccccccc','Office')", []).unwrap();
    for (uuid, context) in [("11111111-1111-4111-8111-111111111111","bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),("22222222-2222-4222-8222-222222222222","cccccccc-cccc-4ccc-8ccc-cccccccccccc")] {
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", serde_json::json!({"uuid":uuid,"day":"2020-01-01","hour":10,"context_uuid":context,"words":1,"spoken_words":null,"duration_ms":1000})).unwrap();
        conn.execute("INSERT INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) VALUES (?1,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',?2,?3)", params![uuid,context,if context.starts_with('b') {"Work"} else {"Office"}]).unwrap();
    }
    conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','cccccccc-cccc-4ccc-8ccc-cccccccccccc','Office')", []).unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
    drop(conn);
    with_import_payload(&serde_json::to_string(&payload).unwrap(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    assert_eq!(dest.db.lock().unwrap().query_row("SELECT local_uuid FROM backup_context_aliases WHERE source_uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, String>(0)).unwrap(), "cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    let conn = dest.db.lock().unwrap();
    let call = crate::sync::protocol::SyncOp { table:"call_receipts".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"11111111-1111-4111-8111-111111111111","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0})) };
    assert!(!crate::sync::engine::apply_ops(&conn, &[call]).unwrap().deferred);
    assert_eq!(conn.query_row("SELECT context_uuid FROM call_receipts WHERE uuid='dddddddd-dddd-4ddd-8ddd-dddddddddddd'", [], |r| r.get::<_,String>(0)).unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
}

#[test]
fn backup_alias_resolves_existing_pending_activity() {
    let source = Fixture::new();
    let dest = Fixture::new();
    {
        let conn = source.db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
    }
    crate::commands::settings::import_sync_receipt(&dest.db.lock().unwrap(), "transcription_receipts", serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
    let conn = source.db.lock().unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
    drop(conn);
    with_import_payload(&serde_json::to_string(&payload).unwrap(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(conn.query_row("SELECT context_uuid FROM transcription_receipts WHERE uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_,String>(0)).unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
}

#[test]
fn orphan_provider_context_provenance_survives_backup_and_source_deletion() {
    use crate::sync::{engine, protocol::SyncOp};
    for everywhere in [false, true] {
        let source = Fixture::new();
        let dest = Fixture::new();
        let third = Fixture::new();
        crate::sync::store::ensure_self_identity(&source.db.lock().unwrap(), "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", "source").unwrap();
        {
            let conn = source.db.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Work')", []).unwrap();
            crate::commands::settings::import_sync_receipt(&conn, "call_receipts", serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0})).unwrap();
        }
        if everywhere {
            dest.db.lock().unwrap().execute("UPDATE contexts SET name='Work' WHERE is_everywhere=1", []).unwrap();
        } else {
            dest.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        }
        let export = |fixture: &Fixture| {
            let conn = fixture.db.lock().unwrap();
            let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
            ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] }
        };
        for (from, to) in [(&source, &dest), (&dest, &third)] {
            with_import_payload(&serde_json::to_string(&export(from)).unwrap(), |p| import_payload(&to.settings, &to.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        }
        let id: i64 = source.db.lock().unwrap().query_row("SELECT id FROM contexts WHERE name='Work'", [], |r| r.get(0)).unwrap();
        db::delete_context(&source.db, id).unwrap();
        let (ops, _, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 1000, &mut engine::SnapshotProgress::default()).unwrap();
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:None };
        for target in [&dest, &third] {
            let conn = target.db.lock().unwrap();
            engine::apply_ops(&conn, &[deletion.clone()]).unwrap();
            for _ in 0..2 { assert!(!engine::apply_ops(&conn, &ops).unwrap().deferred); }
            assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }
}

#[test]
fn source_context_deletion_snapshot_preserves_mapped_history() {
    use crate::sync::{engine, protocol::SyncOp};
    for compacted in [false, true] {
        let source = Fixture::new();
        let dest = Fixture::new();
        crate::sync::store::ensure_self_identity(&source.db.lock().unwrap(), "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", "source").unwrap();
        {
            let conn = source.db.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Work')", []).unwrap();
            conn.execute("UPDATE transcriptions SET context_id=(SELECT id FROM contexts WHERE name='Work')", []).unwrap();
        }
        let parent: i64 = source.db.lock().unwrap().query_row("SELECT id FROM transcriptions LIMIT 1", [], |r| r.get(0)).unwrap();
        db::insert_api_calls(&source.db, &[db::ApiCall { transcription_id:parent, model:"synthetic".into(), provider:"test".into(), task:"stt".into(), audio_ms:1000, input_chars:0, output_chars:1, created_at:"2020-01-01 00:00:00".into() }]).unwrap();
        if compacted { db::prune_transcriptions_older_than(&source.db, 7).unwrap(); }
        dest.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        let conn = source.db.lock().unwrap();
        let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
        let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
        let context_id: i64 = conn.query_row("SELECT id FROM contexts WHERE name='Work'", [], |r| r.get(0)).unwrap();
        drop(conn);
        with_import_payload(&serde_json::to_string(&payload).unwrap(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        source.db.lock().unwrap().execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) VALUES ('dddddddd-dddd-4ddd-8ddd-dddddddddddd','new','new',2,1000,'test',?1,'2020-01-02 12:00:00','2020-01-02',12)", [context_id]).unwrap();
        if compacted { db::prune_transcriptions_older_than(&source.db, 7).unwrap(); }
        let (new_ops, _, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 1000, &mut engine::SnapshotProgress::default()).unwrap();
        assert!(!engine::apply_ops(&dest.db.lock().unwrap(), &new_ops).unwrap().deferred);
        let third = Fixture::new();
        third.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('ffffffff-ffff-4fff-8fff-ffffffffffff','Work')", []).unwrap();
        let conn = dest.db.lock().unwrap();
        let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
        let relayed = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
        drop(conn);
        with_import_payload(&serde_json::to_string(&relayed).unwrap(), |p| import_payload(&third.settings, &third.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        db::delete_context(&source.db, context_id).unwrap();
        let (ops, _, _) = engine::collect_ops(&source.db.lock().unwrap(), 0, true, 1000, &mut engine::SnapshotProgress::default()).unwrap();
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:None };
        for target in [&dest, &third] {
            let conn = target.db.lock().unwrap();
            engine::apply_ops(&conn, &[deletion.clone()]).unwrap();
            for _ in 0..2 { assert!(!engine::apply_ops(&conn, &ops).unwrap().deferred); }
            assert!(conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_context_daily_stats WHERE context_uuid=(SELECT uuid FROM contexts WHERE name='Work') AND total_words>0)", [], |r| r.get::<_, bool>(0)).unwrap());
            assert_eq!(conn.query_row("SELECT (SELECT COUNT(*) FROM api_calls)+(SELECT COALESCE(SUM(calls),0) FROM provider_daily_stats)", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }
}

#[test]
fn invalid_context_identity_rejects_backup_without_persisting_aliases() {
    let dest = Fixture::new();
    dest.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
    let mut value = serde_json::to_value(Fixture::payload("2")).unwrap();
    value["contexts"] = serde_json::json!([{"uuid":"invalid-context-id","name":"Work"}]);
    assert!(with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).is_err());
    assert_eq!(dest.db.lock().unwrap().query_row("SELECT COUNT(*) FROM backup_context_aliases", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn restored_named_context_survives_collision_and_sync_replay() {
    use crate::sync::{engine, protocol::SyncOp};
    for (compacted, timestamp) in [(false, 1), (true, 1), (false, i64::MAX), (true, i64::MAX)] {
        let dest = Fixture::new();
        dest.db.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        dest.db.lock().unwrap().execute("UPDATE transcriptions SET context_id=(SELECT id FROM contexts WHERE name='Work')", []).unwrap();
        let before: i64 = dest.db.lock().unwrap().query_row("SELECT COALESCE(SUM(total_words),0) FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get(0)).unwrap();
        let mut value = serde_json::to_value(Fixture::payload("2")).unwrap();
        value["settings"] = serde_json::json!({});
        value["contexts"] = serde_json::json!([{"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"Work"}]);
        let receipt = serde_json::json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":12,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000});
        let history = serde_json::json!({"raw_text":"synthetic","clean_text":"synthetic","words":5,"spoken_words":null,"duration_ms":1000,"api_used":"test","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","created_at":"2020-01-01 12:00:00","activity_day":"2020-01-01","activity_hour":12});
        if compacted {
            value["activity"] = serde_json::json!({"lifetime":{"total_words":0,"dictionary_fixes":0,"wpm_sum":0.0,"wpm_count":0},"daily":[],"transcription_receipts":[receipt.clone()]});
        } else {
            let mut record = history.clone();
            record["uuid"] = serde_json::json!("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
            value["history"] = serde_json::json!({"transcriptions":[record],"calls":[]});
        }
        with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        dest.db.lock().unwrap().execute("UPDATE contexts SET name='Personal' WHERE uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", []).unwrap();
        with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        assert_eq!(dest.db.lock().unwrap().query_row("SELECT COUNT(*) FROM contexts WHERE uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let conn = dest.db.lock().unwrap();
        let context = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"upsert".into(), ts_ms:timestamp, origin:"peer".into(), origin_seq:1, payload:Some(serde_json::json!({"name":"Work","created_at":"2020-01-01","updated_at":"2020-01-01"})) };
        engine::apply_ops(&conn, &[context]).unwrap();
        drop(conn);
        with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        let conn = dest.db.lock().unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM contexts WHERE uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let event = SyncOp { table:if compacted {"transcription_receipts"} else {"transcriptions"}.into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:2, payload:Some(if compacted {receipt} else {history}) };
        for _ in 0..2 { assert!(!engine::apply_ops(&conn, &[event.clone()]).unwrap().deferred); }
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_, i64>(0)).unwrap(), before + 5);
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:3, payload:None };
        engine::apply_ops(&conn, &[deletion]).unwrap();
        assert!(!engine::apply_ops(&conn, &[event.clone()]).unwrap().deferred);
        drop(conn);
        value["contexts"][0]["name"] = serde_json::json!("Office");
        value["history"] = serde_json::Value::Null;
        value["activity"] = serde_json::Value::Null;
        with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).unwrap();
        let conn = dest.db.lock().unwrap();
        let new_receipt = SyncOp { table:"transcription_receipts".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:4, payload:Some(serde_json::json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-02","hour":12,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":7,"spoken_words":null,"duration_ms":1000})) };
        assert!(!engine::apply_ops(&conn, &[new_receipt]).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_context_daily_stats WHERE context_uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_, i64>(0)).unwrap(), before + 5);
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:9, payload:None };
        engine::apply_ops(&conn, &[deletion]).unwrap();
        let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE is_everywhere=1", [], |r| r.get(0)).unwrap();
        let mut moved = event;
        moved.payload.as_mut().unwrap()["context_uuid"] = serde_json::json!(everywhere);
        assert!(!engine::apply_ops(&conn, &[moved]).unwrap().deferred);
    }
}

#[test]
fn retained_and_compacted_hourly_overflow_rolls_back() {
    let dest = Fixture::new();
    dest.db.lock().unwrap().execute("INSERT INTO transcription_hourly_stats(day,context_id,hour,total_words) VALUES ('2019-12-31',0,12,?1)", [i64::MAX]).unwrap();
    let mut payload = Fixture::payload("2");
    payload.settings = serde_json::json!({});
    let mut value = serde_json::to_value(payload).unwrap();
    value["history"] = serde_json::json!({"transcriptions":[{"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","raw_text":"synthetic","clean_text":"synthetic","words":1,"spoken_words":1,"duration_ms":1000,"api_used":"test","created_at":"2020-01-01 12:00:00","activity_day":"2020-01-01","activity_hour":12}],"calls":[]});
    assert!(with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).is_err());
    assert_eq!(dest.history_count(), 1);
}

#[test]
fn retained_provider_overflow_rolls_back() {
    for compacted in [false, true] {
        let dest = Fixture::new();
        if compacted {
            dest.db.lock().unwrap().execute("INSERT INTO provider_daily_stats VALUES ('2019-12-31', 0, 'synthetic', 'test', 'stt', 1, ?1, 0, 0)", [i64::MAX]).unwrap();
        }
        let mut payload = Fixture::payload("2");
        payload.settings = serde_json::json!({});
        let mut value = serde_json::to_value(payload).unwrap();
        let parent = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let call = |uuid: &str| serde_json::json!({"uuid":uuid,"transcription_uuid":parent,"model":"synthetic","provider":"test","task":"stt","audio_ms":i64::MAX,"input_chars":0,"output_chars":0,"created_at":"2020-01-01 12:00:00","activity_day":"2020-01-01"});
        let mut calls = vec![call("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")];
        if !compacted { calls.push(call("cccccccc-cccc-4ccc-8ccc-cccccccccccc")); }
        value["history"] = serde_json::json!({"transcriptions":[{"uuid":parent,"raw_text":"synthetic","clean_text":"synthetic","words":1,"spoken_words":1,"duration_ms":1000,"api_used":"test","created_at":"2020-01-01 12:00:00","activity_day":"2020-01-01","activity_hour":12}],"calls":calls});
        assert!(with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).is_err());
        assert_eq!(dest.history_count(), 1);
        assert_eq!(dest.db.lock().unwrap().query_row("SELECT count(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
}

#[test]
fn history_only_backup_overflow_rolls_back() {
    let dest = Fixture::new();
    dest.settings.save_value(store::HISTORY_RETENTION, serde_json::json!("Forever")).unwrap();
    let original = dest.history_count();
    let mut payload = Fixture::payload("2");
    payload.settings = serde_json::json!({});
    let record = |uuid: &str| serde_json::json!({"uuid":uuid, "raw_text":"synthetic", "clean_text":"synthetic", "words":i64::MAX, "spoken_words":1, "duration_ms":1000, "api_used":"test", "created_at":"2020-01-01 12:00:00", "activity_day":"2020-01-01", "activity_hour":12});
    let mut value = serde_json::to_value(payload).unwrap();
    value["history"] = serde_json::json!({"transcriptions":[record("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),record("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")],"calls":[]});
    assert!(with_import_payload(&value.to_string(), |p| import_payload(&dest.settings, &dest.db, p, &[], |_, _| Ok(()), |_| {})).is_err());
    assert_eq!(dest.history_count(), original);
}

#[test]
fn import_applies_existing_retention_when_backup_omits_the_setting() {
    let source = Fixture::new();
    let dest = Fixture::new();
    dest.settings.save_value(store::HISTORY_RETENTION, serde_json::json!("7 days")).unwrap();
    let conn = source.db.lock().unwrap();
    let uuid: String = conn.query_row("SELECT uuid FROM transcriptions LIMIT 1", [], |r| r.get(0)).unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload { version:"3".into(), app_version:"test".into(), exported_at:"test".into(), stats:ExportStats::default(), activity:Some(backup_stats::export_activity_conn(&conn).unwrap()), history:Some(backup_history::export_history_conn(&conn).unwrap()), settings:serde_json::json!({}), dictionary, contexts, snippets:vec![], sub_apps:vec![] };
    drop(conn);
    let json = serde_json::to_string(&payload).unwrap();
    for _ in 0..2 {
        with_import_payload(&json, |payload| import_payload(&dest.settings, &dest.db, payload, &[], |_, _| Ok(()), |_| {})).unwrap();
    }
    let conn = dest.db.lock().unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions WHERE uuid=?1", [&uuid], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts WHERE uuid=?1", [&uuid], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT SUM(total_transcriptions) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
}

#[test]
fn serialized_backup_merges_independent_history_and_is_idempotent() {
    let source = Fixture::new();
    let dest = Fixture::new();
    crate::sync::store::ensure_self_identity(&source.db.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
    crate::sync::store::ensure_self_identity(&dest.db.lock().unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "destination").unwrap();
    let transcription_id: i64 = source.db.lock().unwrap().query_row("SELECT id FROM transcriptions LIMIT 1", [], |r| r.get(0)).unwrap();
    db::insert_api_calls(&source.db, &[db::ApiCall { transcription_id, model: "synthetic".into(), provider: "test".into(), task: "stt".into(), audio_ms: 1000, input_chars: 0, output_chars: 9, created_at: "2020-01-01 00:00:00".into() }]).unwrap();
    let source_before = db::query_stats(&source.db).unwrap().total_words;
    let dest_before = db::query_stats(&dest.db).unwrap().total_words;
    let conn = source.db.lock().unwrap();
    let (dictionary, contexts) = export_contextual_library_conn(&conn).unwrap();
    let payload = ExportPayload {
        version: "3".into(), app_version: "test".into(), exported_at: "test".into(),
        stats: ExportStats::default(), activity: Some(backup_stats::export_activity_conn(&conn).unwrap()),
        history: Some(backup_history::export_history_conn(&conn).unwrap()), settings: serde_json::json!({}),
        dictionary, contexts, snippets: vec![], sub_apps: export_unassigned_sub_apps_conn(&conn).unwrap(),
    };
    drop(conn);
    let json = serde_json::to_string(&payload).unwrap();
    let mut malformed = serde_json::to_value(&payload).unwrap();
    malformed["history"]["transcriptions"][0]["words"] = serde_json::json!(-1);
    assert!(with_import_payload(&malformed.to_string(), |payload| import_payload(&dest.settings, &dest.db, payload, &[], |_, _| Ok(()), |_| {})).is_err());
    assert_eq!(dest.history_count(), 1);
    assert_eq!(db::query_stats(&dest.db).unwrap().total_words, dest_before);
    for expected_inserted in [1, 0] {
        let summary = with_import_payload(&json, |payload| import_payload(&dest.settings, &dest.db, payload, &[], |_, _| Ok(()), |_| {})).unwrap();
        assert_eq!(summary.history_inserted, expected_inserted);
    }
    assert_eq!(dest.history_count(), 2);
    assert_eq!(dest.db.lock().unwrap().query_row("SELECT count(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db::query_stats(&dest.db).unwrap().total_words, source_before + dest_before);
    let exchange = crate::sync::engine::build_stats_exchange(&source.db.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    crate::sync::engine::apply_stats_exchange(&dest.db.lock().unwrap(), &exchange, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    assert_eq!(db::query_stats(&dest.db).unwrap().total_words, source_before + dest_before);
    crate::sync::store::remove_remote_stats(&dest.db.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    assert_eq!(db::query_stats(&dest.db).unwrap().total_words, source_before + dest_before);
    dest.settings.save_value(store::HISTORY_RETENTION, serde_json::json!("7 days")).unwrap();
    db::prune_transcriptions_for_retention(&dest.db, &dest.settings, 7).unwrap();
    assert_eq!(dest.history_count(), 0);
    let summary = with_import_payload(&json, |payload| import_payload(&dest.settings, &dest.db, payload, &[], |_, _| Ok(()), |_| {})).unwrap();
    assert_eq!(summary.history_existing, 1);
    assert_eq!(summary.history_conflicts, 0);
    assert_eq!(db::query_stats(&dest.db).unwrap().total_words, source_before + dest_before);
}

#[test]
fn legacy_app_mappings_match_local_apps_and_preserve_existing_mappings() {
    let f = Fixture::new();
    f.settings.save_value(store::APP_MAPPINGS, serde_json::json!([{ "exe": "slack", "profile": "formal", "name": "Slack" }])).unwrap();
    let mut payload = Fixture::payload("2");
    payload.settings = serde_json::json!({"app_mappings": [
        {"exe": "word.exe", "profile": "casual", "name": "Microsoft Word"},
        {"exe": "unknown.exe", "profile": "casual", "name": "Unknown Product"}
    ]});
    let installed = [crate::system::apps::InstalledApp { name: "Microsoft Word".into(), exe: "com.microsoft.word".into(), developer: None }];
    let summary = import_payload(&f.settings, &f.db, payload, &installed, |_, _| Ok(()), |_| {}).unwrap();
    let mappings = f.settings.get(store::APP_MAPPINGS).unwrap();
    assert_eq!(mappings.as_array().unwrap().len(), 2);
    assert!(mappings.as_array().unwrap().iter().any(|m| m["exe"] == "com.microsoft.word"));
    assert_eq!(summary.app_targets_dropped, 1);
}

#[test]
fn concurrent_key_for_old_endpoint_cannot_survive_import_publication() {
    let f = Fixture::new();
    let provider = serde_json::json!({
        "id": "custom:11111111-1111-4111-8111-111111111111", "name": "Synthetic",
        "protocol": "openai", "base_url": "https://old.example.invalid/v1", "supports_cleanup": true
    });
    f.settings
        .save_value(store::CUSTOM_PROVIDERS, serde_json::json!([provider]))
        .unwrap();
    let mut payload = Fixture::payload("2");
    let mut next = provider.clone();
    next["base_url"] = serde_json::json!("https://new.example.invalid/v1");
    payload.settings[store::CUSTOM_PROVIDERS] = serde_json::json!([next]);
    let expected = custom_endpoint(
        &f.settings.snapshot().unwrap(),
        provider["id"].as_str().unwrap(),
    )
    .unwrap();
    let fake_key = std::sync::Arc::new(std::sync::Mutex::new(Some("fake-A")));
    let (cleared_tx, cleared_rx) = std::sync::mpsc::channel();
    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let settings = f.settings.clone();
    let key_for_writer = fake_key.clone();
    let writer = std::thread::spawn(move || {
        cleared_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        attempted_tx.send(()).unwrap();
        with_custom_endpoint(&settings, &expected.id, Some(&expected), || {
            *key_for_writer.lock().unwrap() = Some("fake-A");
            Ok(())
        })
    });
    import_payload(
        &f.settings,
        &f.db,
        payload,
        &[],
        |previous, next| {
            clear_changed_provider_keys(previous, next, |_| {
                *fake_key.lock().unwrap() = None;
                Ok(())
            })?;
            cleared_tx.send(()).unwrap();
            attempted_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            Ok(())
        },
        |_| {},
    )
    .unwrap();
    assert!(
        writer.join().unwrap().is_err(),
        "queued key write must reject changed endpoint"
    );
    assert!(
        fake_key.lock().unwrap().is_none(),
        "old endpoint key survived publication of new endpoint"
    );
}

#[test]
fn relaxed_retention_before_import_completion_preserves_history() {
    let f = Fixture::new();
    import_payload(
        &f.settings,
        &f.db,
        Fixture::payload("2"),
        &[],
        |_, _| Ok(()),
        |effects| {
            f.settings
                .save_value(store::HISTORY_RETENTION, serde_json::json!("Forever"))
                .unwrap();
            db::prune_transcriptions_for_retention(
                &f.db,
                &f.settings,
                effects.history_prune_days.unwrap(),
            )
            .unwrap();
        },
    )
    .unwrap();
    assert_eq!(
        f.history_count(),
        1,
        "stale import retention pruned history after Forever was saved"
    );
}

#[test]
fn import_compares_latest_endpoint_after_database_wait() {
    let f = Fixture::new();
    let provider = serde_json::json!({
        "id": "custom:11111111-1111-4111-8111-111111111111", "name": "Synthetic",
        "protocol": "openai", "base_url": "https://old.example.invalid/v1", "supports_cleanup": true
    });
    f.settings
        .save_value(store::CUSTOM_PROVIDERS, serde_json::json!([provider]))
        .unwrap();
    let mut payload = Fixture::payload("2");
    let mut next = provider.clone();
    next["base_url"] = serde_json::json!("https://import.example.invalid/v1");
    payload.settings[store::CUSTOM_PROVIDERS] = serde_json::json!([next]);
    let fake_key = std::sync::Arc::new(std::sync::Mutex::new(Some("fake-A")));
    let db_guard = f.db.lock().unwrap();
    let db = f.db.clone();
    let settings = f.settings.clone();
    let key_for_import = fake_key.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let importer = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        import_payload(
            &settings,
            &db,
            payload,
            &[],
            |previous, next| {
                assert_eq!(
                    custom_endpoint(previous, &next[0].id).unwrap().base_url,
                    "https://intermediate.example.invalid/v1"
                );
                clear_changed_provider_keys(previous, next, |_| {
                    *key_for_import.lock().unwrap() = None;
                    Ok(())
                })
            },
            |_| {},
        )
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let mut intermediate = provider;
    intermediate["base_url"] = serde_json::json!("https://intermediate.example.invalid/v1");
    let intermediate =
        crate::api::custom::normalize_list(&serde_json::json!([intermediate])).unwrap();
    f.settings
        .save_values_with_commit(
            [(store::CUSTOM_PROVIDERS, serde_json::json!(intermediate))],
            |previous| {
                clear_changed_provider_keys(previous, &intermediate, |_| {
                    *fake_key.lock().unwrap() = None;
                    Ok(())
                })
            },
            || Ok(()),
        )
        .unwrap();
    with_custom_endpoint(
        &f.settings,
        &intermediate[0].id,
        Some(&intermediate[0]),
        || {
            *fake_key.lock().unwrap() = Some("fake-C");
            Ok(())
        },
    )
    .unwrap();
    drop(db_guard);
    importer.join().unwrap().unwrap();
    assert!(
        fake_key.lock().unwrap().is_none(),
        "latest endpoint key was not cleared"
    );
    assert_eq!(
        custom_endpoint(&f.settings.snapshot().unwrap(), &intermediate[0].id)
            .unwrap()
            .base_url,
        "https://import.example.invalid/v1"
    );
}

#[test]
fn valid_import_settings_write_failure_preserves_stores() {
    let f = Fixture::new();
    let disk = std::fs::read(f.dir.join("settings.json")).unwrap();
    let library = f.library();
    std::fs::create_dir(f.dir.join("settings.json.tmp")).unwrap();
    let notified = Cell::new(false);
    assert!(f.run("2", &notified).is_err());
    assert_eq!(
        f.settings.get(store::CLEANUP_ENABLED),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        f.settings.get(store::HISTORY_RETENTION),
        Some(serde_json::json!("forever"))
    );
    assert_eq!(std::fs::read(f.dir.join("settings.json")).unwrap(), disk);
    assert_eq!(f.library(), library);
    assert_eq!(f.history_count(), 1);
    assert!(!notified.get());
}

#[test]
fn valid_import_library_commit_failure_preserves_stores() {
    let f = Fixture::new();
    std::fs::write(
        f.dir.join("settings.json"),
        b"{\"cleanup_enabled\":true,\"history_retention\":\"forever\"}\n",
    )
    .unwrap();
    let disk = std::fs::read(f.dir.join("settings.json")).unwrap();
    let library = f.library();
    f.db.lock().unwrap().execute_batch("PRAGMA foreign_keys = ON;
        CREATE TABLE synthetic_parent (id INTEGER PRIMARY KEY);
        CREATE TABLE synthetic_child (parent_id INTEGER REFERENCES synthetic_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER synthetic_commit_failure AFTER INSERT ON snippets BEGIN
            INSERT INTO synthetic_child VALUES (123);
        END;").unwrap();
    let notified = Cell::new(false);
    let error = f.run("2", &notified).err().expect("commit must fail");
    assert!(error.contains("constraint"), "{error}");
    assert_eq!(
        f.settings.get(store::CLEANUP_ENABLED),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        f.settings.get(store::HISTORY_RETENTION),
        Some(serde_json::json!("forever"))
    );
    assert_eq!(std::fs::read(f.dir.join("settings.json")).unwrap(), disk);
    assert_eq!(f.library(), library);
    assert_eq!(f.history_count(), 1);
    assert!(!notified.get());
}

#[test]
fn valid_import_success_completes_after_commit_for_compatible_versions() {
    for version in ["1", "2"] {
        let f = Fixture::new();
        let observer = Connection::open(f.dir.join("library.db")).unwrap();
        let notified = Cell::new(false);
        let summary = import_payload(
            &f.settings,
            &f.db,
            Fixture::payload(version),
            &[],
            |_, _| Ok(()),
            |effects| {
                assert_eq!(
                    observer
                        .query_row(
                            "SELECT count(*) FROM snippets WHERE trigger = 'synthetic-new'",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    1,
                    "completion happened before library commit"
                );
                assert_eq!(
                    f.settings.get(store::CLEANUP_ENABLED),
                    Some(serde_json::json!(false))
                );
                let disk: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(f.dir.join("settings.json")).unwrap())
                        .unwrap();
                assert_eq!(disk[store::CLEANUP_ENABLED], false);
                assert_eq!(f.history_count(), 1, "history pruned before completion");
                assert!(f.library().0.iter().any(|e| e.term == "SyntheticNew"));
                db::prune_transcriptions_for_retention(
                    &f.db,
                    &f.settings,
                    effects.history_prune_days.unwrap(),
                )
                .unwrap();
                notified.set(true);
            },
        )
        .unwrap();
        assert!(notified.get());
        assert_eq!(f.history_count(), 0);
        assert_eq!(summary.snippets_inserted, 1);
        assert_eq!(summary.settings_applied, 2);
        assert_eq!(summary.settings_skipped, 2);
        assert!(f.settings.get("api_key").is_none());
        assert!(f.settings.get(store::SETUP_COMPLETE).is_none());
    }
}

#[test]
fn valid_import_library_transaction_begin_failure_preserves_stores() {
    let f = Fixture::new();
    let disk = std::fs::read(f.dir.join("settings.json")).unwrap();
    let library = f.library();
    f.db.lock().unwrap().execute_batch("BEGIN").unwrap();
    let notified = Cell::new(false);
    assert!(f
        .run("1", &notified)
        .err()
        .expect("transaction begin must fail")
        .contains("within a transaction"));
    f.db.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        f.settings.get(store::CLEANUP_ENABLED),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        f.settings.get(store::HISTORY_RETENTION),
        Some(serde_json::json!("forever"))
    );
    assert_eq!(std::fs::read(f.dir.join("settings.json")).unwrap(), disk);
    assert_eq!(f.library(), library);
    assert_eq!(f.history_count(), 1);
    assert!(!notified.get());
}

#[test]
fn valid_import_changed_endpoint_never_restores_fake_credentials() {
    for fail_commit in [false, true] {
        let f = Fixture::new();
        let provider = serde_json::json!({
            "id": "custom:11111111-1111-4111-8111-111111111111", "name": "Synthetic",
            "protocol": "openai", "base_url": "https://old.example.invalid/v1", "supports_cleanup": true
        });
        f.settings
            .save_value(store::CUSTOM_PROVIDERS, serde_json::json!([provider]))
            .unwrap();
        let disk = std::fs::read(f.dir.join("settings.json")).unwrap();
        let mut payload = Fixture::payload("2");
        let mut restored = provider.clone();
        restored["base_url"] = serde_json::json!("https://new.example.invalid/v1");
        payload.settings[store::CUSTOM_PROVIDERS] = serde_json::json!([restored]);
        if fail_commit {
            f.db.lock().unwrap().execute_batch("PRAGMA foreign_keys = ON;
                CREATE TABLE synthetic_parent (id INTEGER PRIMARY KEY);
                CREATE TABLE synthetic_child (parent_id INTEGER REFERENCES synthetic_parent(id) DEFERRABLE INITIALLY DEFERRED);
                CREATE TRIGGER synthetic_commit_failure AFTER INSERT ON snippets BEGIN INSERT INTO synthetic_child VALUES (123); END;").unwrap();
        }
        let fake_key = Cell::new(Some("fake-key-only"));
        let notified = Cell::new(false);
        let result = import_payload(
            &f.settings,
            &f.db,
            payload,
            &[],
            |_, next| {
                assert_eq!(next[0].base_url, "https://new.example.invalid/v1");
                fake_key.set(None);
                Ok(())
            },
            |_| {
                assert!(fake_key.get().is_none());
                notified.set(true);
            },
        );
        assert_eq!(result.is_err(), fail_commit);
        assert_eq!(notified.get(), !fail_commit);
        assert!(
            fake_key.get().is_none(),
            "a cleared key must never be restored"
        );
        if fail_commit {
            assert_eq!(std::fs::read(f.dir.join("settings.json")).unwrap(), disk);
            assert_eq!(
                f.settings.get(store::CUSTOM_PROVIDERS),
                Some(serde_json::json!([provider]))
            );
        }
    }
}

#[test]
fn valid_import_commit_failure_preserves_absent_settings_file() {
    let f = Fixture::new();
    std::fs::remove_file(f.dir.join("settings.json")).unwrap();
    f.db.lock().unwrap().execute_batch("PRAGMA foreign_keys = ON;
        CREATE TABLE synthetic_parent (id INTEGER PRIMARY KEY);
        CREATE TABLE synthetic_child (parent_id INTEGER REFERENCES synthetic_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER synthetic_commit_failure AFTER INSERT ON snippets BEGIN INSERT INTO synthetic_child VALUES (123); END;").unwrap();
    let library = f.library();
    let notified = Cell::new(false);
    assert!(f.run("2", &notified).is_err());
    assert!(!f.dir.join("settings.json").exists());
    assert_eq!(
        f.settings.get(store::CLEANUP_ENABLED),
        Some(serde_json::json!(true))
    );
    assert_eq!(f.library(), library);
    assert_eq!(f.history_count(), 1);
    assert!(!notified.get());
}
