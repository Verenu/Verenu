//! Temporary, connection-scoped mute leases. Content sync never stores these.
use super::*;

const HEARTBEAT: Duration = Duration::from_secs(1);
const LEASE_TIMEOUT: Duration = Duration::from_secs(4);
const BUSY_RETRY_DELAY: Duration = Duration::from_secs(1);
const STATE_CHECK: Duration = Duration::from_millis(100);
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

fn enabled(app: &AppHandle) -> bool {
    store::settings_snapshot(app).is_ok_and(|settings| {
        settings.get(store::SYNC_ENABLED).and_then(|v| v.as_bool()) == Some(true)
            && settings
                .get(store::SYNC_MUTING_ENABLED)
                .and_then(|v| v.as_bool())
                == Some(true)
    })
}

fn recording_session(app: &AppHandle) -> Option<Arc<AtomicBool>> {
    app.try_state::<crate::pipeline::SharedState>()
        .and_then(|state| {
            let state = state.lock().ok()?;
            match &state.lifecycle {
                crate::pipeline::DictationLifecycle::Recording { session, .. } => {
                    Some(session.active.clone())
                }
                _ => None,
            }
        })
}

fn trusted(inner: &Inner, uuid: &str, fingerprint: &str) -> bool {
    peer_trusted(&inner.db, uuid, fingerprint)
}

fn sender_is_authorized(
    recording_is_active: impl Fn() -> bool,
    peer_is_trusted: impl Fn() -> bool,
) -> bool {
    recording_is_active() && peer_is_trusted()
}

fn peer_trusted(db: &DbHandle, uuid: &str, fingerprint: &str) -> bool {
    db.lock()
        .ok()
        .and_then(|conn| sync_store::get_peer(&conn, uuid).ok().flatten())
        .is_some_and(|peer| peer.cert_fp == fingerprint)
}

fn same_recording(app: &AppHandle, active: &Arc<AtomicBool>) -> bool {
    active.load(Ordering::Relaxed)
        && recording_session(app).is_some_and(|current| same_session(active, &current))
}

fn same_session(active: &Arc<AtomicBool>, current: &Arc<AtomicBool>) -> bool {
    active.load(Ordering::Relaxed) && Arc::ptr_eq(active, current)
}

fn claim_mute_session(
    sessions: &mut std::collections::HashMap<String, Arc<AtomicBool>>,
    uuid: &str,
    active: &Arc<AtomicBool>,
) -> bool {
    if sessions
        .get(uuid)
        .is_some_and(|current| Arc::ptr_eq(current, active))
    {
        return false;
    }
    sessions.insert(uuid.to_string(), active.clone());
    true
}

fn release_mute_session(
    sessions: &mut std::collections::HashMap<String, Arc<AtomicBool>>,
    uuid: &str,
    active: &Arc<AtomicBool>,
) -> bool {
    if sessions
        .get(uuid)
        .is_some_and(|current| Arc::ptr_eq(current, active))
    {
        sessions.remove(uuid);
        return true;
    }
    false
}

async fn wait_for_recording_to_end(is_active: impl Fn() -> bool) {
    while is_active() {
        tokio::time::sleep(STATE_CHECK).await;
    }
}

fn mute_ack_enables(message: &Message) -> bool {
    matches!(message, Message::DictationMuteAck { enabled: true, .. })
}

fn mute_ack_busy(message: &Message) -> bool {
    matches!(
        message,
        Message::DictationMuteAck {
            enabled: false,
            busy: true
        }
    )
}

