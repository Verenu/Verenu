use super::*;
use rusqlite::params;
use uuid::Uuid;

fn temp_db_path() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "verenu_autolearn_v26_{}_{}.db",
        std::process::id(),
        nanos
    ))
}

fn remove_db_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn v25_reopen_runs_schema_before_scoped_autolearn_migration() {
    let path = temp_db_path();
    {
        let db = open(&path).expect("create fixture database");
        let development =
            insert_context_returning(&db, "Development", None, None, None, None, false)
                .expect("development context");
        insert_dictionary_entry(&db, "SharedTerm", Some("shared typo")).expect("manual term");
        let shared_id = query_dictionary(&db)
            .expect("dictionary")
            .into_iter()
            .find(|entry| entry.term == "SharedTerm")
            .expect("shared canonical row")
            .id;
        set_dictionary_context_assignment(&db, development.id, shared_id, true)
            .expect("share manual term");
        // Keep a malformed legacy collision in the fixture as well: the
        // manual mapping must win over an old automatic row when v26 creates
        // the new Context/variant uniqueness constraint.
        {
            let conn = lock_conn(&db).expect("collision fixture lock");
            conn.execute(
                "INSERT INTO dictionary (term, mistake, auto_learned, confidence_tier)
                 VALUES ('AutoCollision', 'shared typo', 1, 'high')",
                [],
            )
            .expect("legacy automatic collision");
            let collision_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO dictionary_contexts (context_id, dictionary_id)
                 VALUES (?1, ?2)",
                params![EVERYWHERE_CONTEXT_ID, collision_id],
            )
            .expect("collision Everywhere assignment");
        }
        insert_dictionary_entry_auto_learned(&db, "LegacyLearned", Some("legacy typo"), "high")
            .expect("historical auto-learned term");
        let legacy_id = query_dictionary(&db)
            .expect("dictionary after auto-learn")
            .into_iter()
            .find(|entry| entry.term == "LegacyLearned")
            .expect("legacy canonical row")
            .id;
        // A targeted assignment without reliable origin evidence must not
        // cause the old automatic mapping to be copied into that Context.
        set_dictionary_context_assignment(&db, development.id, legacy_id, true)
            .expect("share legacy canonical term");

        let conn = lock_conn(&db).expect("fixture lock");
        // Make the current database look like a real v25 install: the old
        // global projection is populated, the child table did not exist, and
        // transient evidence has no originating Context column.
        conn.execute("DELETE FROM dictionary_corrections", [])
            .expect("clear v26 child rows");
        conn.execute(
            "UPDATE dictionary
                SET mistake = CASE term
                    WHEN 'SharedTerm' THEN 'shared typo'
                    WHEN 'AutoCollision' THEN 'shared typo'
                    WHEN 'LegacyLearned' THEN 'legacy typo'
                    ELSE mistake
                END
              WHERE term IN ('SharedTerm', 'AutoCollision', 'LegacyLearned')",
            [],
        )
        .expect("restore legacy correction projection");
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS trg_dictionary_contexts_delete_corrections;
             DROP TABLE dictionary_corrections;
             DROP INDEX IF EXISTS idx_pending_words;
             ALTER TABLE pending_corrections RENAME TO pending_corrections_v26;
             CREATE TABLE pending_corrections (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO pending_corrections (wrong_word, correct_word)
               VALUES ('ambiguous typo', 'TargetTerm');
             DROP TABLE pending_corrections_v26;
             DROP INDEX IF EXISTS idx_auto_learn_candidates_seen;
             ALTER TABLE auto_learn_candidates RENAME TO auto_learn_candidates_v26;
             CREATE TABLE auto_learn_candidates (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               confidence_sum REAL NOT NULL DEFAULT 0.0,
               confidence_avg REAL NOT NULL DEFAULT 0.0,
               seen_count INTEGER NOT NULL DEFAULT 0,
               last_seen_at DATETIME NOT NULL DEFAULT (datetime('now')),
               cooldown_until DATETIME,
               promoted_at DATETIME,
               UNIQUE(wrong_word, correct_word)
             );
             INSERT INTO auto_learn_candidates
               (wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
               VALUES ('ambiguous typo', 'TargetTerm', 0.7, 0.7, 1);
             DROP TABLE auto_learn_candidates_v26;
             DROP TABLE auto_learn_events;
             CREATE TABLE auto_learn_events (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               event_type TEXT NOT NULL,
               reason_code TEXT NOT NULL DEFAULT '',
               app_context TEXT NOT NULL DEFAULT '',
               mistake_hash TEXT NOT NULL DEFAULT '',
               correction_hash TEXT NOT NULL DEFAULT '',
               confidence REAL NOT NULL DEFAULT 0.0,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT INTO auto_learn_events (event_type, reason_code)
               VALUES ('candidate', 'legacy_fixture');
             PRAGMA user_version = 25;",
        )
        .expect("downgrade fixture to v25 shapes");
    }

    let db = open(&path).expect("v25 database must reopen through v26");
    let conn = lock_conn(&db).expect("repaired lock");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 26);
    assert!(table_has_column(&conn, "pending_corrections", "context_id").expect("pending scope"));
    assert!(
        table_has_column(&conn, "auto_learn_candidates", "context_id").expect("candidate scope")
    );
    assert!(table_has_column(&conn, "auto_learn_events", "context_id").expect("event scope"));
    let transient_rows: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM pending_corrections)
                  + (SELECT COUNT(*) FROM auto_learn_candidates)",
            [],
            |row| row.get(0),
        )
        .expect("transient row count");
    assert_eq!(transient_rows, 0, "ambiguous v25 evidence is discarded");
    let global_projection: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary WHERE mistake IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("legacy projection count");
    assert_eq!(global_projection, 0, "global mistake projection is retired");
    let correction_uuids: Vec<String> = conn
        .prepare("SELECT uuid FROM dictionary_corrections ORDER BY id")
        .expect("correction UUID query")
        .query_map([], |row| row.get(0))
        .expect("correction UUID rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("correction UUID collection");
    assert_eq!(
        correction_uuids.len(),
        3,
        "one shared row per assigned Context"
    );
    assert!(correction_uuids
        .iter()
        .all(|uuid| Uuid::parse_str(uuid).is_ok()));

    let shared_id: i64 = conn
        .query_row(
            "SELECT id FROM dictionary WHERE term = 'SharedTerm'",
            [],
            |row| row.get(0),
        )
        .expect("shared id");
    let shared_contexts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dictionary_corrections
              WHERE dictionary_id = ?1 AND mistake = 'shared typo'",
            params![shared_id],
            |row| row.get(0),
        )
        .expect("shared mappings");
    assert_eq!(
        shared_contexts, 2,
        "manual sharing is preserved in migration"
    );
    let automatic_collision_mapping: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM dictionary_corrections c
               JOIN dictionary d ON d.id = c.dictionary_id
              WHERE d.term = 'AutoCollision' AND c.mistake = 'shared typo'",
            [],
            |row| row.get(0),
        )
        .expect("legacy collision mapping");
    assert_eq!(
        automatic_collision_mapping, 0,
        "manual legacy correction wins a same-Context automatic collision"
    );
    let everywhere_mapping: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM dictionary_corrections c
               JOIN dictionary d ON d.id = c.dictionary_id
              WHERE d.term = 'LegacyLearned' AND c.context_id = ?1
                AND c.mistake = 'legacy typo'",
            params![EVERYWHERE_CONTEXT_ID],
            |row| row.get(0),
        )
        .expect("Everywhere mapping");
    assert_eq!(
        everywhere_mapping, 1,
        "legacy promoted data stays in Everywhere"
    );
    let targeted_legacy_mapping: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM dictionary_corrections c
               JOIN dictionary d ON d.id = c.dictionary_id
               JOIN contexts ctx ON ctx.id = c.context_id
              WHERE d.term = 'LegacyLearned' AND ctx.name = 'Development'",
            [],
            |row| row.get(0),
        )
        .expect("targeted legacy mapping");
    assert_eq!(
        targeted_legacy_mapping, 0,
        "ambiguous legacy AutoLearn scope is not copied into Development"
    );
    drop(conn);
    drop(db);

    // Reopening an already-v26 database is idempotent and preserves the
    // mapping identities and count.
    let db = open(&path).expect("reopen migrated database");
    let conn = lock_conn(&db).expect("second repaired lock");
    let second_uuids: Vec<String> = conn
        .prepare("SELECT uuid FROM dictionary_corrections ORDER BY id")
        .expect("second UUID query")
        .query_map([], |row| row.get(0))
        .expect("second UUID rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("second UUID collection");
    assert_eq!(second_uuids, correction_uuids);
    drop(conn);
    drop(db);
    remove_db_files(&path);
}

