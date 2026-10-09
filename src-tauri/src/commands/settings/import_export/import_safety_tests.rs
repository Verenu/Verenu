use super::*;

fn valid_backup(version: &str) -> serde_json::Value {
    serde_json::json!({
        "version": version, "app_version": "synthetic", "exported_at": "synthetic",
        "settings": { "cleanup_enabled": false },
        "contexts": [{ "name": "Existing", "tone": "professional" }],
        "snippets": [{ "trigger": "synthetic", "expansion": "new", "instructions": "", "created_at": "synthetic" }]
    })
}

fn assert_rejected_without_restore(json: &str) {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE good_data (value TEXT); INSERT INTO good_data VALUES ('original');",
    )
    .unwrap();
    let mut settings = serde_json::json!({ "cleanup_enabled": true });
    let mut applied = false;
    let result = with_import_payload(json, |payload| {
        applied = true;
        settings = payload.settings;
        conn.execute("UPDATE good_data SET value = 'overwritten'", [])
            .unwrap();
        Ok(())
    });
    assert!(result.is_err());
    assert!(!applied, "invalid backup reached restore side effects");
    assert_eq!(settings, serde_json::json!({ "cleanup_enabled": true }));
    assert_eq!(
        conn.query_row("SELECT value FROM good_data", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "original"
    );
}

#[test]
fn truncated_import_never_reaches_restore_side_effects() {
    for version in ["1", "2"] {
        let json = serde_json::to_string(&valid_backup(version)).unwrap();
        for end in 0..json.len() {
            assert_rejected_without_restore(&json[..end]);
        }
    }
}

#[test]
fn malformed_import_never_reaches_restore_side_effects() {
    let mut wrong_type = valid_backup("2");
    wrong_type["snippets"][0]["expansion"] = serde_json::json!(123);
    assert_rejected_without_restore(&wrong_type.to_string());
    let mut missing_field = valid_backup("2");
    missing_field["contexts"][0]
        .as_object_mut()
        .unwrap()
        .remove("name");
    assert_rejected_without_restore(&missing_field.to_string());
    assert_rejected_without_restore(&(valid_backup("2").to_string() + " trailing garbage"));
    assert_rejected_without_restore(&valid_backup("future").to_string());
}

#[test]
fn supported_import_versions_reach_restore_after_complete_decode() {
    for version in ["1", "2"] {
        let applied = with_import_payload(&valid_backup(version).to_string(), |payload| {
            assert_eq!(payload.version, version);
            assert_eq!(payload.snippets[0].expansion, "new");
            Ok(true)
        })
        .unwrap();
        assert!(applied);
    }
}
