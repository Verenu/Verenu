use super::*;

#[test]
fn agent_audit_records_pending_and_completed_without_change_contents() {
    let db = open(":memory:").unwrap();
    let id = begin_agent_change(&db, "edit_snippet", Some(17), "grant-one").unwrap();
    let pending = query_agent_changes(&db, 10).unwrap();
    assert_eq!(pending["entries"][0]["status"], "pending");
    assert_eq!(pending["atomic_with_change"], false);
    finish_agent_change(&db, id, true).unwrap();
    let done = query_agent_changes(&db, 10).unwrap();
    assert_eq!(done["entries"][0]["status"], "succeeded");
    assert_eq!(done["entries"][0]["target_id"], 17);
    assert_eq!(done["entries"][0]["actor_id"], "grant-one");
    assert!(done["entries"][0].get("value").is_none());
    assert!(query_agent_changes(&db, 0).is_err());
}

#[test]
fn library_usage_tracks_mechanical_matches_and_prunes_with_history() {
    let db = open(":memory:").unwrap();
    let term = insert_dictionary_entry_returning(&db, "Kubernetes", Some("kubernetez"), None).unwrap();
    let snippet = insert_snippet_returning(&db, "greeting", "Hello", "", None).unwrap();
    record_dictionary_usage(&db, &[term.id]).unwrap();
    increment_snippet_use_counts(&db, &[(snippet.id, 2)]).unwrap();
    let report = query_library_usage(&db).unwrap();
    assert_eq!(report["coverage"]["proves_unused"], false);
    let items = report["items"].as_array().unwrap();
    assert!(items.iter().any(|item| item["kind"] == "dictionary" && item["id"] == term.id && item["matches"] == 1));
    assert!(items.iter().any(|item| item["kind"] == "snippet" && item["id"] == snippet.id && item["matches"] == 2));
    {
        let conn = db.lock().unwrap();
        conn.execute("UPDATE library_usage_daily SET day = '2000-01-01'", []).unwrap();
    }
    prune_transcriptions_older_than(&db, 30).unwrap();
    assert!(query_library_usage(&db).unwrap()["items"].as_array().unwrap().is_empty());
}