#[test]
fn scoped_promotion_and_rejection_preserve_shared_canonical_identity() {
    let db = open(":memory:").expect("db");
    let development = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("development");
    let writing =
        insert_context_returning(&db, "Writing", None, None, None, None, false).expect("writing");

    for context_id in [development.id, writing.id] {
        upsert_auto_learn_candidate(&db, context_id, "Kubernetez", "Kubernetes", 0.6)
            .expect("candidate");
        assert_eq!(
            auto_learn_promote(&db, context_id, "Kubernetez", "Kubernetes", "medium", 2, 2)
                .expect("first observation"),
            AutoLearnPromoteResult::BelowThreshold { pending_count: 1 }
        );
    }
    assert!(query_dictionary_for_context(&db, development.id)
        .expect("development before threshold")
        .is_empty());
    assert!(query_dictionary_for_context(&db, writing.id)
        .expect("writing before threshold")
        .is_empty());
    assert_eq!(
        count_pending_corrections_recent_for_context(
            &db,
            development.id,
            "Kubernetez",
            "Kubernetes",
            2,
        )
        .expect("development evidence"),
        1
    );

    upsert_auto_learn_candidate(&db, development.id, "Kubernetez", "Kubernetes", 0.6)
        .expect("development second candidate");
    assert_eq!(
        auto_learn_promote(
            &db,
            development.id,
            "Kubernetez",
            "Kubernetes",
            "medium",
            2,
            2,
        )
        .expect("development promotion"),
        AutoLearnPromoteResult::Promoted
    );
    let development_entry = query_dictionary_for_context(&db, development.id)
        .expect("development dictionary")
        .into_iter()
        .find(|entry| entry.term == "Kubernetes")
        .expect("development learned entry");
    assert_eq!(development_entry.mistake.as_deref(), Some("Kubernetez"));
    assert!(development_entry.corrections[0].auto_learned);
    assert!(query_dictionary_for_context(&db, writing.id)
        .expect("writing remains isolated")
        .is_empty());

    upsert_auto_learn_candidate(&db, writing.id, "Kubernetez", "Kubernetes", 0.6)
        .expect("writing second candidate");
    assert_eq!(
        auto_learn_promote(&db, writing.id, "Kubernetez", "Kubernetes", "medium", 2, 2,)
            .expect("writing promotion"),
        AutoLearnPromoteResult::Promoted
    );

    let writing_mapping_id = query_dictionary_for_context(&db, writing.id)
        .expect("writing dictionary")
        .into_iter()
        .find(|entry| entry.term == "Kubernetes")
        .and_then(|entry| entry.corrections.into_iter().next())
        .expect("writing mapping")
        .id;
    assert_eq!(
        delete_auto_learned_corrections_by_ids(
            &db,
            development.id,
            &[development_entry.corrections[0].id]
        )
        .expect("reject development mapping"),
        1
    );
    assert!(query_dictionary_for_context(&db, development.id)
        .expect("development after rejection")
        .into_iter()
        .find(|entry| entry.term == "Kubernetes")
        .expect("development canonical remains")
        .corrections
        .is_empty());
    let writing_after = query_dictionary_for_context(&db, writing.id)
        .expect("writing after development rejection")
        .into_iter()
        .find(|entry| entry.term == "Kubernetes")
        .expect("writing mapping remains");
    assert_eq!(writing_after.corrections[0].id, writing_mapping_id);
    assert_eq!(
        count_pending_corrections_recent_for_context(
            &db,
            development.id,
            "Kubernetez",
            "Kubernetes",
            2,
        )
        .expect("development evidence after rejection"),
        0
    );
}

