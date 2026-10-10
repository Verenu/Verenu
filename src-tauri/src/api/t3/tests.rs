use super::*;

#[tokio::test]
async fn t3_skill_pull_negotiates_protocol_two_and_uses_ticket_rpc() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let byte = stream.read_u8().await.unwrap();
            bytes.push(byte);
            assert!(bytes.len() < 8192);
        }
        let request = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
        assert!(request.starts_with("post /api/auth/websocket-ticket "));
        assert!(request.contains("authorization: bearer synthetic-credential"));
        let body = r#"{"ticket":"synthetic-ticket","expiresAt":"2099-01-01T00:00:00Z"}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        drop(stream);
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert!(request
                    .uri()
                    .to_string()
                    .contains("orchestrationProtocol=2"));
                assert!(request
                    .uri()
                    .to_string()
                    .contains("wsTicket=synthetic-ticket"));
                assert!(!request.uri().to_string().contains("synthetic-credential"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        for method in ["server.refreshProviders", "server.getConfig"] {
            let request: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(request["tag"], method);
            assert_eq!(request["_tag"], "Request");
            let value = if method == "server.getConfig" {
                json!({"cwd":"/synthetic", "providers":[{"instanceId":"codex", "enabled":true, "skills":[{"name":"babysit-pr", "enabled":true}]}]})
            } else {
                json!(null)
            };
            socket.send(Message::Text(json!({"_tag":"Exit", "requestId":request["id"], "exit":{"_tag":"Success", "value":value}}).to_string().into())).await.unwrap();
        }
    });
    let catalogs = fetch_catalogs(&base, "synthetic-credential", "synthetic-env", true)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(catalogs[0].skills[0].name, "babysit-pr");
}

#[test]
fn pairing_supports_direct_hosted_and_websocket_links_without_retaining_secrets() {
    for link in [
        "https://example.test/pair#token=synthetic",
        "https://example.test/pair?token=synthetic",
        "https://app.t3.codes/pair?host=%2F%2Fexample.test#token=synthetic",
        "wss://example.test/pair#token=synthetic",
    ] {
        let target = parse_pairing_link(link).unwrap();
        assert_eq!(target.base_url.as_str(), "https://example.test/");
        assert_eq!(target.credential, "synthetic");
        assert!(target.base_url.query().is_none());
        assert!(target.base_url.fragment().is_none());
    }
    assert!(parse_pairing_link("http://127.0.0.1:1234/pair#token=synthetic").is_ok());
    for link in [
        "http://192.168.1.25:3773/pair#token=synthetic",
        "http://10.0.0.25:3773/pair#token=synthetic",
        "http://172.16.0.25:3773/pair#token=synthetic",
        "http://169.254.1.25:3773/pair#token=synthetic",
        "http://[::1]:3773/pair#token=synthetic",
        "http://[fd12::25]:3773/pair#token=synthetic",
        "ws://192.168.1.25:3773/pair#token=synthetic",
        "https://app.t3.codes/pair?host=http%3A%2F%2F192.168.1.25%3A3773#token=synthetic",
    ] {
        let target = parse_pairing_link(link).unwrap();
        assert_eq!(target.base_url.scheme(), "http");
        assert_eq!(target.credential, "synthetic");
        assert!(target.base_url.query().is_none());
        assert!(target.base_url.fragment().is_none());
    }
    for link in [
        "https://example.test/pair",
        "ftp://example.test/#token=x",
        "http://remote.test/#token=x",
        "http://8.8.8.8/#token=x",
        "http://[2001:db8::25]/#token=x",
        "https://user:password@example.test/#token=x",
    ] {
        assert!(parse_pairing_link(link).is_err());
    }
}

#[test]
fn provider_catalogs_follow_workspace_precedence_and_hide_noninvocable_skills() {
    let config = json!({"cwd":"/synthetic", "providers":[{
        "instanceId":"codex", "enabled":true, "skills":[{"name":"default-only","enabled":true}],
        "workspaceSnapshots":[{"cwd":"/synthetic", "skills":[
            {"name":"babysit-pr","enabled":true,"description":"Monitor PR","path":"/private/path/SKILL.md"},
            {"name":"disabled","enabled":false}, {"name":"agent-only","enabled":true,"userInvocable":false}
        ]}]
    }]});
    let catalogs = parse_catalogs(&config, "env").unwrap();
    assert_eq!(catalogs.len(), 1);
    assert_eq!(catalogs[0].skills.len(), 1);
    assert_eq!(catalogs[0].skills[0].name, "babysit-pr");
    assert!(!serde_json::to_string(&catalogs)
        .unwrap()
        .contains("private/path"));
    let mut changed = config.clone();
    changed["providers"][0]["workspaceSnapshots"][0]["skills"][0]["description"] = json!("Updated");
    assert_ne!(
        catalogs[0].revision,
        parse_catalogs(&changed, "env").unwrap()[0].revision
    );
}

#[test]
fn incomplete_same_cwd_workspace_snapshot_does_not_hide_default_catalog() {
    let config = json!({"cwd":"/synthetic", "providers":[{
        "instanceId":"codex", "enabled":true,
        "skills":[{"name":"default-only","enabled":true}],
        "workspaceSnapshots":[{"cwd":"/synthetic"}]
    }]});
    let catalogs = parse_catalogs(&config, "env").unwrap();
    assert_eq!(catalogs.len(), 1);
    assert_eq!(catalogs[0].skills.len(), 1);
    assert_eq!(catalogs[0].skills[0].name, "default-only");
}