#[test]
fn failed_dictionary_edits_do_not_change_other_contexts_or_assign_orphans() {
    let path = temp_db_path("agent_dictionary_edit_scope");
    let db = open(path.to_str().unwrap()).unwrap();
    let first = insert_context_returning(&db, "First", None, None, None, None, false).unwrap();
    let second = insert_context_returning(&db, "Second", None, None, None, None, false).unwrap();
    let shared = insert_dictionary_entry_returning(&db, "Shared", Some("wrongone"), Some(first.id)).unwrap();
    insert_dictionary_entry_returning(&db, "Shared", Some("wrongtwo"), Some(second.id)).unwrap();
    assert!(update_dictionary_entry_for_context(&db, first.id, shared.id, "Renamed", None).is_err());
    assert_eq!(query_dictionary(&db).unwrap().iter().find(|entry| entry.id == shared.id).unwrap().term, "Shared");
    assert!(rename_dictionary_entry_global(&db, shared.id, "Renamed", "Stale").is_err());
    rename_dictionary_entry_global(&db, shared.id, "Renamed", "Shared").unwrap();
    for (context, correction) in [(first.id, "wrongone"), (second.id, "wrongtwo")] {
        let entries = query_effective_dictionary_for_context(&db, context).unwrap();
        let entry = entries.iter().find(|entry| entry.id == shared.id).unwrap();
        assert_eq!(entry.term, "Renamed");
        assert_eq!(entry.mistake.as_deref(), Some(correction));
    }
    let orphan = insert_dictionary_entry_returning(&db, "Orphan", None, None).unwrap();
    remove_dictionary_entry_from_context(&db, EVERYWHERE_CONTEXT_ID, orphan.id).unwrap();
    assert!(update_dictionary_entry(&db, orphan.id, "", None).is_err());
    let conn = db.lock().unwrap();
    let assignments: i64 = conn.query_row("SELECT COUNT(*) FROM dictionary_contexts WHERE dictionary_id = ?1", [orphan.id], |row| row.get(0)).unwrap();
    assert_eq!(assignments, 0);
    drop(conn);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn global_rename_clears_old_learning_evidence_in_inheriting_contexts() {
    let db = test_db();
    let context = insert_context_returning(&db, "Inheriting", None, None, None, None, false).unwrap();
    let term = insert_dictionary_entry_returning(&db, "Original", Some("Origional"), None).unwrap();
    insert_dictionary_entry_returning(&db, "Existing", None, None).unwrap();
    {
        let conn = db.lock().unwrap();
        for context_id in [EVERYWHERE_CONTEXT_ID, context.id] {
            for table in ["pending_corrections", "auto_learn_candidates"] {
                conn.execute(&format!("INSERT INTO {table} (context_id, wrong_word, correct_word) VALUES (?1, 'mistake', 'Original')"), [context_id]).unwrap();
                conn.execute(&format!("INSERT INTO {table} (context_id, wrong_word, correct_word) VALUES (?1, 'unrelated', 'Other')"), [context_id]).unwrap();
            }
        }
    }
    assert!(rename_dictionary_entry_global(&db, term.id, "Existing", "Original").is_err());
    {
        let conn = db.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM pending_corrections WHERE correct_word = 'Original'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
    }
    rename_dictionary_entry_global(&db, term.id, "Renamed", "Original").unwrap();
    let conn = db.lock().unwrap();
    for table in ["pending_corrections", "auto_learn_candidates"] {
        let old: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE correct_word = 'Original'"), [], |r| r.get(0)).unwrap();
        let unrelated: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE correct_word = 'Other'"), [], |r| r.get(0)).unwrap();
        assert_eq!((old, unrelated), (0, 2));
    }
}

#[test]
fn agent_context_patch_preserves_omitted_fields_and_rejects_stale_input() {
    let path = temp_db_path("agent_context_patch");
    let db = open(path.to_str().unwrap()).unwrap();
    let context = insert_context_returning(&db, "Synthetic", Some("star"), Some("friendly"), None, Some("instructions"), false).unwrap();
    let expected = serde_json::to_value(&context).unwrap();
    let updated = patch_context_settings_extended(&db, context.id, expected.clone(), serde_json::json!({"tone": null, "paste_in_chunks": true, "color": "blue", "t3_skill_mentions_disabled": true})).unwrap();
    assert_eq!(updated.icon.as_deref(), Some("star"));
    assert_eq!(updated.custom_instructions.as_deref(), Some("instructions"));
    assert_eq!(updated.tone, None);
    assert!(updated.paste_in_chunks);
    assert_eq!(updated.color.as_deref(), Some("blue"));
    assert!(updated.t3_skill_mentions_disabled);
    assert!(patch_context_settings_extended(&db, context.id, expected, serde_json::json!({"icon": "other"})).is_err());
    assert_eq!(query_context(&db, context.id).unwrap().icon.as_deref(), Some("star"));
    insert_context_returning(&db, "Existing", None, None, None, None, false).unwrap();
    let fresh = serde_json::to_value(query_context(&db, context.id).unwrap()).unwrap();
    assert!(patch_context_settings_extended(&db, context.id, fresh, serde_json::json!({"name": "Existing", "tone": "formal", "color": "red"})).is_err());
    assert_eq!(query_context(&db, context.id).unwrap().tone, None);
    assert_eq!(query_context(&db, context.id).unwrap().color.as_deref(), Some("blue"));
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn agent_tables_migrate_from_master_version_36() {
    let path = temp_db_path("agent_from_v36");
    {
        let db = open(&path).unwrap();
        let conn = db.lock().unwrap();
        conn.execute_batch("DROP TABLE agent_library_changes; DROP TABLE agent_change_audit; DROP TABLE library_usage_daily; DROP TABLE library_usage_coverage; PRAGMA user_version = 36;").unwrap();
    }
    {
        let db = open(&path).unwrap();
        assert!(query_library_usage(&db).is_ok());
        let id = begin_agent_change(&db, "edit_snippet", None, "grant").unwrap();
        finish_agent_change(&db, id, true).unwrap();
        let conn = db.lock().unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
        assert_eq!(version, 39);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn agent_schema_upgrades_preserve_existing_usage_and_audit() {
    for version in 36..=38 {
        let path = temp_db_path(&format!("agent_from_v{version}"));
        {
            let db = open(&path).unwrap();
            let term = insert_dictionary_entry_returning(&db, "Synthetic", Some("Synthitik"), None).unwrap();
            record_dictionary_usage(&db, &[term.id]).unwrap();
            let id = begin_agent_change(&db, "edit_snippet", None, "grant").unwrap();
            finish_agent_change(&db, id, true).unwrap();
            insert_transcription_returning(&db, "synthetic raw", "synthetic clean", 0, 1000, "test", None, None).unwrap();
            let conn = db.lock().unwrap();
            conn.execute_batch("ALTER TABLE transcriptions DROP COLUMN vocabulary_replacements; ALTER TABLE transcriptions DROP COLUMN snippet_expansions;").unwrap();
            if version < 38 {
                conn.execute_batch("ALTER TABLE agent_change_audit DROP COLUMN actor_id;").unwrap();
            }
            if version < 37 {
                conn.execute_batch("DROP TABLE agent_library_changes;").unwrap();
            }
            conn.execute_batch(&format!("PRAGMA user_version = {version};")).unwrap();
        }
        {
            let db = open(&path).unwrap();
            assert_eq!(query_library_usage(&db).unwrap()["items"][0]["matches"], 1);
            assert_eq!(query_agent_changes(&db, 10).unwrap()["entries"][0]["status"], "succeeded");
            let conn = db.lock().unwrap();
            let row: (Option<i64>, Option<i64>) = conn.query_row("SELECT vocabulary_replacements, snippet_expansions FROM transcriptions", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            assert_eq!(row, (None, None));
            let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
            assert_eq!(current, 39);
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn cleanup_activity_migration_preserves_history_as_untracked() {
    let path = temp_db_path("cleanup_activity_v35");
    {
        let db = open(&path).unwrap();
        insert_transcription_returning(&db, "synthetic raw", "synthetic clean", 0, 1000, "test", None, None).unwrap();
        let conn = db.lock().unwrap();
        conn.execute_batch("ALTER TABLE transcriptions DROP COLUMN vocabulary_replacements; ALTER TABLE transcriptions DROP COLUMN snippet_expansions; PRAGMA user_version = 35;").unwrap();
    }
    {
        let db = open(&path).unwrap();
        let conn = db.lock().unwrap();
        let row: (String, String, Option<i64>, Option<i64>) = conn.query_row("SELECT raw_text, clean_text, vocabulary_replacements, snippet_expansions FROM transcriptions", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!(row, ("synthetic raw".into(), "synthetic clean".into(), None, None));
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn t3_skill_opt_out_migrates_defaults_and_survives_reopen_and_duplicate() {
    let path = temp_db_path("t3_skills_v31");
    let context_id;
    {
        let db = open(path.to_str().unwrap()).unwrap();
        context_id = insert_context_returning(&db, "Synthetic T3", None, None, None, None, false)
            .unwrap()
            .id;
        assign_context_target(&db, context_id, "t3code").unwrap();
        let conn = db.lock().unwrap();
        conn.execute_batch("ALTER TABLE contexts DROP COLUMN t3_skill_mentions_disabled; PRAGMA user_version = 31;").unwrap();
    }
    {
        let db = open(path.to_str().unwrap()).unwrap();
        assert!(
            !query_context(&db, context_id)
                .unwrap()
                .t3_skill_mentions_disabled
        );
        assert_eq!(
            query_context_targets(&db, Some(context_id)).unwrap().len(),
            1
        );
        update_context_t3_skill_mentions(&db, context_id, false).unwrap();
        assert!(
            duplicate_context(&db, context_id)
                .unwrap()
                .t3_skill_mentions_disabled
        );
    }
    {
        let db = open(path.to_str().unwrap()).unwrap();
        assert!(
            query_context(&db, context_id)
                .unwrap()
                .t3_skill_mentions_disabled
        );
        update_context_t3_skill_mentions(&db, context_id, true).unwrap();
        assert!(
            !query_context(&db, context_id)
                .unwrap()
                .t3_skill_mentions_disabled
        );
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn paste_chunks_migrates_v30_defaults_and_survives_reopen() {
    let path = temp_db_path("paste_chunks_v30");
    {
        let db = open(path.to_str().unwrap()).unwrap();
        let conn = db.lock().unwrap();
        conn.execute_batch("ALTER TABLE contexts DROP COLUMN paste_in_chunks; PRAGMA user_version = 30;").unwrap();
    }
    {
        let db = open(path.to_str().unwrap()).unwrap();
        assert!(!query_context(&db, EVERYWHERE_CONTEXT_ID).unwrap().paste_in_chunks);
        update_context_paste_in_chunks(&db, EVERYWHERE_CONTEXT_ID, true).unwrap();
    }
    {
        let db = open(path.to_str().unwrap()).unwrap();
        assert!(query_context(&db, EVERYWHERE_CONTEXT_ID).unwrap().paste_in_chunks);
    }
    std::fs::remove_file(path).unwrap();
}

fn test_db() -> Db {
    open(":memory:").expect("test db")
}

#[test]
fn stale_snippet_editor_cannot_overwrite_new_content() {
    let db = test_db();
    let entry = insert_snippet_returning(&db, "shortcut", "Original", "", None).unwrap();
    update_snippet(&db, entry.id, "shortcut", "Agent change", "").unwrap();
    let stale = ExpectedSnippet { trigger: "shortcut".into(), expansion: "Original".into(), instructions: "".into() };
    assert!(update_snippet_if_unchanged(&db, entry.id, "shortcut", "UI change", "", Some(&stale)).is_err());
    assert_eq!(query_snippets(&db).unwrap()[0].expansion, "Agent change");
}

#[test]
fn library_operations_can_be_rolled_back_together() {
    let db = test_db();
    let original = insert_dictionary_entry_returning(&db, "Original", Some("Orignal"), None)
        .unwrap();
    let snippet = insert_snippet_returning(&db, "shortcut", "Original text", "", None).unwrap();
    {
        let mut conn = db.lock().unwrap();
        let tx = conn.transaction().unwrap();
        delete_dictionary_entry_conn(&tx, original.id).unwrap();
        insert_dictionary_entry_returning_conn(&tx, "New term", None, None).unwrap();
        update_snippet_conn(&tx, snippet.id, "changed", "Changed text", "").unwrap();
        tx.rollback().unwrap();
    }
    let dictionary = query_dictionary(&db).unwrap();
    assert!(dictionary.iter().any(|entry| entry.id == original.id));
    assert!(!dictionary.iter().any(|entry| entry.term == "New term"));
    let snippets = query_snippets(&db).unwrap();
    assert!(snippets.iter().any(|entry| entry.id == snippet.id && entry.trigger == "shortcut"));
}

#[test]
fn agent_bulk_delete_previews_retries_and_restores_atomically() {
    let db = test_db();
    let term = insert_dictionary_entry_returning(&db, "Example", Some("Exampel"), None).unwrap();
    let snippet = insert_snippet_returning(&db, "shortcut", "Some text", "", None).unwrap();
    let listed = list_library_items(&db).unwrap();
    let items: Vec<LibraryItem> = listed["items"].as_array().unwrap().iter().map(|v| LibraryItem {
        kind: v["kind"].as_str().unwrap().into(), uuid: v["uuid"].as_str().unwrap().into(),
    }).collect();
    let preview = preview_library_delete(&db, items.clone()).unwrap();
    let basis = preview["basis"].as_str().unwrap().to_owned();
    record_dictionary_usage(&db, &[term.id]).unwrap();
    assert!(apply_library_delete(&db, items.clone(), &basis, "00000000-0000-4000-8000-000000000001").is_err());
    let preview = preview_library_delete(&db, items.clone()).unwrap();
    assert_eq!(preview["items"].as_array().unwrap().iter().find(|item| item["kind"] == "dictionary").unwrap()["local_matches"], 1);
    let basis = preview["basis"].as_str().unwrap().to_owned();
    update_snippet(&db, snippet.id, "shortcut", "Changed", "").unwrap();
    assert!(apply_library_delete(&db, items.clone(), &basis, "00000000-0000-4000-8000-000000000001").is_err());
    assert_eq!(query_dictionary(&db).unwrap().len(), 1);
    let preview = preview_library_delete(&db, items.clone()).unwrap();
    let basis = preview["basis"].as_str().unwrap();
    let request = "00000000-0000-4000-8000-000000000001";
    assert_eq!(apply_library_delete(&db, items.clone(), basis, request).unwrap()["deleted"], 2);
    assert!(query_dictionary(&db).unwrap().is_empty());
    assert!(query_snippets(&db).unwrap().is_empty());
    assert_eq!(apply_library_delete(&db, items, basis, request).unwrap()["replayed"], true);
    insert_snippet_returning(&db, "shortcut", "Replacement", "", None).unwrap();
    assert!(undo_library_delete(&db, request).is_err());
    assert!(query_dictionary(&db).unwrap().is_empty());
    delete_snippet(&db, query_snippets(&db).unwrap()[0].id).unwrap();
    assert_eq!(undo_library_delete(&db, request).unwrap()["restored"], 2);
    let restored = query_dictionary(&db).unwrap();
    assert_eq!(restored[0].term, "Example");
    assert_eq!(restored[0].mistake.as_deref(), Some("Exampel"));
    assert_eq!(query_snippets(&db).unwrap()[0].expansion, "Changed");
    assert_eq!(undo_library_delete(&db, request).unwrap()["replayed"], true);
    assert_ne!(restored[0].id, term.id);
}

#[test]
fn retention_prune_does_not_create_transcription_delete_tombstones() {
    let db = test_db();
    insert_transcription_returning(&db, "old", "old", 1, 1_000, "test", None, None)
        .expect("transcription");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET created_at = datetime('now', '-30 days')",
            [],
        )
        .expect("backdate transcription");
    }

    assert_eq!(prune_transcriptions_older_than(&db, 7).expect("prune"), 1);
    let conn = lock_conn(&db).expect("lock");
    let delete_logs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_log WHERE table_name = 'transcriptions' AND op = 'delete'",
            [],
            |r| r.get(0),
        )
        .expect("delete log count");
    assert_eq!(delete_logs, 0);
}

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("verenu_{name}_{}_{}.db", std::process::id(), nanos))
}

#[test]
fn open_repairs_v20_sync_peers_missing_recv_cursor() {
    let path = temp_db_path("v20_missing_recv_cursor");
    {
        let db = open(path.to_str().expect("path string")).expect("create current db");
        let conn = lock_conn(&db).expect("lock current db");
        conn.execute_batch(
            "ALTER TABLE sync_peers RENAME TO sync_peers_current;
             CREATE TABLE sync_peers (
               device_uuid    TEXT PRIMARY KEY,
               name           TEXT NOT NULL DEFAULT '',
               cert_fp        TEXT NOT NULL,
               added_at       TEXT NOT NULL DEFAULT (datetime('now')),
               last_sync_at   TEXT,
               send_cursor    INTEGER NOT NULL DEFAULT 0,
               needs_snapshot INTEGER NOT NULL DEFAULT 1,
               last_error     TEXT
             );
             DROP TABLE sync_peers_current;
             PRAGMA user_version = 20;",
        )
        .expect("seed partial v20 sync schema");
    }

    let db = open(path.to_str().expect("path string")).expect("open repairs partial v20 db");
    let conn = lock_conn(&db).expect("lock repaired db");
    assert!(table_has_column(&conn, "sync_peers", "recv_cursor").expect("column check"));
    conn.execute(
        "INSERT INTO sync_peers (device_uuid, name, cert_fp, recv_cursor)
         VALUES ('peer', 'Peer', 'fingerprint', 7)",
        [],
    )
    .expect("use repaired receive cursor");
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn sqlite_disk_maintenance_reclaims_incremental_free_pages() {
    let path = temp_db_path("vacuum_incremental");
    let db = open(path.to_str().expect("path string")).expect("open db");
    {
        let conn = lock_conn(&db).expect("lock db");
        conn.execute_batch("CREATE TABLE reclaim_probe (payload BLOB NOT NULL);")
            .expect("probe table");
        let payload = vec![b'x'; 4096];
        for _ in 0..1_024 {
            conn.execute(
                "INSERT INTO reclaim_probe (payload) VALUES (?1)",
                rusqlite::params![payload],
            )
            .expect("probe row");
        }
        conn.execute("DELETE FROM reclaim_probe", [])
            .expect("delete probe rows");
    }

    sqlite_disk_maintenance(&db).expect("reclaim database");
    let conn = lock_conn(&db).expect("lock reclaimed db");
    let auto_vacuum: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .expect("auto vacuum mode");
    let freelist: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .expect("freelist");
    assert_eq!(auto_vacuum, 2);
    assert!(freelist >= 0);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn failed_fts_repair_leaves_transcription_writes_safe() {
    let db = test_db();
    let conn = lock_conn(&db).expect("lock");
    conn.execute_batch(
        "DROP TRIGGER trg_transcriptions_fts_ins;
         DROP TRIGGER trg_transcriptions_fts_del;
         DROP TRIGGER trg_transcriptions_fts_upd;
         DROP TABLE transcriptions_fts;
         CREATE TABLE transcriptions_fts (rowid INTEGER PRIMARY KEY);
         UPDATE transcription_fts_meta SET populated = 1 WHERE name = 'history';",
    )
    .expect("seed interrupted FTS setup");

    super::schema::ensure_history_fts(&conn);
    assert_eq!(
        conn.query_row(
            "SELECT populated FROM transcription_fts_meta WHERE name = 'history'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("FTS state"),
        0
    );
    conn.execute(
        "INSERT INTO transcriptions (raw_text, clean_text, words, duration_ms, api_used)
         VALUES ('raw', 'clean', 1, 1000, 'test')",
        [],
    )
    .expect("insert without an incomplete FTS trigger");
    conn.execute("UPDATE transcriptions SET clean_text = 'updated'", [])
        .expect("update without an incomplete FTS trigger");
    conn.execute("DELETE FROM transcriptions", [])
        .expect("delete without an incomplete FTS trigger");
}

#[test]
fn fts_repair_rebuilds_updates_made_while_triggers_were_missing() {
    let db = test_db();
    let conn = lock_conn(&db).expect("lock");
    conn.execute_batch(
        "INSERT INTO transcriptions (raw_text, clean_text, words)
         VALUES ('synthetic', 'beforemarker', 1);
         DROP TRIGGER trg_transcriptions_fts_upd;
         UPDATE transcriptions SET clean_text = 'aftermarker';",
    )
    .expect("simulate an interrupted index installation");
    assert!(!super::transcriptions::history_fts_available(&conn));

    super::schema::ensure_history_fts(&conn);
    assert!(super::transcriptions::history_fts_available(&conn));
    for (term, expected) in [("beforemarker", 0), ("aftermarker", 1)] {
        let matches: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcriptions_fts WHERE transcriptions_fts MATCH ?1",
                [term],
                |row| row.get(0),
            )
            .expect("indexed search without LIKE fallback");
        assert_eq!(matches, expected);
    }
    super::schema::ensure_history_fts(&conn);
    conn.execute(
        "INSERT INTO transcriptions_fts(transcriptions_fts, rank) VALUES ('integrity-check', 1)",
        [],
    )
    .expect("index agrees with external content");
}

#[test]
fn open_repairs_legacy_cleanup_cache_missing_epoch_columns() {
    let path = temp_db_path("legacy_cleanup_cache");
    {
        let conn = Connection::open(&path).expect("create legacy db");
        conn.execute_batch(
            "CREATE TABLE cleanup_cache (
               key         TEXT PRIMARY KEY,
               clean_text  TEXT NOT NULL,
               hit_count   INTEGER NOT NULL DEFAULT 0,
               created_at  DATETIME NOT NULL DEFAULT (datetime('now')),
               last_hit_at DATETIME NOT NULL DEFAULT (datetime('now')),
               expires_at  DATETIME NOT NULL,
               is_snippet  INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO cleanup_cache
               (key, clean_text, hit_count, created_at, last_hit_at, expires_at, is_snippet)
             VALUES
               ('legacy', 'hello', 1, '2026-01-01 00:00:00', '2026-01-01 00:00:00', '2999-01-01 00:00:00', 0);
             PRAGMA user_version = 6;",
        )
        .expect("seed legacy db");
    }

    let db = open(path.to_str().expect("path string")).expect("open repairs legacy db");
    assert!(cleanup_cache_get_active(&db, "legacy")
        .expect("query repaired row")
        .is_none());

    let conn = lock_conn(&db).expect("lock");
    assert!(table_has_column(&conn, "cleanup_cache", "expires_at_epoch").expect("column"));
    assert!(table_has_column(&conn, "cleanup_cache", "last_hit_at_epoch").expect("column"));
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn open_backfills_legacy_spoken_words_column() {
    let path = temp_db_path("legacy_spoken_words");
    {
        let conn = Connection::open(&path).expect("create legacy db");
        conn.execute_batch(
            "CREATE TABLE transcriptions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               raw_text TEXT NOT NULL,
               clean_text TEXT NOT NULL,
               words INTEGER NOT NULL DEFAULT 0,
               duration_ms INTEGER NOT NULL DEFAULT 0,
               api_used TEXT NOT NULL DEFAULT '',
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE snippets (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               trigger TEXT NOT NULL UNIQUE,
               expansion TEXT NOT NULL,
               instructions TEXT NOT NULL DEFAULT '',
               use_count INTEGER NOT NULL DEFAULT 0,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO snippets (trigger, expansion) VALUES ('sig', 'signature');
             WITH RECURSIVE rows(n) AS (
               SELECT 1 UNION ALL SELECT n + 1 FROM rows WHERE n < 600
             )
             INSERT INTO transcriptions (raw_text, clean_text, words, duration_ms, api_used)
               SELECT 'hello sig world', 'clean', 3, 1000, 'test' FROM rows;
             PRAGMA user_version = 6;",
        )
        .expect("seed legacy db");
    }

    let db = open(path.to_str().expect("path string")).expect("open repairs legacy db");
    let conn = lock_conn(&db).expect("lock");
    let spoken_words: i64 = conn
        .query_row("SELECT spoken_words FROM transcriptions LIMIT 1", [], |r| {
            r.get(0)
        })
        .expect("spoken words");
    assert_eq!(spoken_words, 2);
    let backfilled: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcriptions WHERE spoken_words = 2",
            [],
            |r| r.get(0),
        )
        .expect("all legacy rows backfilled in batches");
    assert_eq!(backfilled, 600);
    assert!(table_exists(&conn, "transcription_daily_stats").expect("daily summary table"));
    assert!(table_exists(&conn, "transcriptions_fts").expect("history FTS table"));
    assert!(!table_exists(&conn, "transcriptions_vocab").expect("legacy vocabulary FTS removed"));
    let wpm_count: i64 = conn
        .query_row(
            "SELECT wpm_count FROM lifetime_stats WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("wpm summary");
    assert_eq!(wpm_count, 600);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn open_repairs_db_stuck_at_v7_without_spoken_words_column() {
    let path = temp_db_path("v7_missing_spoken_words");
    {
        let conn = Connection::open(&path).expect("create stuck db");
        conn.execute_batch(
            "CREATE TABLE transcriptions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               raw_text TEXT NOT NULL,
               clean_text TEXT NOT NULL,
               words INTEGER NOT NULL DEFAULT 0,
               duration_ms INTEGER NOT NULL DEFAULT 0,
               api_used TEXT NOT NULL DEFAULT '',
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE snippets (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               trigger TEXT NOT NULL UNIQUE,
               expansion TEXT NOT NULL,
               instructions TEXT NOT NULL DEFAULT '',
               use_count INTEGER NOT NULL DEFAULT 0,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO snippets (trigger, expansion) VALUES ('sig', 'signature');
             INSERT INTO transcriptions (raw_text, clean_text, words, duration_ms, api_used)
               VALUES ('hello sig world', 'clean', 3, 1000, 'test');
             PRAGMA user_version = 7;",
        )
        .expect("seed stuck db");
    }

    let db = open(path.to_str().expect("path string")).expect("open repairs stuck db");

    insert_transcription_returning(
        &db,
        "second clip",
        "second clip",
        2,
        1000,
        "test",
        None,
        None,
    )
    .expect("insert after repair");

    let conn = lock_conn(&db).expect("lock");
    let spoken_words: i64 = conn
        .query_row(
            "SELECT spoken_words FROM transcriptions WHERE raw_text = 'hello sig world'",
            [],
            |r| r.get(0),
        )
        .expect("spoken words backfilled");
    assert_eq!(spoken_words, 2);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn open_alone_does_not_seed_the_dictionary() {
    let db = test_db();
    let entries = query_dictionary(&db).expect("query dictionary");
    assert!(entries.is_empty());
}

#[test]
fn reopen_existing_database_preserves_peer_counters_with_backup_view() {
    let path = temp_db_path("backup_counter_view");
    {
        let db = open(&path).expect("create database");
        let conn = lock_conn(&db).expect("lock");
        conn.execute("INSERT INTO sync_remote_stats(device_id, total_words, dictionary_fixes) VALUES ('peer', 42, 3)", []).unwrap();
        conn.execute_batch("DROP VIEW combined_device_counters; DROP TABLE backup_device_counters;").unwrap();
    }
    {
        let db = open(&path).expect("reopen existing database");
        let conn = lock_conn(&db).expect("lock");
        let totals: (i64, i64) = conn.query_row("SELECT total_words, dictionary_fixes FROM combined_device_counters WHERE device_id = 'peer'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(totals, (42, 3));
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn migration_marks_only_ambiguous_preupgrade_history() {
    let path = temp_db_path("preupgrade_activity_identity");
    {
        let db = open(&path).unwrap();
        insert_transcription_returning(&db, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
        let conn = lock_conn(&db).unwrap();
        conn.execute_batch("DELETE FROM backup_preupgrade_history; PRAGMA user_version=33;").unwrap();
    }
    {
        let db = open(&path).unwrap();
        let conn = lock_conn(&db).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_preupgrade_history", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        conn.execute("UPDATE transcription_daily_stats SET total_words=total_words+2, total_transcriptions=total_transcriptions+1", []).unwrap();
        conn.execute_batch("PRAGMA user_version=33;").unwrap();
    }
    {
        let db = open(&path).unwrap();
        let conn = lock_conn(&db).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_preupgrade_history", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn opening_a_newer_database_is_rejected_before_schema_changes() {
    let path = temp_db_path("future_database");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE future_data(value TEXT); INSERT INTO future_data VALUES ('preserved'); PRAGMA user_version = 999;").unwrap();
    }
    assert!(open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(conn.query_row("SELECT value FROM future_data", [], |r| r.get::<_, String>(0)).unwrap(), "preserved");
    drop(conn);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn retention_keeps_statistical_receipts_and_newer_calls_without_text() {
    let db = test_db();
    let entry = insert_transcription_returning(&db, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
    {
        let conn = lock_conn(&db).unwrap();
        conn.execute("UPDATE transcriptions SET created_at = '2020-01-01 12:00:00' WHERE id = ?1", [entry.id]).unwrap();
        conn.execute("INSERT INTO api_calls(transcription_id, model, provider, task, created_at) VALUES (?1, 'synthetic', 'test', 'stt', datetime('now'))", [entry.id]).unwrap();
    }
    assert_eq!(prune_transcriptions_older_than(&db, 7).unwrap(), 1);
    let conn = lock_conn(&db).unwrap();
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn open_self_heals_database_stuck_at_v2_with_legacy_dictionary() {
    let path = temp_db_path("v2_legacy_dictionary");
    {
        let conn = Connection::open(&path).expect("create stuck db");
        conn.execute_batch(
            "CREATE TABLE transcriptions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               raw_text TEXT NOT NULL,
               clean_text TEXT NOT NULL,
               words INTEGER NOT NULL DEFAULT 0,
               duration_ms INTEGER NOT NULL DEFAULT 0,
               api_used TEXT NOT NULL DEFAULT '',
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE dictionary (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               wrong TEXT NOT NULL,
               correct TEXT NOT NULL,
               auto_learned INTEGER NOT NULL DEFAULT 0,
               correction_count INTEGER NOT NULL DEFAULT 0,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE TABLE snippets (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               trigger TEXT NOT NULL UNIQUE,
               expansion TEXT NOT NULL,
               use_count INTEGER NOT NULL DEFAULT 0,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO dictionary (wrong, correct, auto_learned)
               VALUES ('Varinu', 'Verenu', 1);
             INSERT INTO transcriptions (raw_text, clean_text, words, duration_ms, api_used)
               VALUES ('hello sig world', 'clean', 3, 1000, 'test');
             INSERT INTO snippets (trigger, expansion) VALUES ('sig', 'signature');
             PRAGMA user_version = 2;",
        )
        .expect("seed stuck db");
    }

    let db = open(path.to_str().expect("path string")).expect("open self-heals stuck db");

    let entries = query_dictionary(&db).expect("query dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].term, "Verenu");
    assert_eq!(entries[0].mistake.as_deref(), Some("Varinu"));
    insert_dictionary_entry(&db, "Tauri", Some("Tari")).expect("insert modern entry");
    assert!(
        table_has_column(&lock_conn(&db).expect("lock"), "snippets", "instructions")
            .expect("instructions column"),
        "snippets.instructions must be self-healed"
    );
    assert!(
        table_exists(&lock_conn(&db).expect("lock"), "pending_corrections").expect("table"),
        "pending_corrections must be self-healed"
    );
    let conn = lock_conn(&db).expect("lock");
    let counter_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM combined_device_counters", [], |r| r.get(0))
        .expect("counter view is available after legacy migrations");
    assert_eq!(counter_rows, 0);
    let spoken_words: i64 = conn
        .query_row(
            "SELECT spoken_words FROM transcriptions WHERE raw_text = 'hello sig world'",
            [],
            |r| r.get(0),
        )
        .expect("spoken words backfilled");
    assert_eq!(spoken_words, 2);
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("version");
    assert_eq!(version, 39);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn open_with_recovery_does_not_quarantine_non_corruption_errors() {
    let parent = std::env::temp_dir().join(format!(
        "verenu_noncorrupt_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&parent).expect("create parent");
    let path = parent.join("verenu.db");
    std::fs::create_dir(&path).expect("create dir-as-db");

    let err = open_with_recovery(&path).expect_err("a directory is not a database");
    assert!(
        err.to_string().to_lowercase().contains("unable to open"),
        "unexpected error: {err}"
    );

    let quarantined = std::fs::read_dir(&parent)
        .expect("list dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".corrupt-"))
        .collect::<Vec<_>>();
    assert!(
        quarantined.is_empty(),
        "no quarantine files may be created for a non-corruption error: {quarantined:?}"
    );

    let _ = std::fs::remove_dir(&path);
    let _ = std::fs::remove_dir(&parent);
}

#[test]
fn open_with_recovery_quarantines_a_corrupt_database_and_opens_fresh() {
    let path = temp_db_path("corrupt_db_recovery");
    std::fs::write(&path, b"this is not a sqlite database at all").expect("write garbage");

    assert!(open(&path).is_err());

    let db = open_with_recovery(&path).expect("recovery opens a fresh database");
    insert_transcription_returning(&db, "fresh", "fresh", 1, 1000, "test", None, None)
        .expect("fresh db is writable");

    let quarantine = std::fs::read_dir(path.parent().expect("parent"))
        .expect("list dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            let stem = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.starts_with(stem) && name.contains(".db.corrupt-")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        quarantine.len(),
        1,
        "exactly one quarantine file for this database: {quarantine:?}"
    );

    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _ = std::fs::remove_file(quarantine[0].clone());
}

#[test]
fn open_with_recovery_leaves_a_healthy_database_untouched() {
    let path = temp_db_path("healthy_db_recovery");
    {
        let db = open(&path).expect("healthy open");
        insert_transcription_returning(&db, "keep me", "keep me", 2, 1000, "test", None, None)
            .expect("insert");
    }
    let db = open_with_recovery(&path).expect("recovery keeps healthy db");
    let entries = query_recent_page(&db, 10, 0, None, None).expect("query");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].clean_text, "keep me");
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn open_with_recovery_quarantines_db_and_sidecars_together() {
    let path = temp_db_path("sidecar_recovery");
    {
        let db = open(&path).expect("create db");
        insert_transcription_returning(&db, "first", "first", 1, 1000, "test", None, None)
            .expect("insert");
        drop(db);
    }
    let wal_path = path.with_extension("db-wal");
    let shm_path = path.with_extension("db-shm");
    std::fs::write(&wal_path, b"stale wal").expect("create stale wal sidecar");
    std::fs::write(&shm_path, b"stale shm").expect("create stale shm sidecar");

    assert!(
        super::quarantine_corrupt_db_files(&path),
        "all three files must be moved aside"
    );

    assert!(!path.exists(), "main file moved aside");
    assert!(!wal_path.exists(), "wal sidecar moved aside");
    assert!(!shm_path.exists(), "shm sidecar moved aside");

    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let quarantined: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
        .expect("dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(stem) && name.contains(".corrupt-"))
        .collect();
    assert_eq!(
        quarantined.len(),
        3,
        "main + wal + shm all quarantined: {quarantined:?}"
    );

    for name in quarantined {
        let _ = std::fs::remove_file(path.parent().expect("parent").join(name));
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(wal_path);
    let _ = std::fs::remove_file(shm_path);
}

#[test]
fn seed_default_dictionary_entries_adds_a_verenu_entry_with_every_known_variant() {
    let db = test_db();
    seed_default_dictionary_entries(&db).expect("seed");
    let entries = query_dictionary(&db).expect("query dictionary");
    let verenu = entries
        .iter()
        .find(|e| e.term == "Verenu")
        .expect("default Verenu entry seeded");
    assert!(
        !verenu.auto_learned,
        "seeded entry must be manual, not gated by distinctiveness"
    );
    let mistake = verenu.mistake.as_deref().unwrap_or_default();
    for variant in [
        "Varinu", "Verena", "Virinu", "Varino", "Varinew", "Varina", "Verminu", "Varinian",
        "Marino", "Zarinu", "Berenu", "Ferenu", "Werenu", "Verinu", "Varineu", "Verino",
    ] {
        assert!(mistake.contains(variant), "missing variant: {variant}");
    }
}

#[test]
fn seed_default_dictionary_entries_upgrades_verino_in_an_already_seeded_dictionary() {
    let db = test_db();
    insert_dictionary_entry(&db, "Verenu", Some("Verinu")).expect("old default");
    db.lock().unwrap().execute(
        "INSERT INTO seeded_defaults (key) VALUES ('verenu_dictionary_v1')", [],
    ).unwrap();
    seed_default_dictionary_entries(&db).expect("upgrade");
    seed_default_dictionary_entries(&db).expect("idempotent upgrade");
    let entries = query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID).unwrap();
    let verenu = entries.iter().find(|entry| entry.term == "Verenu").unwrap();
    assert_eq!(verenu.corrections.iter().filter(|entry| entry.mistake == "Verino").count(), 1);
    let prompt = crate::data::dictionary::build_relevant_dictionary_prompt_from(
        &entries, "Please open Verino and start dictating this message.",
    );
    assert!(prompt.contains("preferred: \"Verenu\""));
    assert!(prompt.contains("\"Verino\""));
}

#[test]
fn seed_default_dictionary_entries_preserves_a_custom_verino_mapping() {
    let db = test_db();
    insert_dictionary_entry(&db, "Custom name", Some("Verino")).unwrap();
    seed_default_dictionary_entries(&db).unwrap();
    let entries = query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID).unwrap();
    let mappings: Vec<_> = entries.iter().filter(|entry|
        entry.corrections.iter().any(|correction| correction.mistake == "Verino")
    ).collect();
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].term, "Custom name");
}

#[test]
fn seed_default_dictionary_entries_does_not_resurrect_a_deleted_entry() {
    let path = temp_db_path("verenu_seed_deletion");
    {
        let db = open(path.to_str().expect("path string")).expect("first open");
        seed_default_dictionary_entries(&db).expect("first seed");
        let entries = query_dictionary(&db).expect("query dictionary");
        let verenu_id = entries
            .iter()
            .find(|e| e.term == "Verenu")
            .expect("seeded on first call")
            .id;
        delete_dictionary_entry(&db, verenu_id).expect("user deletes the default entry");
    }
    let db = open(path.to_str().expect("path string")).expect("second open");
    seed_default_dictionary_entries(&db).expect("second seed is a no-op");
    let entries = query_dictionary(&db).expect("query dictionary after reopen");
    assert!(!entries.iter().any(|e| e.term == "Verenu"));
    drop(db);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn seed_default_dictionary_entries_merges_into_a_preexisting_manual_verenu_entry() {
    let db = test_db();
    insert_dictionary_entry(&db, "Verenu", Some("Vernu")).expect("user's own manual entry");

    seed_default_dictionary_entries(&db).expect("seed merges into existing entry");

    let entries = query_dictionary(&db).expect("query dictionary");
    assert_eq!(
        entries.iter().filter(|e| e.term == "Verenu").count(),
        1,
        "must not create a duplicate row"
    );
    let verenu = entries.iter().find(|e| e.term == "Verenu").expect("entry");
    let mistake = verenu.mistake.as_deref().unwrap_or_default();
    assert!(mistake.contains("Vernu"), "user's own variant must survive");
    for variant in [
        "Varinu", "Verena", "Virinu", "Varino", "Varinew", "Varina", "Verminu", "Varinian",
        "Marino", "Zarinu", "Berenu", "Ferenu", "Werenu", "Verinu", "Varineu", "Verino",
    ] {
        assert!(mistake.contains(variant), "missing variant: {variant}");
    }
}

#[test]
fn seed_default_dictionary_entries_does_not_duplicate_variants_already_present() {
    let db = test_db();
    insert_dictionary_entry(&db, "Verenu", Some("Varinu, Verena")).expect("partial entry");

    seed_default_dictionary_entries(&db).expect("seed");
    seed_default_dictionary_entries(&db).expect("seed again is a no-op");

    let entries = query_dictionary(&db).expect("query dictionary");
    let mistake = entries
        .iter()
        .find(|e| e.term == "Verenu")
        .and_then(|e| e.mistake.clone())
        .unwrap_or_default();
    assert_eq!(
        mistake.matches("Varinu").count(),
        1,
        "an already-present variant must not be duplicated"
    );
}

#[test]
fn auto_learn_does_not_overwrite_manual_dictionary_entry() {
    let db = test_db();

    insert_dictionary_entry(&db, "Kubernetes", Some("manual typo")).expect("manual insert");
    let promoted =
        insert_dictionary_entry_auto_learned(&db, "Kubernetes", Some("manual typo"), "high")
            .expect("auto insert");

    assert!(!promoted);
    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].term, "Kubernetes");
    assert_eq!(entries[0].mistake.as_deref(), Some("manual typo"));
    assert!(!entries[0].auto_learned);
    assert_eq!(entries[0].correction_count, 0);
}

#[test]
fn auto_learn_updates_and_keeps_context_owned_pairs_independent() {
    let db = test_db();

    assert!(
        insert_dictionary_entry_auto_learned(&db, "Kubernetes", Some("Koobernetes"), "high")
            .expect("first insert")
    );
    assert!(
        insert_dictionary_entry_auto_learned(&db, "Kubernetes", Some("Koobernetes"), "high")
            .expect("same pair")
    );
    assert!(
        insert_dictionary_entry_auto_learned(&db, "Kubernetes", Some("Kubernetties"), "low",)
            .expect("different pair")
    );

    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].mistake.as_deref(),
        Some("Koobernetes, Kubernetties")
    );
    assert_eq!(entries[0].correction_count, 3);
    assert_eq!(entries[0].corrections.len(), 2);
}

#[test]
fn auto_learn_promote_promotes_when_pending_reaches_threshold() {
    let db = test_db();

    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    let first = auto_learn_promote_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        "medium",
        2,
        2,
    )
    .expect("first promote call");
    assert_eq!(
        first,
        AutoLearnPromoteResult::BelowThreshold { pending_count: 1 }
    );
    assert!(query_dictionary(&db).expect("dictionary").is_empty());

    let second = auto_learn_promote_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        "medium",
        2,
        2,
    )
    .expect("second promote call");
    assert_eq!(second, AutoLearnPromoteResult::Promoted);

    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].term, "Kubernetes");
    assert_eq!(entries[0].mistake.as_deref(), Some("Koobernetes"));
    assert!(entries[0].auto_learned);
}

#[test]
fn auto_learn_promote_is_atomic_against_double_promotion() {
    let db = test_db();
    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("prior pending 1");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("prior pending 2");

    let first = auto_learn_promote_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        "medium",
        2,
        2,
    )
    .expect("first");
    assert_eq!(first, AutoLearnPromoteResult::Promoted);

    let second = auto_learn_promote_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        "medium",
        2,
        2,
    )
    .expect("second");
    assert_eq!(
        second,
        AutoLearnPromoteResult::AlreadyPromoted,
        "second concurrent promotion must be refused"
    );

    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].correction_count, 1,
        "correction_count must not be inflated by the concurrent duplicate"
    );
}

#[test]
fn auto_learn_promote_does_not_resurrect_a_rejected_candidate() {
    let db = test_db();
    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("prior pending");
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("promote"),
        AutoLearnPromoteResult::Promoted
    );
    let id = query_dictionary(&db)
        .expect("dictionary")
        .into_iter()
        .next()
        .expect("entry")
        .id;

    delete_auto_learned_entries_by_ids(&db, &[id]).expect("reject");
    assert!(query_dictionary(&db).expect("dictionary after").is_empty());

    let stale = auto_learn_promote_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        "medium",
        2,
        2,
    )
    .expect("stale");
    assert_ne!(
        stale,
        AutoLearnPromoteResult::Promoted,
        "rejected pair must not be resurrected by an in-flight monitor"
    );
    assert!(
        query_dictionary(&db)
            .expect("dictionary after stale")
            .is_empty(),
        "rejected pair must not be resurrected"
    );
}

#[test]
fn auto_learn_promote_can_relearn_after_rejection() {
    let db = test_db();
    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("prior pending");
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("promote"),
        AutoLearnPromoteResult::Promoted
    );
    let id = query_dictionary(&db)
        .expect("dictionary")
        .into_iter()
        .next()
        .expect("entry")
        .id;
    delete_auto_learned_entries_by_ids(&db, &[id]).expect("reject");

    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate again");
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("s1"),
        AutoLearnPromoteResult::BelowThreshold { pending_count: 1 }
    );
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("s2"),
        AutoLearnPromoteResult::Promoted
    );
    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].correction_count, 1);
}

#[test]
fn auto_learn_promote_manual_entry_blocks_without_claiming() {
    let db = test_db();
    insert_dictionary_entry(&db, "Kubernetes", Some("Koobernetes")).expect("manual");
    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("pending 1");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("pending 2");

    for _ in 0..2 {
        assert_eq!(
            auto_learn_promote_for_context(
                &db,
                EVERYWHERE_CONTEXT_ID,
                "Koobernetes",
                "Kubernetes",
                "medium",
                2,
                2,
            )
            .expect("promote"),
            AutoLearnPromoteResult::Blocked
        );
    }
    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].term, "Kubernetes");
    assert_eq!(entries[0].mistake.as_deref(), Some("Koobernetes"));
    assert!(!entries[0].auto_learned);

    let conn = lock_conn(&db).expect("lock");
    let promoted_at: Option<String> = conn
        .query_row(
            "SELECT promoted_at FROM auto_learn_candidates WHERE wrong_word='Koobernetes'",
            [],
            |r| r.get(0),
        )
        .expect("promoted_at");
    assert!(
        promoted_at.is_none(),
        "gate must not be claimed for a manual block"
    );
    drop(conn);
}

#[test]
fn cleanup_cache_insert_get_and_clear() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "abc", "hello", "2999-01-01 00:00:00", false).expect("insert");
    let hit = cleanup_cache_get_active(&db, "abc")
        .expect("query")
        .expect("exists");
    assert_eq!(hit.clean_text, "hello");
    assert_eq!(hit.hit_count, 1);

    assert_eq!(cleanup_cache_count(&db).expect("count"), 1);
    assert_eq!(cleanup_cache_clear_all(&db).expect("clear"), 1);
    assert_eq!(cleanup_cache_count(&db).expect("count"), 0);
}

#[test]
fn cleanup_cache_prunes_expired_only() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "old", "x", "2000-01-01 00:00:00", false).expect("insert old");
    cleanup_cache_insert_new(&db, "live", "y", "2999-01-01 00:00:00", false).expect("insert live");

    assert_eq!(cleanup_cache_prune_expired(&db).expect("prune"), 1);
    assert!(cleanup_cache_get_active(&db, "old")
        .expect("query old")
        .is_none());
    assert!(cleanup_cache_get_active(&db, "live")
        .expect("query live")
        .is_some());
}

#[test]
fn cleanup_cache_reads_and_pruning_share_idle_and_maximum_lifetimes() {
    let db = test_db();
    for (key, snippet) in [("idle", false), ("snippet-idle", true), ("old", false), ("snippet-old", true)] {
        cleanup_cache_insert_new(&db, key, "public synthetic result", "2999-01-01 00:00:00", snippet).unwrap();
    }
    {
        let conn = lock_conn(&db).unwrap();
        conn.execute_batch("UPDATE cleanup_cache SET last_hit_at = datetime('now', '-2 days'),
            last_hit_at_epoch = CAST(strftime('%s', 'now', '-2 days') AS INTEGER) WHERE key LIKE '%idle';
            UPDATE cleanup_cache SET created_at = datetime('now', '-7 days'), created_at_epoch = NULL
            WHERE key LIKE '%old';").unwrap();
    }
    for key in ["idle", "snippet-idle", "old", "snippet-old"] {
        assert!(cleanup_cache_get_active(&db, key).unwrap().is_none());
    }
    assert_eq!(cleanup_cache_prune_expired(&db).unwrap(), 4);
}

#[test]
fn cleanup_cache_payload_counts_utf8_bytes_and_disabled_insert_is_a_noop() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "key", "é", "2999-01-01 00:00:00", false).unwrap();
    assert_eq!(cleanup_cache_payload_bytes(&db).unwrap(), 5);
    cleanup_cache_clear_all(&db).unwrap();
    cleanup_cache_insert_if(&db, "key", "result", "2999-01-01 00:00:00", false, || false).unwrap();
    assert_eq!(cleanup_cache_count(&db).unwrap(), 0);
    assert_eq!(cleanup_cache_payload_bytes(&db).unwrap(), 0);
}

#[test]
fn cleanup_cache_upgrade_discards_legacy_keys_and_preserves_other_data() {
    let path = temp_db_path("cache_key_upgrade");
    let key = format!("cleanup-v2:{}", "a".repeat(64));
    {
        let db = open(path.to_str().unwrap()).unwrap();
        insert_dictionary_entry(&db, "Synthetic", None).unwrap();
        cleanup_cache_insert_new(&db, "readable old input", "old result", "2999-01-01 00:00:00", false).unwrap();
        cleanup_cache_insert_new(&db, &key, "new result", "2999-01-01 00:00:00", false).unwrap();
    }
    {
        let db = open(path.to_str().unwrap()).unwrap();
        assert_eq!(cleanup_cache_count(&db).unwrap(), 1);
        assert!(cleanup_cache_get_active(&db, &key).unwrap().is_some());
        assert_eq!(query_dictionary(&db).unwrap()[0].term, "Synthetic");
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn cleanup_cache_enforces_a_byte_budget_opportunistically() {
    let db = test_db();
    let oversized = "x".repeat((CLEANUP_CACHE_MAX_BYTES + 1) as usize);
    cleanup_cache_insert_new(&db, "oversized", &oversized, "2999-01-01 00:00:00", false)
        .expect("insert oversized cache entry");
    assert_eq!(cleanup_cache_count(&db).expect("count"), 0);
}

#[test]
fn cleanup_cache_get_active_supports_null_epoch_fallback() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "legacy", "hello", "2999-01-01 00:00:00", false).expect("insert");

    let conn = lock_conn(&db).expect("lock");
    conn.execute(
        "UPDATE cleanup_cache
         SET expires_at_epoch = NULL
         WHERE key = 'legacy'",
        [],
    )
    .expect("null out epoch");
    drop(conn);

    assert!(cleanup_cache_get_active(&db, "legacy")
        .expect("query")
        .is_some());
}

#[test]
fn cleanup_cache_get_active_handles_null_created_and_last_hit_epochs() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "partial", "hello", "2999-01-01 00:00:00", false)
        .expect("insert");

    let conn = lock_conn(&db).expect("lock");
    conn.execute(
        "UPDATE cleanup_cache
         SET created_at_epoch = NULL,
             last_hit_at_epoch = NULL
         WHERE key = 'partial'",
        [],
    )
    .expect("null out partial epochs");
    drop(conn);

    let row = cleanup_cache_get_active(&db, "partial")
        .expect("query")
        .expect("row exists");
    assert_eq!(row.key, "partial");
    assert_eq!(row.clean_text, "hello");
}