#[test]
fn correction_rows_capture_stable_sync_identity_and_delete_events() {
    let db = open(":memory:").expect("db");
    let context = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    insert_dictionary_entry_auto_learned_for_context(
        &db,
        context.id,
        "Rust",
        Some("Russt"),
        "high",
    )
    .expect("mapping");

    let (mapping_id, mapping_uuid): (i64, String) = {
        let conn = lock_conn(&db).expect("lock");
        conn.query_row(
            "SELECT id, uuid FROM dictionary_corrections
              WHERE context_id = ?1",
            params![context.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("mapping identity")
    };
    assert!(Uuid::parse_str(&mapping_uuid).is_ok());
    let conn = lock_conn(&db).expect("sync log lock");
    let insert_log: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_log
              WHERE table_name = 'dictionary_corrections'
                AND row_uuid = ?1 AND op = 'upsert'",
            params![mapping_uuid],
            |row| row.get(0),
        )
        .expect("insert sync log");
    assert!(insert_log >= 1);
    drop(conn);
    delete_auto_learned_corrections_by_ids(&db, context.id, &[mapping_id]).expect("delete mapping");
    let conn = lock_conn(&db).expect("delete log lock");
    let delete_log: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_log
              WHERE table_name = 'dictionary_corrections'
                AND row_uuid = ?1 AND op = 'delete'",
            params![mapping_uuid],
            |row| row.get(0),
        )
        .expect("delete sync log");
    assert!(delete_log >= 1);
}

