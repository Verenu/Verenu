use super::*;

#[tokio::test]
#[ignore = "Requires an explicit private T3 pairing link file; makes a real metadata pull"]
async fn live_t3_pairing_and_catalog_pull() {
    let path = std::env::var("VERENU_T3_PAIRING_FILE").expect("Private pairing file required");
    let contents = std::fs::read_to_string(path).expect("Cannot read private pairing file");
    let link = contents
        .split_whitespace()
        .find(|word| word.starts_with("http") && word.contains("token="))
        .expect("Pairing link missing")
        .trim_end_matches(|c: char| c.is_control());
    // CLI pairing links may advertise LAN HTTP. The fixture explicitly uses
    // the caller's loopback endpoint so credentials never cross that transport.
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
    let (token, expiry) = exchange(&target).await.expect("T3 exchange failed");
    assert!(expiry > 0);
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
    // Report counts only, never labels, workspace paths, names, or credentials.
    println!(
        "Live T3 metadata pull: {} catalogs, {} skills",
        catalogs.len(),
        catalogs
            .iter()
            .map(|catalog| catalog.skills.len())
            .sum::<usize>()
    );
}
