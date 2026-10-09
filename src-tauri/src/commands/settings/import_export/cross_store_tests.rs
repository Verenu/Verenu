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
                // The same completion boundary drives runtime updates, pruning,
                // and the production success event. Never access host credentials.
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
            // Model a concurrent ordinary setting save after import publishes.
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
    // A directory at the temporary file path gives a deterministic real I/O
    // error, including when tests run with elevated filesystem privileges.
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
    // Rollback must preserve actual bytes, even noncanonical whitespace.
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