#[test]
fn context_edit_replaces_only_its_mapping_and_ignores_stale_global_projection() {
    let db = open(":memory:").expect("db");
    let development = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("development");
    let writing =
        insert_context_returning(&db, "Writing", None, None, None, None, false).expect("writing");
    let entry = insert_dictionary_entry_returning(&db, "Groq", Some("grock"), Some(development.id))
        .expect("manual development mapping");
    set_dictionary_context_assignment(&db, writing.id, entry.id, true)
        .expect("share canonical term");
    insert_dictionary_entry_auto_learned_for_context(
        &db,
        writing.id,
        "Groq",
        Some("grocker"),
        "high",
    )
    .expect("independent writing mapping");

    update_dictionary_entry_for_context(&db, development.id, entry.id, "Groq", Some("grok"))
        .expect("edit development mapping");

    // Simulate a stale write from a pre-v26/remote legacy consumer. Context
    // materialization must remain child-table-only even if the compatibility
    // column is unexpectedly populated after migration.
    {
        let conn = lock_conn(&db).expect("stale projection lock");
        conn.execute(
            "UPDATE dictionary SET mistake = 'must not leak' WHERE id = ?1",
            params![entry.id],
        )
        .expect("stale projection");
    }

    let development_entry = query_dictionary_for_context(&db, development.id)
        .expect("development dictionary")
        .into_iter()
        .find(|row| row.id == entry.id)
        .expect("development entry");
    assert_eq!(development_entry.mistake.as_deref(), Some("grok"));
    assert_eq!(development_entry.corrections.len(), 1);
    assert!(!development_entry.corrections[0].auto_learned);

    let writing_entry = query_dictionary_for_context(&db, writing.id)
        .expect("writing dictionary")
        .into_iter()
        .find(|row| row.id == entry.id)
        .expect("writing entry");
    assert_eq!(writing_entry.mistake.as_deref(), Some("grocker"));
    assert_eq!(writing_entry.corrections.len(), 1);
    assert!(writing_entry.corrections[0].auto_learned);
}