#[test]
fn cleanup_cache_epoch_columns_treat_utc_text_as_utc() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "utc", "value", "2026-01-01 00:00:00", false).expect("insert");

    let conn = lock_conn(&db).expect("lock");
    let (inserted_expiry_epoch, created_at): (i64, String) = conn
        .query_row(
            "SELECT expires_at_epoch, created_at FROM cleanup_cache WHERE key = 'utc'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("select insert epoch");
    drop(conn);
    assert_eq!(inserted_expiry_epoch, 1_767_225_600);

    cleanup_cache_touch_hit(
        &db,
        "utc",
        &created_at,
        1,
        2,
        "2026-01-02 03:04:05",
        "2026-02-03 04:05:06",
    )
    .expect("touch");
    let conn = lock_conn(&db).expect("lock");
    let (last_hit_epoch, expires_epoch): (i64, i64) = conn
        .query_row(
            "SELECT last_hit_at_epoch, expires_at_epoch FROM cleanup_cache WHERE key = 'utc'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("select touched epochs");

    assert_eq!(last_hit_epoch, 1_767_323_045);
    assert_eq!(expires_epoch, 1_770_091_506);
}

#[test]
fn cache_rejection_delete_removes_entry() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "key1", "bad answer", "2999-01-01 00:00:00", false)
        .expect("insert");

    assert!(cleanup_cache_get_active(&db, "key1")
        .expect("get")
        .is_some());

    cleanup_cache_delete_by_key(&db, "key1").expect("delete");

    assert!(cleanup_cache_get_active(&db, "key1")
        .expect("get after")
        .is_none());
    assert_eq!(cleanup_cache_count(&db).expect("count"), 0);
}

