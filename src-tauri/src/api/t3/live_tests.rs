use super::*;

#[tokio::test]
#[ignore = "Requires an explicit private T3 pairing link file; makes a real metadata pull"]
async fn live_t3_pairing_and_catalog_pull() {
    crate::legal_consent::enable_test_consent();
    let path = std::env::var("VERENU_T3_PAIRING_FILE").expect("Private pairing file required");
    let contents = std::fs::read_to_string(path).expect("Cannot read private pairing file");
    let link = contents
        .split_whitespace()
        .find(|word| word.starts_with("http") && word.contains("token="))
        .expect("Pairing link missing")
        .trim_end_matches(|c: char| c.is_control());
    let mut url = Url::parse(link).expect("Invalid private pairing link");
    if let Ok(base) = std::env::var("VERENU_T3_TEST_BASE") {
        let base = Url::parse(&base).expect("Invalid fixture base");
        assert!(base
            .host_str()
            .is_some_and(|host| host == "127.0.0.1" || host == "localhost"));
        url.set_scheme(base.scheme()).unwrap();
        url.set_host(base.host_str()).unwrap();
        url.set_port(base.port()).unwrap();
        let token = url.fragment().map(str::to_string);
        url.set_query(None);
        url.set_fragment(token.as_deref());
    }
    let target = parse_pairing_link(url.as_str()).expect("Pairing link could not be parsed");
    let description = descriptor(&target.base_url)
        .await
        .expect("T3 descriptor failed");
    let environment = description["environmentId"]
        .as_str()
        .expect("Missing environment ID");
    let granted = exchange(&target).await.expect("T3 exchange failed");
    let (token, expiry) = (granted.token, granted.expires_in);
    assert!(expiry > 0);
    assert!(granted.scopes.iter().any(|scope| scope == READ_SCOPE));
    let pulled = fetch_catalogs(&target.base_url, &token, environment, true).await;
    let catalogs = pulled.expect("T3 catalog pull failed");
    assert!(
        !catalogs.is_empty(),
        "No enabled provider catalogs returned"
    );
    assert!(
        catalogs.iter().any(|catalog| !catalog.skills.is_empty()),
        "No skills returned"
    );
    assert!(catalogs.iter().all(t3_skills::valid_catalog));
    println!(
        "Live T3 metadata pull: {} catalogs, {} skills",
        catalogs.len(),
        catalogs
            .iter()
            .map(|catalog| catalog.skills.len())
            .sum::<usize>()
    );
}

#[tokio::test]
#[ignore = "Requires an explicit private T3 pairing link file for a disposable T3; writes its shared dictionary"]
async fn live_t3_dictionary_sync_preserves_native_entries() {
    crate::legal_consent::enable_test_consent();
    use crate::system::t3_dictionary::Entry;
    let path = std::env::var("VERENU_T3_PAIRING_FILE").expect("Private pairing file required");
    let contents = std::fs::read_to_string(path).expect("Cannot read private pairing file");
    let link = contents
        .split_whitespace()
        .find(|word| word.starts_with("http") && word.contains("token="))
        .expect("Pairing link missing");
    let target = parse_pairing_link(link).expect("Pairing link could not be parsed");
    let granted = exchange(&target).await.expect("T3 exchange failed");
    assert!(
        granted.scopes.iter().any(|scope| scope == SETTINGS_SCOPE),
        "pairing link must allow settings changes: {:?}",
        granted.scopes
    );
    let base = &target.base_url;
    let read = || async {
        let mut socket = open_socket(base, &granted.token, "open failed").await.unwrap();
        let config = call(&mut socket, "1", "server.getConfig", json!({}), "read failed").await.unwrap();
        config.pointer("/settings/speechProjectCustomWords").cloned().unwrap()
    };
    // A native entry added in T3 itself.
    let mut socket = open_socket(base, &granted.token, "open failed").await.unwrap();
    call(&mut socket, "1", "server.updateSettings", json!({"patch":{"speechProjectCustomWords":[{"term":"NativeT3Term","aliases":["native alias"]}]}}), "seed failed").await.unwrap();
    let wanted = [
        Entry { term: "Verenu".into(), aliases: vec!["Vreenu".into(), "ver new".into()] },
        Entry { term: "Omarchy".into(), aliases: vec![] },
    ];
    let first = sync_dictionary(base, &granted.token, &[], &wanted).await.expect("first sync");
    assert_eq!(first.synced_terms, ["Verenu", "Omarchy"]);
    let after_first = read().await;
    assert_eq!(after_first.as_array().unwrap().len(), 3);
    assert_eq!(after_first[0]["term"], "NativeT3Term");
    assert_eq!(after_first[1]["aliases"], json!(["Vreenu", "ver new"]));
    // Removing Omarchy in Verenu removes only Omarchy.
    let second = sync_dictionary(base, &granted.token, &first.synced_terms, &wanted[..1]).await.expect("second sync");
    assert_eq!(second.synced_terms, ["Verenu"]);
    let after_second = read().await;
    let terms: Vec<_> = after_second.as_array().unwrap().iter().map(|entry| entry["term"].as_str().unwrap()).collect();
    assert_eq!(terms, ["NativeT3Term", "Verenu"]);
    println!("Live T3 dictionary sync verified: {terms:?}");
}