#[test]
fn removing_an_auto_assignment_cleans_orphan_and_scoped_evidence() {
    let db = open(":memory:").expect("db");
    let context = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    insert_dictionary_entry_auto_learned_for_context(
        &db,
        context.id,
        "Kubernetes",
        Some("kubernetez"),
        "high",
    )
    .expect("mapping");
    let dictionary_id = query_dictionary_for_context(&db, context.id)
        .expect("context dictionary")
        .into_iter()
        .find(|entry| entry.term == "Kubernetes")
        .expect("entry")
        .id;
    {
        let conn = lock_conn(&db).expect("evidence lock");
        conn.execute(
            "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
             VALUES (?1, 'stale typo', 'Kubernetes')",
            params![context.id],
        )
        .expect("pending evidence");
        conn.execute(
            "INSERT INTO auto_learn_candidates
               (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count)
             VALUES (?1, 'stale typo', 'Kubernetes', 0.8, 0.8, 1)",
            params![context.id],
        )
        .expect("candidate evidence");
    }

    set_dictionary_context_assignment(&db, context.id, dictionary_id, false)
        .expect("remove assignment");

    assert!(query_dictionary_for_context(&db, context.id)
        .expect("context dictionary after removal")
        .is_empty());
    assert!(query_dictionary(&db)
        .expect("global dictionary after removal")
        .into_iter()
        .all(|entry| entry.term != "Kubernetes"));
    let conn = lock_conn(&db).expect("evidence verification lock");
    let evidence_rows: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM pending_corrections WHERE context_id = ?1)
                    + (SELECT COUNT(*) FROM auto_learn_candidates WHERE context_id = ?1)",
            params![context.id],
            |row| row.get(0),
        )
        .expect("evidence count");
    assert_eq!(evidence_rows, 0);
}

#[test]
fn context_event_and_backup_helpers_keep_child_mapping_metadata() {
    let db = open(":memory:").expect("db");
    let context = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    log_auto_learn_event_for_context(
        &db,
        context.id,
        "candidate",
        "test",
        "Development",
        "mistake-hash",
        "correction-hash",
        0.8,
    )
    .expect("context event");
    let event = get_recent_auto_learn_activity(&db, 1)
        .expect("recent event")
        .into_iter()
        .next()
        .expect("event row");
    assert_eq!(event.context_id, Some(context.id));

    {
        let conn = lock_conn(&db).expect("backup lock");
        insert_dictionary_entry_from_backup_conn(
            &conn,
            "BackupTerm",
            Some("BackupTypo"),
            true,
            "high",
            4,
        )
        .expect("backup entry");
    }
    let conn = lock_conn(&db).expect("backup verification lock");
    let (uuid, auto_learned, correction_count, mapping_context): (String, i64, i64, i64) = conn
        .query_row(
            "SELECT c.uuid, c.auto_learned, c.correction_count, c.context_id
               FROM dictionary_corrections c
               JOIN dictionary d ON d.id = c.dictionary_id
              WHERE d.term = 'BackupTerm'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("backup mapping");
    assert!(Uuid::parse_str(&uuid).is_ok());
    assert_eq!(auto_learned, 1);
    assert_eq!(correction_count, 4);
    assert_eq!(mapping_context, EVERYWHERE_CONTEXT_ID);
    let legacy_projection: Option<String> = conn
        .query_row(
            "SELECT mistake FROM dictionary WHERE term = 'BackupTerm'",
            [],
            |row| row.get(0),
        )
        .expect("backup legacy projection");
    assert!(legacy_projection.is_none());
}

#[test]
fn auto_learn_conflict_returns_blocked_without_claiming_candidate() {
    let db = open(":memory:").expect("db");
    let context = insert_context_returning(&db, "Development", None, None, None, None, false)
        .expect("context");
    insert_dictionary_entry_returning(&db, "ExistingTerm", Some("shared typo"), Some(context.id))
        .expect("manual mapping");
    upsert_auto_learn_candidate(&db, context.id, "shared typo", "NewTerm", 0.95)
        .expect("candidate");

    assert_eq!(
        auto_learn_promote(&db, context.id, "shared typo", "NewTerm", "high", 2, 1)
            .expect("promotion result"),
        AutoLearnPromoteResult::Blocked
    );

    let conn = lock_conn(&db).expect("verification lock");
    let promoted_at: Option<String> = conn
        .query_row(
            "SELECT promoted_at FROM auto_learn_candidates
              WHERE context_id = ?1 AND wrong_word = 'shared typo' AND correct_word = 'NewTerm'",
            params![context.id],
            |row| row.get(0),
        )
        .expect("candidate row");
    assert!(promoted_at.is_none());
    drop(conn);
    assert!(!query_dictionary_for_context(&db, context.id)
        .expect("context dictionary")
        .iter()
        .any(|entry| entry.term == "NewTerm"));
}