#[test]
fn cache_rejection_leaves_other_keys_intact() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "target", "bad", "2999-01-01 00:00:00", false).expect("target");
    cleanup_cache_insert_new(&db, "bystander", "good", "2999-01-01 00:00:00", false)
        .expect("bystander");

    cleanup_cache_delete_by_key(&db, "target").expect("delete");

    assert!(cleanup_cache_get_active(&db, "target")
        .expect("target")
        .is_none());
    assert!(
        cleanup_cache_get_active(&db, "bystander")
            .expect("bystander")
            .is_some(),
        "unrelated entry must survive"
    );
}

#[test]
fn cache_rejection_after_hit_removes_entry() {
    let db = test_db();
    cleanup_cache_insert_new(&db, "k", "stale text", "2999-01-01 00:00:00", false).expect("insert");

    let created_at = cleanup_cache_get_active(&db, "k")
        .expect("get")
        .expect("exists")
        .created_at;
    cleanup_cache_touch_hit(
        &db,
        "k",
        &created_at,
        1,
        2,
        &created_at,
        "2999-01-01 00:00:00",
    )
    .expect("touch");

    let hit = cleanup_cache_get_active(&db, "k")
        .expect("get")
        .expect("exists");
    assert_eq!(hit.hit_count, 2);

    cleanup_cache_delete_by_key(&db, "k").expect("delete");

    assert!(cleanup_cache_get_active(&db, "k")
        .expect("get after")
        .is_none());
}

