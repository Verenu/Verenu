//! Two virtual devices on one host, using real TCP, TLS, pairing, and SQLite.
//! All identities and content are disposable fixtures. No credential store,
//! discovery broadcast, app database, microphone, or provider is touched.
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

impl Device {
    fn new() -> Self {
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
                    &protocol::Message::PairRequest {
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
                        &protocol::Message::PairReject {
                            reason: "code mismatch".into(),
                        },
                    )
                    .await;
                }
                let remote = result?;
                if !save_trust {
                    protocol::send_message(
                        &mut server,
                        &protocol::Message::Error {
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
                protocol::send_message(&mut server, &protocol::Message::PairComplete).await?;
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

/// Compare stored content and relationships, excluding device-local SQLite
/// ids. This catches missing assignments even when row counts happen to match.
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
    // More rows than a protocol batch exercises pagination and acknowledgements.
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
                    &protocol::Message::HelloAck(protocol::Hello {
                        device_uuid: b.host.uuid.clone(),
                        device_name: "Fixture B".into(),
                        protocol: protocol::PROTOCOL_VERSION,
                        app_version: "test".into(),
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
                    &protocol::Message::Meta {
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
                    &protocol::Message::Ops(protocol::OpsBatch {
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
                // Drop a real encrypted connection after one committed batch.
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
    // C still has A's old counters. Its gossip must not overwrite B's newer
    // knowledge when B and C exchange metadata in both directions.
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
    // D goes offline with a complete but now stale snapshot. The other three
    // receive a deletion while D makes a new independent edit and reopens DB.
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
