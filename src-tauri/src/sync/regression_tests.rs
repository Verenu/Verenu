//! Regression cases use synthetic data and isolated device databases.
use super::*;

#[tokio::test]
async fn approval_reader_uses_the_requested_deadline() {
    use super::super::protocol::{read_message_with_timeout, send_message};
    let (mut reader, mut writer) = tokio::io::duplex(4096);
    let (result, ()) = tokio::join!(
        read_message_with_timeout(&mut reader, std::time::Duration::from_secs(1)),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            send_message(&mut writer, &Message::PairBusy).await.unwrap();
        },
    );
    assert!(matches!(result.unwrap(), Message::PairBusy));
    let (_writer, mut reader) = tokio::io::duplex(4096);
    assert!(
        read_message_with_timeout(&mut reader, std::time::Duration::from_millis(20))
            .await
            .is_err()
    );
}

#[test]
fn stale_third_device_counters_cannot_reduce_lifetime_totals() {
    let db = test_db(&uuid("counter-receiver"));
    let conn = db.lock().unwrap();
    let mut stats = sync_store::DeviceStats {
        device_id: uuid("counter-origin"),
        total_words: 100,
        dictionary_fixes: 9,
    };
    sync_store::upsert_remote_stats(&conn, &stats).unwrap();
    stats.total_words = 60;
    stats.dictionary_fixes = 4;
    sync_store::upsert_remote_stats(&conn, &stats).unwrap();
    assert_eq!(
        sync_store::effective_lifetime_totals(&conn).unwrap(),
        (100, 9)
    );
}

#[tokio::test]
async fn session_rejects_a_different_peer_uuid() {
    let a = test_db(&uuid("hello-a"));
    let host = TestHost::new(&uuid("hello-a"));
    let peer = peer_of(&uuid("hello-b"));
    let (mut client, mut server) = tokio::io::duplex(4096);
    let (result, ()) = tokio::join!(
        engine::run_session(&a, &host, &mut client, true, &peer),
        async {
            super::super::protocol::read_message(&mut server)
                .await
                .unwrap();
            super::super::protocol::send_message(
                &mut server,
                &Message::HelloAck(super::super::protocol::Hello {
                    device_uuid: uuid("impostor"),
                    device_name: "Fixture".into(),
                    protocol: super::super::protocol::PROTOCOL_VERSION,
                    app_version: "test".into(),
                }),
            )
            .await
            .unwrap();
        },
    );
    assert!(result.unwrap_err().to_string().contains("identity"));
}

#[tokio::test]
async fn session_rejects_completion_before_final_batch() {
    let a = test_db(&uuid("early-a"));
    let b = test_db(&uuid("early-b"));
    let host = TestHost::new(&uuid("early-a"));
    pair_test_dbs(&a, &b, &host.uuid, &uuid("early-b"));
    let peer = peer_of(&uuid("early-b"));
    let (mut client, mut server) = tokio::io::duplex(4096);
    let (result, ()) = tokio::join!(
        engine::run_session(&a, &host, &mut client, true, &peer),
        async {
            super::super::protocol::read_message(&mut server)
                .await
                .unwrap();
            super::super::protocol::send_message(
                &mut server,
                &Message::HelloAck(super::super::protocol::Hello {
                    device_uuid: uuid("early-b"),
                    device_name: "Fixture".into(),
                    protocol: super::super::protocol::PROTOCOL_VERSION,
                    app_version: "test".into(),
                }),
            )
            .await
            .unwrap();
            super::super::protocol::read_message(&mut server)
                .await
                .unwrap();
            let stats = engine::build_stats_exchange(&b.lock().unwrap(), &uuid("early-b")).unwrap();
            super::super::protocol::send_message(
                &mut server,
                &Message::Meta {
                    stats,
                    settings: vec![],
                },
            )
            .await
            .unwrap();
            super::super::protocol::read_message(&mut server)
                .await
                .unwrap();
            super::super::protocol::send_message(&mut server, &Message::SyncDone)
                .await
                .unwrap();
        },
    );
    assert!(
        result.is_err(),
        "an incomplete transfer must not be reported as synced"
    );
    assert_eq!(
        sync_store::peer_recv_cursor(&a.lock().unwrap(), &peer.device_uuid).unwrap(),
        0
    );
}