#[test]
fn pruning_history_preserves_api_cost_rows_for_insights() {
    let db = test_db();
    let old = insert_transcription_returning(&db, "old", "old", 2, 1000, "test", None, None)
        .expect("old transcription");
    let recent =
        insert_transcription_returning(&db, "recent", "recent", 3, 1000, "test", None, None)
            .expect("recent transcription");
    let call = |transcription_id: i64| ApiCall {
        transcription_id,
        model: "test-model".into(),
        provider: "groq".into(),
        task: "transcribe".into(),
        audio_ms: 100,
        input_chars: 1,
        output_chars: 2,
        created_at: "2026-01-01 00:00:00".into(),
    };
    insert_api_calls(&db, &[call(old.id), call(recent.id)]).expect("insert api calls");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET created_at = datetime('now', '-30 days') WHERE clean_text = 'old'",
            [],
        )
        .expect("backdate old row");
    }

    assert_eq!(prune_transcriptions_older_than(&db, 7).expect("prune"), 1);

    let conn = lock_conn(&db).expect("lock");
    let remaining_calls: i64 = conn
        .query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get(0))
        .expect("remaining calls");
    assert_eq!(
        remaining_calls, 0,
        "old API calls are compacted after retention"
    );
    let orphan: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM api_calls WHERE transcription_id = ?1",
            [old.id],
            |r| r.get(0),
        )
        .expect("orphan count");
    assert_eq!(
        orphan, 0,
        "compacted cost rows no longer retain raw transcription links"
    );
    let provider_rollups: i64 = conn
        .query_row("SELECT COUNT(*) FROM provider_daily_stats", [], |r| {
            r.get(0)
        })
        .expect("provider rollups");
    assert!(
        provider_rollups > 0,
        "old API calls are represented by provider rollups"
    );
    let hourly_rollups: i64 = conn
        .query_row("SELECT COUNT(*) FROM transcription_hourly_stats", [], |r| {
            r.get(0)
        })
        .expect("hourly rollups");
    assert!(
        hourly_rollups > 0,
        "old transcriptions are represented by hourly rollups"
    );
    drop(conn);
}