async fn wait_for_busy_retry(is_active: impl Fn() -> bool, retry_delay: Duration) -> bool {
    let deadline = Instant::now() + retry_delay;
    loop {
        if !is_active() {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        tokio::time::sleep(remaining.min(STATE_CHECK)).await;
    }
}

enum CandidateAttempt<T> {
    ConnectFailed,
    PinMismatch,
    Pinned(T),
}

struct CandidateScan<T, A> {
    pinned: Option<T>,
    pin_mismatches: Vec<A>,
    saw_connect_failed: bool,
    had_candidates: bool,
    attempted_candidate: bool,
}

/// Try candidate routes in order. A wrong certificate rejects only that route,
/// so a stale saved route cannot prevent a later LAN address from matching.
/// Rejected routes are retained for the current recording and not retried.
async fn first_pinned_candidate<A, T, Candidates, Continue, Attempt, AttemptFuture>(
    candidates: Candidates,
    rejected: &std::collections::HashSet<A>,
    should_continue: Continue,
    mut attempt: Attempt,
) -> CandidateScan<T, A>
where
    Candidates: IntoIterator<Item = A>,
    A: Eq + std::hash::Hash + Clone,
    Continue: Fn() -> bool,
    Attempt: FnMut(A) -> AttemptFuture,
    AttemptFuture: std::future::Future<Output = CandidateAttempt<T>>,
{
    let mut scan = CandidateScan {
        pinned: None,
        pin_mismatches: Vec::new(),
        saw_connect_failed: false,
        had_candidates: false,
        attempted_candidate: false,
    };
    for candidate in candidates {
        scan.had_candidates = true;
        if rejected.contains(&candidate) {
            continue;
        }
        if !should_continue() {
            return scan;
        }
        scan.attempted_candidate = true;
        let address = candidate.clone();
        match attempt(candidate).await {
            CandidateAttempt::ConnectFailed => scan.saw_connect_failed = true,
            CandidateAttempt::PinMismatch => {
                log::debug!("sync: mute route did not match pinned peer certificate");
                scan.pin_mismatches.push(address);
            }
            CandidateAttempt::Pinned(connection) => {
                scan.pinned = Some(connection);
                return scan;
            }
        }
    }
    scan
}

/// Transfer an authenticated lease away from the general connection budget.
/// The lease pool stays bounded while ordinary sync and pairing slots reopen.
fn handoff_to_mute_lease(
    incoming: tokio::sync::OwnedSemaphorePermit,
    lease_slots: &Arc<Semaphore>,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    let lease = lease_slots.clone().try_acquire_owned().ok()?;
    drop(incoming);
    Some(lease)
}

impl SyncManager {
    pub(crate) fn monitor_muting(&self) {
        let Some(active) = recording_session(&self.inner.app) else {
            return;
        };
        if !enabled(&self.inner.app) {
            return;
        }
        for peer in conn_peers(&self.inner.db) {
            let uuid = peer.device_uuid;
            let mut sessions = self.inner.mute_sessions.lock().expect("mute sessions lock");
            if !claim_mute_session(&mut sessions, &uuid, &active) {
                continue;
            }
            drop(sessions);
            let manager = self.clone();
            let active = active.clone();
            tauri::async_runtime::spawn(async move {
                // A mute failure must never stop or delay microphone capture.
                let _ = manager.send_mute(&uuid, active.clone()).await;
                release_mute_session(
                    &mut manager
                        .inner
                        .mute_sessions
                        .lock()
                        .expect("mute sessions lock"),
                    &uuid,
                    &active,
                );
            });
        }
    }

    async fn send_mute(&self, uuid: &str, active: Arc<AtomicBool>) -> Result<()> {
        let peer = {
            let conn = self.lock_db()?;
            sync_store::get_peer(&conn, uuid)?.ok_or_else(|| anyhow!("not paired"))?
        };
        let (cert, key) = {
            let guard = self.inner.identity.read().expect("identity lock");
            let identity = guard.as_ref().ok_or_else(|| anyhow!("sync unavailable"))?;
            (identity.cert_der().clone(), identity.tls_key())
        };
        let connector = transport::tls_connector(transport::client_config(cert, key)?);
        let mut rejected_routes = std::collections::HashSet::new();
        loop {
            // Use the same saved-route and LAN fallback candidates as content sync.
            // A TLS pin mismatch rejects that address, not the remaining candidates.
            let addresses = self.addr_candidates_for_peer(uuid)?;
            let connector_for_candidates = connector.clone();
            let peer_uuid = uuid.to_string();
            let expected_fingerprint = peer.cert_fp.clone();
            let scan = first_pinned_candidate(
                addresses,
                &rejected_routes,
                || {
                    sender_is_authorized(
                        || enabled(&self.inner.app) && same_recording(&self.inner.app, &active),
                        || trusted(&self.inner, uuid, &peer.cert_fp),
                    )
                },
                move |addr| {
                    let connector = connector_for_candidates.clone();
                    let peer_uuid = peer_uuid.clone();
                    let expected_fingerprint = expected_fingerprint.clone();
                    async move {
                        let connect = async {
                            let tcp = tokio::net::TcpStream::connect(addr).await?;
                            let tls = connector
                                .connect(transport::server_name_for(&peer_uuid), tcp)
                                .await?;
                            let Some(certs) = tls.get_ref().1.peer_certificates() else {
                                return Ok::<_, anyhow::Error>(CandidateAttempt::PinMismatch);
                            };
                            let Ok(fingerprint) = transport::peer_fingerprint(certs) else {
                                return Ok(CandidateAttempt::PinMismatch);
                            };
                            if fingerprint != expected_fingerprint {
                                return Ok(CandidateAttempt::PinMismatch);
                            }
                            Ok(CandidateAttempt::Pinned(tls))
                        };
                        match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
                            Ok(Ok(result)) => result,
                            _ => CandidateAttempt::ConnectFailed,
                        }
                    }
                },
            )
            .await;
            let saw_pin_mismatch = !scan.pin_mismatches.is_empty();
            rejected_routes.extend(scan.pin_mismatches);
            let Some(mut tls) = scan.pinned else {
                if scan.saw_connect_failed
                    || saw_pin_mismatch
                    || (scan.had_candidates
                        && !scan.attempted_candidate
                        && !rejected_routes.is_empty())
                {
                    if wait_for_busy_retry(
                        || {
                            sender_is_authorized(
                                || {
                                    enabled(&self.inner.app)
                                        && same_recording(&self.inner.app, &active)
                                },
                                || trusted(&self.inner, uuid, &peer.cert_fp),
                            )
                        },
                        BUSY_RETRY_DELAY,
                    )
                    .await
                    {
                        continue;
                    }
                    return Ok(());
                }
                return Ok(());
            };
            if !sender_is_authorized(
                || enabled(&self.inner.app) && same_recording(&self.inner.app, &active),
                || trusted(&self.inner, uuid, &peer.cert_fp),
            ) {
                return Ok(());
            }
            send_message(
                &mut tls,
                &Message::DictationMute {
                    device_uuid: self.device_uuid(),
                },
            )
            .await?;
            let ack = tokio::time::timeout(CONNECT_TIMEOUT, read_message(&mut tls)).await;
            if matches!(ack.as_ref(), Ok(Ok(message)) if mute_ack_busy(message)) {
                if wait_for_busy_retry(
                    || {
                        sender_is_authorized(
                            || enabled(&self.inner.app) && same_recording(&self.inner.app, &active),
                            || trusted(&self.inner, uuid, &peer.cert_fp),
                        )
                    },
                    BUSY_RETRY_DELAY,
                )
                .await
                {
                    continue;
                }
                return Ok(());
            }
            if !matches!(ack.as_ref(), Ok(Ok(message)) if mute_ack_enables(message)) {
                // An explicit opt-out or an older/unsupported peer that
                // closes without a mute ACK both decline this recording. A
                // busy ACK uses a bounded retry window above.
                wait_for_recording_to_end(|| {
                    sender_is_authorized(
                        || enabled(&self.inner.app) && same_recording(&self.inner.app, &active),
                        || trusted(&self.inner, uuid, &peer.cert_fp),
                    )
                })
                .await;
                return Ok(());
            }
            let mut next_heartbeat = Instant::now();
            loop {
                if !enabled(&self.inner.app)
                    || !same_recording(&self.inner.app, &active)
                    || !trusted(&self.inner, uuid, &peer.cert_fp)
                {
                    let _ = send_message(&mut tls, &Message::DictationMuteEnd).await;
                    return Ok(());
                }
                if Instant::now() >= next_heartbeat {
                    tokio::time::timeout(
                        HEARTBEAT,
                        send_message(&mut tls, &Message::DictationMuteHeartbeat),
                    )
                    .await??;
                    next_heartbeat = Instant::now() + HEARTBEAT;
                }
                tokio::time::sleep(STATE_CHECK).await;
            }
        }
    }
}