#[test]
fn snapshot_cursor_does_not_skip_edits_to_already_sent_rows() {
    let a = test_db(&uuid("snapshot-a"));
    let b = test_db(&uuid("snapshot-b"));
    let entry = db::insert_dictionary_entry_returning(&a, "Before", None, None).unwrap();
    let mut progress = engine::SnapshotProgress::default();
    let (ops, _, done) =
        engine::collect_ops(&a.lock().unwrap(), 0, true, 1, &mut progress).unwrap();
    assert!(!done);
    engine::apply_ops(&b.lock().unwrap(), &ops).unwrap();
    db::update_dictionary_entry(&a, entry.id, "After", None).unwrap();
    let cursor = loop {
        let (ops, cursor, done) =
            engine::collect_ops(&a.lock().unwrap(), 0, true, 1, &mut progress).unwrap();
        engine::apply_ops(&b.lock().unwrap(), &ops).unwrap();
        if done {
            break cursor;
        }
    };
    let (ops, _, _) = engine::collect_ops(
        &a.lock().unwrap(),
        cursor,
        false,
        100,
        &mut engine::SnapshotProgress::default(),
    )
    .unwrap();
    engine::apply_ops(&b.lock().unwrap(), &ops).unwrap();
    let term: String = b
        .lock()
        .unwrap()
        .query_row("SELECT term FROM dictionary", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        term, "After",
        "the next delta must include edits made during a snapshot"
    );
}

#[test]
fn failed_batch_rolls_back_rows_and_change_capture_state() {
    let a = test_db(&uuid("atomic-a"));
    let b = test_db(&uuid("atomic-b"));
    db::insert_dictionary_entry_returning(&a, "Synthetic", None, None).unwrap();
    let (mut ops, _, _) = engine::collect_ops(
        &a.lock().unwrap(),
        0,
        false,
        100,
        &mut engine::SnapshotProgress::default(),
    )
    .unwrap();
    let mut invalid = ops[0].clone();
    invalid.row_uuid = uuid("invalid");
    invalid.payload = Some(json!({"term": 42}));
    ops.push(invalid);
    let conn = b.lock().unwrap();
    assert!(engine::apply_ops(&conn, &ops).is_err());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM dictionary"), 0);
    assert_eq!(count(&conn, "SELECT applying FROM sync_state"), 0);
}

#[test]
fn context_waits_for_snippet_in_a_later_batch() {
    let a = test_db(&uuid("dependency-a"));
    let b = test_db(&uuid("dependency-b"));
    let ctx = db::insert_context_returning(&a, "Synthetic Context", None, None, None, None, false)
        .unwrap();
    db::insert_snippet_returning(&a, "synthetic", "Fixture expansion", "", Some(ctx.id)).unwrap();
    let (ops, _, _) = engine::collect_ops(
        &a.lock().unwrap(),
        0,
        false,
        100,
        &mut engine::SnapshotProgress::default(),
    )
    .unwrap();
    let context_ops: Vec<_> = ops
        .iter()
        .filter(|op| op.table == "contexts")
        .cloned()
        .collect();
    let snippet_ops: Vec<_> = ops
        .iter()
        .filter(|op| op.table == "snippets")
        .cloned()
        .collect();
    assert!(!snippet_ops.is_empty());
    let conn = b.lock().unwrap();
    let summary = engine::apply_ops(&conn, &context_ops).unwrap();
    assert!(
        summary.deferred,
        "missing members must keep the receive cursor replayable"
    );
    engine::apply_ops(&conn, &snippet_ops).unwrap();
    engine::apply_ops(&conn, &context_ops).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM snippet_contexts sc JOIN contexts c ON c.id = sc.context_id WHERE c.name = 'Synthetic Context'"), 1);
}

#[test]
fn removing_all_context_app_targets_propagates() {
    let b = test_db(&uuid("target-delete-b"));
    let conn = b.lock().unwrap();
    let first = test_context_op(
        "Synthetic Apps",
        100,
        "fixture-app",
        db::current_platform_tag(),
    );
    engine::apply_ops(&conn, std::slice::from_ref(&first)).unwrap();
    let mut removed = first;
    removed.ts_ms = sync_store::now_ms() + 1;
    removed.payload.as_mut().unwrap()["targets"] = json!([]);
    engine::apply_ops(&conn, &[removed]).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM context_targets"), 0);
}