#[test]
fn api_compaction_removes_calls_orphaned_by_history_cleanup() {
    let db = test_db();
    let transcription =
        insert_transcription_returning(&db, "old", "old", 2, 1_000, "test", None, None)
            .expect("transcription");
    insert_api_calls(
        &db,
        &[ApiCall {
            transcription_id: transcription.id,
            model: "test-model".into(),
            provider: "groq".into(),
            task: "transcribe".into(),
            audio_ms: 100,
            input_chars: 1,
            output_chars: 2,
            created_at: "2026-01-01 00:00:00".into(),
        }],
    )
    .expect("api call");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "DELETE FROM transcriptions WHERE id = ?1",
            [transcription.id],
        )
        .expect("delete transcription");
    }

    assert_eq!(compact_api_calls_older_than(&db, 90).expect("compact"), 1);
    let conn = lock_conn(&db).expect("lock after compact");
    let rollup_calls: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(calls), 0) FROM provider_daily_stats",
            [],
            |r| r.get(0),
        )
        .expect("provider rollup calls");
    assert_eq!(rollup_calls, 1);
}

#[test]
fn auto_learn_retention_prunes_only_stale_rows() {
    let db = test_db();
    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    upsert_auto_learn_candidate_for_context(&db, EVERYWHERE_CONTEXT_ID, "Tari", "Tauri", 0.6)
        .expect("candidate");
    log_auto_learn_event_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        AutoLearnEventFields {
            event_type: "monitor",
            reason_code: "started",
            app_context: "",
            mistake_hash: "",
            correction_hash: "",
            confidence: 0.0,
        },
    )
    .expect("event");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE auto_learn_events SET created_at = datetime('now', '-40 days')",
            [],
        )
        .expect("backdate events");
        conn.execute(
            "UPDATE auto_learn_candidates
             SET last_seen_at = datetime('now', '-100 days')
             WHERE wrong_word = 'Koobernetes'",
            [],
        )
        .expect("backdate stale candidate");
    }

    let pruned = prune_auto_learn_retention(&db).expect("prune");
    assert_eq!(pruned, 2, "one stale event + one stale candidate");

    let conn = lock_conn(&db).expect("lock");
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM auto_learn_events", [], |r| r.get(0))
        .expect("events");
    assert_eq!(events, 0, "all events were stale");
    let mut stmt = conn
        .prepare("SELECT wrong_word FROM auto_learn_candidates")
        .expect("prepare");
    let candidates: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("candidates")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect");
    drop(stmt);
    assert_eq!(
        candidates,
        vec!["Tari".to_string()],
        "fresh candidate survives"
    );
    drop(conn);
}