struct RemoteMute(u64);
impl Drop for RemoteMute {
    fn drop(&mut self) {
        let owner = self.0;
        tauri::async_runtime::spawn_blocking(move || {
            crate::media::sound::set_remote_mute(owner, false)
        });
    }
}

pub(super) async fn receive(
    inner: Arc<Inner>,
    mut tls: TlsStream<tokio::net::TcpStream>,
    uuid: String,
    fingerprint: String,
    incoming_permit: tokio::sync::OwnedSemaphorePermit,
) {
    // Unknown devices and certificate mismatches cannot control system audio.
    if !trusted(&inner, &uuid, &fingerprint) {
        return;
    }
    let consent = enabled(&inner.app);
    if !consent {
        let _ = send_message(
            &mut tls,
            &Message::DictationMuteAck {
                enabled: false,
                busy: false,
            },
        )
        .await;
        return;
    }
    let Some(_lease_permit) = handoff_to_mute_lease(incoming_permit, &inner.mute_lease_slots)
    else {
        log::debug!("sync: active mute lease limit reached");
        let _ = send_message(
            &mut tls,
            &Message::DictationMuteAck {
                enabled: false,
                busy: true,
            },
        )
        .await;
        return;
    };
    if !trusted(&inner, &uuid, &fingerprint) || !enabled(&inner.app) {
        let _ = send_message(
            &mut tls,
            &Message::DictationMuteAck {
                enabled: false,
                busy: false,
            },
        )
        .await;
        return;
    }
    if send_message(
        &mut tls,
        &Message::DictationMuteAck {
            enabled: true,
            busy: false,
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let owner = RemoteMute(NEXT_OWNER.fetch_add(1, Ordering::Relaxed));
    let id = owner.0;
    if tauri::async_runtime::spawn_blocking(move || crate::media::sound::set_remote_mute(id, true))
        .await
        .is_err()
    {
        return;
    }
    let mut last_heartbeat = Instant::now();
    wait_for_release(
        &mut tls,
        || enabled(&inner.app) && trusted(&inner, &uuid, &fingerprint),
        &mut last_heartbeat,
        LEASE_TIMEOUT,
        STATE_CHECK,
    )
    .await;
}

async fn wait_for_release<R: tokio::io::AsyncRead + Unpin>(
    stream: &mut R,
    consent: impl Fn() -> bool,
    last_heartbeat: &mut Instant,
    lease_timeout: Duration,
    state_check: Duration,
) {
    loop {
        if !consent() || last_heartbeat.elapsed() >= lease_timeout {
            return;
        }
        // Keep the same read alive across state checks; cancelling partial frame
        // reads would desynchronize framing on a slow connection.
        let read = read_message(stream);
        tokio::pin!(read);
        loop {
            tokio::select! {
                message = &mut read => {
                    match message {
                        Ok(Message::DictationMuteHeartbeat) => *last_heartbeat = Instant::now(),
                        _ => return,
                    }
                    break;
                }
                _ = tokio::time::sleep(state_check) => {
                    if last_heartbeat.elapsed() >= lease_timeout || !consent() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn mute_sender_deduplicates_only_the_same_recording_and_stale_cleanup_is_safe() {
        let mut sessions = std::collections::HashMap::new();
        let first = Arc::new(AtomicBool::new(true));
        let successor = Arc::new(AtomicBool::new(true));

        assert!(claim_mute_session(&mut sessions, "peer", &first));
        assert!(!claim_mute_session(&mut sessions, "peer", &first));
        assert!(claim_mute_session(&mut sessions, "peer", &successor));
        assert!(Arc::ptr_eq(sessions.get("peer").unwrap(), &successor));

        assert!(!release_mute_session(&mut sessions, "peer", &first));
        assert!(Arc::ptr_eq(sessions.get("peer").unwrap(), &successor));
        assert!(release_mute_session(&mut sessions, "peer", &successor));
        assert!(!sessions.contains_key("peer"));
    }

    #[test]
    fn only_currently_paired_pinned_certificates_can_mute() {
        let db = crate::data::db::open(":memory:").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        assert!(!peer_trusted(&db, &uuid, "pinned"));
        sync_store::upsert_peer(&db.lock().unwrap(), &uuid, "Fixture peer", "pinned").unwrap();
        assert!(peer_trusted(&db, &uuid, "pinned"));
        assert!(!peer_trusted(&db, &uuid, "impostor"));
        assert!(!peer_trusted(
            &db,
            &uuid::Uuid::new_v4().to_string(),
            "pinned"
        ));
        sync_store::remove_peer(&db.lock().unwrap(), &uuid).unwrap();
        assert!(!peer_trusted(&db, &uuid, "pinned"));
    }

    #[tokio::test]
    async fn sender_stops_route_fallback_and_retry_after_peer_unpair() {
        let db = crate::data::db::open(":memory:").unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        sync_store::upsert_peer(&db.lock().unwrap(), &uuid, "Fixture peer", "pinned").unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let attempted = Arc::new(std::sync::Mutex::new(Vec::new()));

        let db_for_attempt = db.clone();
        let uuid_for_attempt = uuid.clone();
        let active_for_scan = active.clone();
        let db_for_scan = db.clone();
        let uuid_for_scan = uuid.clone();
        let scan = first_pinned_candidate(
            ["saved Tailscale", "discovered LAN"],
            &std::collections::HashSet::new(),
            move || {
                sender_is_authorized(
                    || active_for_scan.load(Ordering::Relaxed),
                    || peer_trusted(&db_for_scan, &uuid_for_scan, "pinned"),
                )
            },
            {
                let attempted = attempted.clone();
                move |route| {
                    let attempted = attempted.clone();
                    let db = db_for_attempt.clone();
                    let uuid = uuid_for_attempt.clone();
                    async move {
                        attempted.lock().unwrap().push(route);
                        sync_store::remove_peer(&db.lock().unwrap(), &uuid).unwrap();
                        CandidateAttempt::<()>::ConnectFailed
                    }
                }
            },
        )
        .await;

        assert!(scan.saw_connect_failed);
        assert_eq!(*attempted.lock().unwrap(), ["saved Tailscale"]);
        assert!(
            !wait_for_busy_retry(
                || sender_is_authorized(
                    || active.load(Ordering::Relaxed),
                    || peer_trusted(&db, &uuid, "pinned"),
                ),
                Duration::from_millis(20),
            )
            .await
        );
    }

    async fn lease<R: tokio::io::AsyncRead + Unpin>(stream: &mut R, consent: impl Fn() -> bool) {
        wait_for_release(
            stream,
            consent,
            &mut Instant::now(),
            Duration::from_millis(80),
            Duration::from_millis(5),
        )
        .await;
    }

    #[tokio::test]
    async fn explicit_opt_out_waits_until_the_recording_ends() {
        let recording = Arc::new(AtomicBool::new(true));
        let active = recording.clone();
        let task = tokio::spawn(async move {
            wait_for_recording_to_end(|| active.load(Ordering::Relaxed)).await;
        });

        tokio::time::sleep(STATE_CHECK * 2).await;
        assert!(
            !task.is_finished(),
            "a declined mute request must not be retried during this recording"
        );

        recording.store(false, Ordering::Relaxed);
        tokio::time::timeout(STATE_CHECK * 2, task)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn only_an_explicit_enabled_ack_allows_heartbeats() {
        assert!(mute_ack_enables(&Message::DictationMuteAck {
            enabled: true,
            busy: false,
        }));
        assert!(!mute_ack_enables(&Message::DictationMuteAck {
            enabled: false,
            busy: false,
        }));
        assert!(!mute_ack_enables(&Message::DictationMuteEnd));
    }

    #[test]
    fn busy_ack_retries_but_opt_out_and_legacy_ack_do_not() {
        let busy = Message::DictationMuteAck {
            enabled: false,
            busy: true,
        };
        let opted_out = Message::DictationMuteAck {
            enabled: false,
            busy: false,
        };
        let legacy: Message =
            serde_json::from_str(r#"{"type":"dictation_mute_ack","enabled":false}"#).unwrap();

        assert!(mute_ack_busy(&busy));
        assert!(!mute_ack_busy(&opted_out));
        assert!(!mute_ack_busy(&legacy));
    }

    #[tokio::test]
    async fn a_wrong_saved_route_pin_does_not_block_a_later_lan_candidate() {
        let attempted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let attempted_by_connector = attempted.clone();
        let scan = first_pinned_candidate(
            ["saved Tailscale", "discovered LAN"],
            &std::collections::HashSet::new(),
            || true,
            move |route| {
                let attempted = attempted_by_connector.clone();
                async move {
                    attempted.lock().unwrap().push(route);
                    if route == "saved Tailscale" {
                        CandidateAttempt::PinMismatch
                    } else {
                        CandidateAttempt::Pinned(route)
                    }
                }
            },
        )
        .await;

        assert_eq!(scan.pinned, Some("discovered LAN"));
        assert_eq!(scan.pin_mismatches, ["saved Tailscale"]);
        assert_eq!(
            *attempted.lock().unwrap(),
            ["saved Tailscale", "discovered LAN"]
        );
    }

    #[tokio::test]
    async fn a_wrong_saved_pin_is_skipped_while_a_transient_lan_route_retries() {
        let attempted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut rejected = std::collections::HashSet::new();

        let first_attempts = attempted.clone();
        let first = first_pinned_candidate(
            ["saved Tailscale", "discovered LAN"],
            &rejected,
            || true,
            move |route| {
                let attempts = first_attempts.clone();
                async move {
                    attempts.lock().unwrap().push(route);
                    if route == "saved Tailscale" {
                        CandidateAttempt::<()>::PinMismatch
                    } else {
                        CandidateAttempt::<()>::ConnectFailed
                    }
                }
            },
        )
        .await;
        rejected.extend(first.pin_mismatches);
        assert!(first.pinned.is_none());
        assert!(first.saw_connect_failed);
        assert_eq!(rejected, ["saved Tailscale"].into());

        let second_attempts = attempted.clone();
        let second = first_pinned_candidate(
            ["saved Tailscale", "discovered LAN"],
            &rejected,
            || true,
            move |route| {
                let attempts = second_attempts.clone();
                async move {
                    attempts.lock().unwrap().push(route);
                    CandidateAttempt::Pinned(route)
                }
            },
        )
        .await;

        assert_eq!(second.pinned, Some("discovered LAN"));
        assert_eq!(
            *attempted.lock().unwrap(),
            ["saved Tailscale", "discovered LAN", "discovered LAN"]
        );
    }

    #[tokio::test]
    async fn a_new_route_is_tried_after_known_pin_mismatches_are_filtered() {
        let mut rejected = std::collections::HashSet::new();
        let first = first_pinned_candidate(
            ["saved Tailscale"],
            &rejected,
            || true,
            |_| async { CandidateAttempt::<()>::PinMismatch },
        )
        .await;
        let should_retry = first.saw_connect_failed || !first.pin_mismatches.is_empty();
        rejected.extend(first.pin_mismatches);
        assert!(should_retry);

        let attempted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let attempted_by_connector = attempted.clone();
        let second = first_pinned_candidate(
            ["saved Tailscale", "new LAN candidate"],
            &rejected,
            || true,
            move |route| {
                let attempted = attempted_by_connector.clone();
                async move {
                    attempted.lock().unwrap().push(route);
                    CandidateAttempt::Pinned(route)
                }
            },
        )
        .await;

        assert_eq!(second.pinned, Some("new LAN candidate"));
        assert_eq!(*attempted.lock().unwrap(), ["new LAN candidate"]);
    }

    #[tokio::test]
    async fn busy_retry_is_delayed_and_stops_when_recording_ends() {
        let active = Arc::new(AtomicBool::new(true));
        let active_for_task = active.clone();
        let retry = tokio::spawn(async move {
            wait_for_busy_retry(
                || active_for_task.load(Ordering::Relaxed),
                Duration::from_millis(60),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        active.store(false, Ordering::Relaxed);
        assert!(!tokio::time::timeout(Duration::from_millis(150), retry)
            .await
            .unwrap()
            .unwrap());
        assert!(wait_for_busy_retry(|| true, Duration::from_millis(20)).await);
    }

    #[test]
    fn active_mute_leases_release_general_slots_and_stay_bounded() {
        let incoming_slots = Arc::new(Semaphore::new(MAX_INCOMING_CONNECTIONS));
        let lease_slots = Arc::new(Semaphore::new(MAX_ACTIVE_MUTE_LEASES));
        let mut leases = Vec::new();

        for _ in 0..MAX_ACTIVE_MUTE_LEASES {
            let incoming = incoming_slots
                .clone()
                .try_acquire_owned()
                .expect("general slot is available");
            leases.push(
                handoff_to_mute_lease(incoming, &lease_slots)
                    .expect("dedicated lease slot is available"),
            );
        }

        assert_eq!(incoming_slots.available_permits(), MAX_INCOMING_CONNECTIONS);
        assert_eq!(lease_slots.available_permits(), 0);
        let incoming = incoming_slots
            .clone()
            .try_acquire_owned()
            .expect("long leases must not consume general slots");
        assert!(
            handoff_to_mute_lease(incoming, &lease_slots).is_none(),
            "the dedicated active lease pool must remain bounded"
        );
        assert_eq!(incoming_slots.available_permits(), MAX_INCOMING_CONNECTIONS);

        drop(leases.pop());
        let incoming = incoming_slots
            .clone()
            .try_acquire_owned()
            .expect("general capacity remains available");
        assert!(handoff_to_mute_lease(incoming, &lease_slots).is_some());
    }

    #[tokio::test]
    async fn opt_out_wait_ends_when_muting_is_disabled() {
        let enabled = Arc::new(AtomicBool::new(true));
        let active = enabled.clone();
        let task = tokio::spawn(async move {
            wait_for_recording_to_end(|| active.load(Ordering::Relaxed)).await;
        });

        tokio::time::sleep(STATE_CHECK).await;
        enabled.store(false, Ordering::Relaxed);
        tokio::time::timeout(STATE_CHECK * 2, task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_new_recording_identity_can_retry_after_opt_out() {
        let declined_recording = Arc::new(AtomicBool::new(true));
        let current = Arc::new(std::sync::Mutex::new(declined_recording.clone()));
        let task_recording = declined_recording.clone();
        let task_current = current.clone();
        let task = tokio::spawn(async move {
            wait_for_recording_to_end(|| {
                let current = task_current.lock().unwrap();
                same_session(&task_recording, &current)
            })
            .await;
        });

        tokio::time::sleep(STATE_CHECK).await;
        let next_recording = Arc::new(AtomicBool::new(true));
        *current.lock().unwrap() = next_recording.clone();
        tokio::time::timeout(STATE_CHECK * 2, task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            same_session(&next_recording, &current.lock().unwrap()),
            "a new recording has its own active identity and can start a fresh request"
        );
    }

    #[tokio::test]
    async fn disconnect_and_explicit_stop_release_without_waiting_for_expiry() {
        for explicit in [false, true] {
            let (mut sender, mut receiver) = tokio::io::duplex(512);
            if explicit {
                send_message(&mut sender, &Message::DictationMuteEnd)
                    .await
                    .unwrap();
            } else {
                drop(sender);
            }
            tokio::time::timeout(Duration::from_millis(40), lease(&mut receiver, || true))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn stalled_connection_and_partial_frame_cannot_hold_mute_forever() {
        for partial in [false, true] {
            let (mut sender, mut receiver) = tokio::io::duplex(512);
            if partial {
                sender.write_all(&[0, 0]).await.unwrap();
            }
            tokio::time::timeout(Duration::from_millis(200), lease(&mut receiver, || true))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn heartbeats_renew_lease_but_revoking_consent_releases_it() {
        let (mut sender, mut receiver) = tokio::io::duplex(512);
        let consent = Arc::new(AtomicBool::new(true));
        let enabled = consent.clone();
        let task =
            tokio::spawn(
                async move { lease(&mut receiver, || enabled.load(Ordering::Relaxed)).await },
            );
        for _ in 0..5 {
            send_message(&mut sender, &Message::DictationMuteHeartbeat)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert!(!task.is_finished(), "heartbeats must keep the lease alive");
        }
        consent.store(false, Ordering::Relaxed);
        tokio::time::timeout(Duration::from_millis(40), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn state_checks_do_not_corrupt_a_slow_frame() {
        let (mut sender, mut receiver) = tokio::io::duplex(512);
        let body = serde_json::to_vec(&Message::DictationMuteHeartbeat).unwrap();
        let task = tokio::spawn(async move { lease(&mut receiver, || true).await });
        sender
            .write_all(&(body.len() as u32).to_be_bytes()[..2])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;
        sender
            .write_all(&(body.len() as u32).to_be_bytes()[2..])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;
        sender.write_all(&body).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            !task.is_finished(),
            "a completed slow heartbeat renews the lease"
        );
        send_message(&mut sender, &Message::DictationMuteEnd)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_millis(40), task)
            .await
            .unwrap()
            .unwrap();
    }
}