#[test]
fn auto_learn_promote_keeps_different_mistakes_independent() {
    let db = test_db();

    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Koobernetes",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("pending");
    insert_pending_correction(&db, "Koobernetes", "Kubernetes").expect("pending");
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("promote"),
        AutoLearnPromoteResult::Promoted
    );

    upsert_auto_learn_candidate_for_context(
        &db,
        EVERYWHERE_CONTEXT_ID,
        "Kubernetz",
        "Kubernetes",
        0.6,
    )
    .expect("candidate");
    insert_pending_correction(&db, "Kubernetz", "Kubernetes").expect("pending");
    insert_pending_correction(&db, "Kubernetz", "Kubernetes").expect("pending");
    assert_eq!(
        auto_learn_promote_for_context(
            &db,
            EVERYWHERE_CONTEXT_ID,
            "Kubernetz",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("second promote"),
        AutoLearnPromoteResult::Promoted
    );

    let conn = lock_conn(&db).expect("lock");
    let promoted_at: Option<String> = conn
        .query_row(
            "SELECT promoted_at FROM auto_learn_candidates WHERE wrong_word = 'Kubernetz'",
            [],
            |r| r.get(0),
        )
        .expect("gate");
    assert!(
        promoted_at.is_some(),
        "an independently valid mapping must claim its own gate"
    );
    drop(conn);

    let entries = query_dictionary(&db).expect("dictionary");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].term, "Kubernetes");
    assert_eq!(
        entries[0].mistake.as_deref(),
        Some("Koobernetes, Kubernetz")
    );
    assert_eq!(entries[0].corrections.len(), 2);
}

#[test]
fn stats_total_words_uses_spoken_count_excluding_snippet_triggers() {
    let db = test_db();
    insert_snippet(
        &db,
        "sig",
        "A long email signature with a bunch of words",
        "",
    )
    .expect("snippet");
    insert_transcription_returning(
        &db,
        "sig.",
        "A long email signature",
        9,
        2000,
        "test",
        None,
        None,
    )
    .expect("transcription");

    let stats = query_stats(&db).expect("stats");

    assert_eq!(stats.total_words, 0);
    assert_eq!(stats.avg_wpm, 0.0);
}

#[test]
fn stats_avg_wpm_counts_only_non_snippet_spoken_words() {
    let db = test_db();
    insert_snippet(&db, "sig", "A long email signature", "").expect("snippet");
    insert_transcription_returning(
        &db,
        "please add sig thanks",
        "Please add signature",
        4,
        2000,
        "test",
        None,
        None,
    )
    .expect("transcription");

    let stats = query_stats(&db).expect("stats");

    assert_eq!(stats.avg_wpm, 90.0);
}

#[test]
fn stats_avg_wpm_excludes_pure_snippet_rows_from_average() {
    let db = test_db();
    insert_snippet(&db, "sig", "A long email signature", "").expect("snippet");
    insert_transcription_returning(
        &db,
        "hello world",
        "hello world",
        2,
        1000,
        "test",
        None,
        None,
    )
    .expect("normal transcription");
    insert_transcription_returning(
        &db,
        "sig.",
        "A long email signature",
        4,
        1000,
        "test",
        None,
        None,
    )
    .expect("snippet transcription");

    let stats = query_stats(&db).expect("stats");

    assert_eq!(stats.avg_wpm, 120.0);
}

#[test]
fn stats_avg_wpm_streams_large_transcription_sets() {
    let db = test_db();
    insert_snippet(&db, "sig", "signature block", "").expect("snippet");

    for idx in 0..500 {
        insert_transcription_returning(
            &db,
            if idx % 2 == 0 {
                "hello world"
            } else {
                "hello sig world"
            },
            "clean",
            2,
            1000,
            "test",
            None,
            None,
        )
        .expect("transcription");
    }

    let stats = query_stats(&db).expect("stats");

    assert_eq!(stats.total_words, 1000);
    assert!(stats.avg_wpm > 0.0);
}

#[test]
fn prune_transcriptions_older_than_deletes_only_old_rows() {
    let db = test_db();
    insert_transcription_returning(&db, "old one", "old one", 2, 1000, "test", None, None)
        .expect("old transcription");
    insert_transcription_returning(&db, "recent one", "recent one", 2, 1000, "test", None, None)
        .expect("recent transcription");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET created_at = datetime('now', '-30 days') WHERE clean_text = 'old one'",
            [],
        )
        .expect("backdate old row");
    }

    assert_eq!(count_transcriptions_older_than(&db, 7).expect("count"), 1);
    assert_eq!(prune_transcriptions_older_than(&db, 7).expect("prune"), 1);

    let conn = lock_conn(&db).expect("lock");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0))
        .expect("remaining count");
    assert_eq!(remaining, 1);
    let remaining_text: String = conn
        .query_row("SELECT clean_text FROM transcriptions", [], |r| r.get(0))
        .expect("remaining text");
    assert_eq!(remaining_text, "recent one");
}

#[test]
fn pruning_old_transcriptions_does_not_reduce_lifetime_word_total() {
    let db = test_db();
    insert_transcription_returning(&db, "old one", "old one", 2, 1000, "test", None, None)
        .expect("old transcription");
    insert_transcription_returning(&db, "recent one", "recent one", 2, 1000, "test", None, None)
        .expect("recent transcription");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET created_at = datetime('now', '-30 days') WHERE clean_text = 'old one'",
            [],
        )
        .expect("backdate old row");
    }

    let before = query_stats(&db).expect("stats before prune").total_words;
    assert_eq!(before, 4);

    let deleted = prune_transcriptions_older_than(&db, 7).expect("prune");
    assert_eq!(deleted, 1);

    let after = query_stats(&db).expect("stats after prune").total_words;
    assert_eq!(
        after, 4,
        "lifetime word counter must not shrink when old history is pruned"
    );
}

#[test]
fn pruning_old_transcriptions_preserves_daily_insights_and_lifetime_wpm() {
    let db = test_db();
    insert_transcription_returning(&db, "old one", "old one", 2, 2_000, "test", None, None)
        .expect("old transcription");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET created_at = datetime('now', '-30 days') WHERE clean_text = 'old one'",
            [],
        )
        .expect("backdate old row");
    }

    let before = query_insights(&db, 0, None).expect("insights before prune");
    let old_day = before
        .daily
        .iter()
        .find(|day| day.words == 2)
        .expect("daily summary")
        .day
        .clone();
    assert_eq!(before.streak.longest_days, 1);

    prune_transcriptions_older_than(&db, 7).expect("prune");

    let after = query_insights(&db, 0, None).expect("insights after prune");
    let preserved = after
        .daily
        .iter()
        .find(|day| day.day == old_day)
        .expect("preserved daily summary");
    assert_eq!(preserved.words, 2);
    assert_eq!(preserved.transcriptions, 1);
    assert_eq!(preserved.speaking_ms, 2_000);
    assert_eq!(after.totals.total_transcriptions, 1);
    assert!(after.totals.avg_wpm > 0.0);
}

#[test]
fn repair_aligns_legacy_inflated_counts_and_refunds_lifetime() {
    let db = test_db();
    insert_snippet(&db, "sig", "Best regards", "").expect("snippet");
    {
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "INSERT INTO transcriptions (raw_text, clean_text, words, spoken_words, duration_ms, api_used)
             VALUES ('please add sig thanks ...', 'Please add thanks', 5, NULL, 2000, 'test')",
            [],
        )
        .expect("legacy row");
        conn.execute("UPDATE lifetime_stats SET total_words = 5 WHERE id = 1", [])
            .expect("lifetime");
    }
    {
        let mut conn = lock_conn(&db).expect("lock");
        repair_inflated_word_counts(&mut conn).expect("repair");
        repair_inflated_word_counts(&mut conn).expect("repair again");
    }
    let conn = lock_conn(&db).expect("lock");
    let words: i64 = conn
        .query_row("SELECT words FROM transcriptions", [], |r| r.get(0))
        .expect("words");
    assert_eq!(words, 3);
    let lifetime: i64 = conn
        .query_row(
            "SELECT total_words FROM lifetime_stats WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("lifetime");
    assert_eq!(lifetime, 3);
    let daily: i64 = conn
        .query_row(
            "SELECT total_words FROM transcription_daily_stats",
            [],
            |r| r.get(0),
        )
        .expect("daily");
    assert_eq!(daily, 3);
}

#[test]
fn dict_rejection_only_removes_auto_learned_entries() {
    let db = test_db();

    insert_dictionary_entry(&db, "groq", Some("grog")).expect("manual");
    let manual_id = query_dictionary(&db)
        .expect("query")
        .into_iter()
        .find(|e| e.term == "groq")
        .expect("find")
        .id;

    insert_dictionary_entry_auto_learned(&db, "Tauri", Some("Tari"), "high").expect("auto");
    let auto_id = query_dictionary(&db)
        .expect("query")
        .into_iter()
        .find(|e| e.term == "Tauri")
        .expect("find")
        .id;

    delete_auto_learned_entries_by_ids(&db, &[manual_id, auto_id]).expect("reject");

    let remaining: Vec<_> = query_dictionary(&db).expect("query after");
    assert_eq!(remaining.len(), 1, "only manual entry survives");
    assert_eq!(remaining[0].term, "groq");
}

#[test]
fn dict_rejection_cleans_up_pending_corrections() {
    let db = test_db();

    insert_dictionary_entry_auto_learned(&db, "Tauri", Some("Tari"), "low").expect("insert");
    let id = query_dictionary(&db)
        .expect("query")
        .into_iter()
        .next()
        .expect("entry")
        .id;

    insert_pending_correction(&db, "Tari", "Tauri").expect("pending 1");
    insert_pending_correction(&db, "Tari", "Tauri").expect("pending 2");
    assert_eq!(
        count_pending_corrections_recent(&db, EVERYWHERE_CONTEXT_ID, "Tari", "Tauri", 7)
            .expect("count"),
        2
    );

    delete_auto_learned_entries_by_ids(&db, &[id]).expect("reject");

    assert_eq!(query_dictionary(&db).expect("query after").len(), 0);
    assert_eq!(
        count_pending_corrections_recent(&db, EVERYWHERE_CONTEXT_ID, "Tari", "Tauri", 7,)
            .expect("count after"),
        0
    );
}

#[test]
fn cache_rejection_full_lifecycle() {
    let db = test_db();
    let key = "chromium-is-a-web-browser-base";
    let bad_answer = "bad cached answer";

    cleanup_cache_insert_new(&db, key, bad_answer, "2999-01-01 00:00:00", false).expect("insert");
    assert_eq!(cleanup_cache_count(&db).expect("count"), 1);

    let entry = cleanup_cache_get_active(&db, key)
        .expect("get")
        .expect("hit");
    assert_eq!(entry.clean_text, bad_answer);
    cleanup_cache_touch_hit(
        &db,
        key,
        &entry.created_at,
        1,
        2,
        "2026-01-01 00:00:00",
        "2999-01-01 00:00:00",
    )
    .expect("touch");

    cleanup_cache_delete_by_key(&db, key).expect("delete");

    assert!(
        cleanup_cache_get_active(&db, key)
            .expect("get after")
            .is_none(),
        "cache must be empty after rejection so next dictation hits the LLM"
    );
}

#[test]
fn dict_rejection_bulk_removes_multiple_entries() {
    let db = test_db();
    insert_dictionary_entry_auto_learned(&db, "groq", Some("grog"), "high").expect("1");
    insert_dictionary_entry_auto_learned(&db, "Tauri", Some("Tari"), "high").expect("2");
    let ids: Vec<i64> = query_dictionary(&db)
        .expect("query")
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids.len(), 2);
    delete_auto_learned_entries_by_ids(&db, &ids).expect("bulk delete");
    assert_eq!(query_dictionary(&db).expect("after").len(), 0);
}

#[test]
fn manual_dictionary_entries_trim_and_keep_longer_phrases() {
    let db = test_db();
    let long_term =
        "A longer dictionary phrase that still fits inside the supported limit exactly fine";
    let long_mistake = "A slightly mangled version of that longer phrase for recognition";

    insert_dictionary_entry(
        &db,
        &format!("  {long_term}  "),
        Some(&format!("  {long_mistake}  ")),
    )
    .expect("insert trimmed long entry");

    let entry = query_dictionary(&db)
        .expect("query")
        .into_iter()
        .next()
        .expect("entry");
    assert_eq!(entry.term, long_term);
    assert_eq!(entry.mistake.as_deref(), Some(long_mistake));
}

#[test]
fn dictionary_entry_rejects_values_beyond_limit() {
    let db = test_db();
    let too_long = "x".repeat(DICTIONARY_ENTRY_CHAR_LIMIT + 1);
    let err = insert_dictionary_entry(&db, &too_long, None).expect_err("reject long term");
    assert!(
        err.to_string().contains("120 characters or fewer"),
        "unexpected error: {err}"
    );
}

#[test]
fn snippet_insert_rolls_back_when_context_assignment_fails() {
    let db = test_db();
    let existing = insert_snippet_returning(&db, "existing", "Synthetic expansion", "", None)
        .expect("seed preexisting snippet");
    let before = serde_json::to_value(query_snippets(&db).unwrap()).unwrap();
    let assignments_before: i64 = db
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM snippet_contexts", [], |row| row.get(0))
        .unwrap();

    let error = insert_snippet_returning(&db, "new", "Synthetic new expansion", "", Some(i64::MAX))
        .expect_err("missing context must reject assignment");
    assert_eq!(
        error.downcast_ref::<rusqlite::Error>().unwrap().sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        serde_json::to_value(query_snippets(&db).unwrap()).unwrap(),
        before
    );
    let conn = db.lock().unwrap();
    assert!(conn.is_autocommit());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM snippet_contexts", [], |row| row.get::<_, i64>(0)).unwrap(),
        assignments_before
    );
    assert_eq!(
        conn.query_row("SELECT snippet_id FROM snippet_contexts WHERE snippet_id = ?1", [existing.id], |row| row.get::<_, i64>(0)).unwrap(),
        existing.id
    );
    drop(conn);

    let retry = insert_snippet_returning(&db, "new", "Synthetic new expansion", "", None)
        .expect("same trigger remains available after rollback");
    assert!(query_snippets(&db).unwrap().iter().any(|snippet| snippet.id == retry.id));
}

#[test]
fn snippet_update_normalizes_expansion_whitespace() {
    let db = test_db();
    insert_snippet(&db, "sig", "Hi", "").expect("insert");
    let snippet = query_snippets(&db)
        .expect("query")
        .into_iter()
        .next()
        .expect("snippet");
    update_snippet(&db, snippet.id, "sig", "Line one\r\nLine two  \n", "").expect("update");
    let updated = query_snippets(&db)
        .expect("query after")
        .into_iter()
        .next()
        .expect("updated");
    assert_eq!(updated.expansion, "Line one\nLine two");
}

#[test]
fn stale_library_edit_cannot_overwrite_agent_change() {
    let db = test_db();
    let context = insert_context_returning(&db, "Synthetic", None, None, None, None, false).unwrap();
    let entry = insert_dictionary_entry_returning(&db, "Before", Some("old"), Some(context.id)).unwrap();
    let expected = ExpectedDictionaryEntry { term: "Before".into(), mistake: Some("old".into()) };
    update_dictionary_entry_for_context(&db, context.id, entry.id, "Before", Some("new")).unwrap();
    assert!(update_dictionary_entry_if_unchanged(&db, Some(context.id), entry.id, "Renamed", Some("old"), &expected).is_err());
    let current = query_effective_dictionary_for_context(&db, context.id).unwrap();
    assert_eq!(current[0].term, "Before");
    assert_eq!(current[0].mistake.as_deref(), Some("new"));

    let snippet = insert_snippet_returning(&db, "sig", "old", "", None).unwrap();
    let expected = ExpectedSnippet { trigger: "sig".into(), expansion: "old".into(), instructions: "".into() };
    update_snippet(&db, snippet.id, "sig", "new", "").unwrap();
    assert!(update_snippet_if_unchanged(&db, snippet.id, "signature", "old", "", Some(&expected)).is_err());
    let current = query_snippets(&db).unwrap();
    assert_eq!(current[0].trigger, "sig");
    assert_eq!(current[0].expansion, "new");

    let global = insert_dictionary_entry_returning(&db, "Global", Some("global"), None).unwrap();
    let expected = ExpectedDictionaryEntry { term: "Global".into(), mistake: None };
    update_dictionary_entry_if_unchanged(&db, Some(context.id), global.id, "Global", Some("local"), &expected).unwrap();
    assert!(update_dictionary_entry_if_unchanged(&db, Some(context.id), global.id, "Global", Some("stale"), &expected).is_err());
    let current = query_effective_dictionary_for_context(&db, context.id).unwrap();
    let global = current.into_iter().find(|entry| entry.id == global.id).unwrap();
    assert_eq!(global.context_corrections[0].mistake, "local");
}

#[test]
fn snippet_move_and_edit_rolls_back_when_source_assignment_changed() {
    let db = test_db();
    let target = insert_context_returning(&db, "Target", None, None, None, None, false).unwrap();
    let other = insert_context_returning(&db, "Other", None, None, None, None, false).unwrap();
    let snippet = insert_snippet_returning(&db, "sig", "old", "", None).unwrap();
    let edit = SnippetMoveEdit { id: snippet.id, expected: ExpectedSnippet { trigger: "sig".into(), expansion: "old".into(), instructions: "".into() }, expansion: "draft".into(), instructions: "".into() };
    move_snippet_entry_to_context(&db, "sig", other.id, None).unwrap();
    assert!(move_snippet_entry_to_context(&db, "sig", target.id, Some(&edit)).is_err());
    assert_eq!(query_snippets(&db).unwrap()[0].expansion, "old");
    assert!(query_snippets_for_context(&db, target.id).unwrap().is_empty());
    assert_eq!(query_snippets_for_context(&db, other.id).unwrap()[0].id, snippet.id);
}

#[test]
fn deleting_missing_entries_returns_an_error() {
    let db = test_db();
    let dict_err = delete_dictionary_entry(&db, 999).expect_err("missing dictionary entry");
    assert!(
        dict_err
            .to_string()
            .contains("Dictionary entry 999 was not found"),
        "unexpected dictionary error: {dict_err}"
    );

    let snippet_err = delete_snippet(&db, 999).expect_err("missing snippet");
    assert!(
        snippet_err
            .to_string()
            .contains("Snippet 999 was not found"),
        "unexpected snippet error: {snippet_err}"
    );
}

#[test]
fn manual_delete_of_auto_learned_entry_purges_pending_corrections() {
    let db = test_db();

    insert_dictionary_entry_auto_learned(&db, "Tauri", Some("Tari"), "low").expect("insert");
    let id = query_dictionary(&db)
        .expect("query")
        .into_iter()
        .next()
        .expect("entry")
        .id;

    insert_pending_correction(&db, "Tari", "Tauri").expect("pending");
    assert_eq!(
        count_pending_corrections_recent(&db, EVERYWHERE_CONTEXT_ID, "Tari", "Tauri", 7)
            .expect("count before"),
        1
    );

    delete_dictionary_entry(&db, id).expect("delete");

    assert_eq!(query_dictionary(&db).expect("query after").len(), 0);
    assert_eq!(
        count_pending_corrections_recent(&db, EVERYWHERE_CONTEXT_ID, "Tari", "Tauri", 7,)
            .expect("count after"),
        0,
        "pending corrections must be purged when the auto-learned entry is manually deleted"
    );
}
