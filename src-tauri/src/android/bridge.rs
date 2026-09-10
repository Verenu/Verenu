//! Token-authed loopback bridge between Rust and the Kotlin overlay service.
//!
//! The Android `AccessibilityService` + `TYPE_ACCESSIBILITY_OVERLAY` pill and
//! the `EncryptedSharedPreferences` Keystore are Kotlin-owned (see
//! `src-tauri/android/`), while recording, transcription, cleanup, history,
//! and sync stay in Rust. The two sides meet here, over `127.0.0.1`:
//!
//! * No new dependencies: Tokio TCP (`net` feature, already enabled) +
//!   `serde_json`. No JNI glue, no extra Tauri plugin crate, no WebView
//!   relay — the service works even when the main activity is dead.
//! * Authentication: a random 256-bit token minted per boot, stored in a
//!   `0600` file (`android_bridge.json`) inside the app-data dir that only
//!   this UID can read. Every request needs
//!   `Authorization: Bearer <token>`. Loopback HTTP is exempt from the
//!   cleartext ban, and the port binds `127.0.0.1` only.
//! * Privacy: the bridge NEVER logs dictated text, API keys, or the token —
//!   sequence numbers and lengths only.
//!
//! Protocol (JSON, one request per connection, `Connection: close`):
//!
//! | Method | Path | Body → Response |
//! | --- | --- | --- |
//! | GET | `/v1/state` | → `{ lifecycle, dictationActive, overlay, pendingInsertion }` |
//! | POST | `/v1/focus` | `{ keyboardVisible, hasEditableFocus }` → overlay decision |
//! | POST | `/v1/recording/start` | `{ package, hasEditableFocus, supportsSetText }` → `{ ok }` |
//! | POST | `/v1/recording/stop` | → `{ ok }` (pipeline continues through state/outbox) |
//! | POST | `/v1/recording/cancel` | → `{ ok }` |
//! | POST | `/v1/recording/retry` | → `{ ok }` |
//! | POST | `/v1/insertion/ack` | `{ seq, success, strategy, error?, package?, discard? }` → `{ ok }` |
//! | POST | `/v1/credential` | `{ provider, key }` → `{ ok }` (Keystore unlock push) |
//! | POST | `/v1/credentials/clear` | → `{ ok }` (drop Rust's in-memory cache) |
//! | GET | `/v1/keystore/pending` | → `{ provider, key }` once, then `{ provider: null }` |
//!
//! Keystore sync: the main app saves through the `android_keystore_save`
//! command (memory cache + staged rotation); Kotlin sees `keystorePending`
//! in `GET /v1/state`, fetches the rotation once, and persists it to
//! `EncryptedSharedPreferences`. Boot/unlock flows the other way via
//! `POST /v1/credential`. Rust never writes a secret to disk on Android.
//!
//! Insertion handoff: the pipeline deposits final text via
//! [`publish_android_insertion`] (called from the Android branch of
//! `core::injection::inject_text`); Kotlin polls `GET /v1/state`, inserts
//! through accessibility APIs (clipboard fallback where blocked), and acks.
//! History is already written by the pipeline before the handoff, so an ack
//! only drives pill/events/diagnostics — a lost ack can never lose text.

use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Insertion outbox (app-handle-free so the injection path can publish)
// ---------------------------------------------------------------------------

struct OutboxEntry {
    seq: u64,
    text: String,
    published_at: Instant,
}

struct Outbox {
    next_seq: u64,
    current: Option<OutboxEntry>,
}

fn outbox() -> &'static Mutex<Outbox> {
    static OUTBOX: OnceLock<Mutex<Outbox>> = OnceLock::new();
    OUTBOX.get_or_init(|| {
        Mutex::new(Outbox {
            next_seq: 1,
            current: None,
        })
    })
}

/// Deposit final dictated text for the Kotlin overlay service. Returns the
/// sequence number Kotlin must quote back in its ack. Overwrites any
/// unacknowledged entry — a new dictation supersedes the previous one, and
/// history already holds the old text so nothing is lost.
pub fn publish_android_insertion(text: &str) -> u64 {
    let mut guard = outbox().lock().unwrap_or_else(|e| e.into_inner());
    let seq = guard.next_seq;
    guard.next_seq = guard.next_seq.wrapping_add(1).max(1);
    guard.current = Some(OutboxEntry {
        seq,
        text: text.to_string(),
        published_at: Instant::now(),
    });
    log::info!(
        "android bridge: published insertion seq={seq} chars={}",
        text.chars().count()
    );
    seq
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AckOutcome {
    Cleared,
    KeptForRetry,
    Stale { expected: Option<u64> },
}

/// Acknowledge a handoff. Success clears the outbox; failure keeps the text
/// so the overlay can retry from `GET /v1/state` without re-dictating.
pub(crate) fn ack_outbox(seq: u64, success: bool) -> AckOutcome {
    let mut guard = outbox().lock().unwrap_or_else(|e| e.into_inner());
    match &guard.current {
        Some(entry) if entry.seq == seq => {
            if success {
                guard.current = None;
                AckOutcome::Cleared
            } else {
                AckOutcome::KeptForRetry
            }
        }
        other => AckOutcome::Stale {
            expected: other.as_ref().map(|e| e.seq),
        },
    }
}

pub(crate) struct OutboxSnapshot {
    pub seq: u64,
    pub text: String,
    pub age_ms: u64,
}

pub(crate) fn peek_outbox() -> Option<OutboxSnapshot> {
    let guard = outbox().lock().unwrap_or_else(|e| e.into_inner());
    guard.current.as_ref().map(|entry| OutboxSnapshot {
        seq: entry.seq,
        text: entry.text.clone(),
        age_ms: entry
            .published_at
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64,
    })
}

#[cfg(test)]
pub(crate) fn clear_outbox_for_tests() {
    outbox().lock().unwrap_or_else(|e| e.into_inner()).current = None;
}

// ---------------------------------------------------------------------------
// Keystore pending sync (Rust → Kotlin, single delivery)
// ---------------------------------------------------------------------------
//
// API keys are memory-only in Rust on Android; durable storage is the Kotlin
// `VerenuKeystore` (EncryptedSharedPreferences + AndroidKeyStore). The main
// app saves through the `android_keystore_save` command, which stashes the
// rotation here. Kotlin notices `keystore_pending` in `GET /v1/state`,
// fetches it once via `GET /v1/keystore/pending` (cleared on read — the
// secret is never written to disk by Rust), and persists it.

fn keystore_pending_cell() -> &'static Mutex<Option<(String, String)>> {
    static PENDING: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// Stash a Keystore rotation for single-delivery fetch. Empty `key` means
/// "delete this provider".
pub(crate) fn stage_keystore_rotation(provider: &str, key: &str) {
    *keystore_pending_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((provider.to_string(), key.to_string()));
    log::info!(
        "android bridge: staged keystore rotation provider={provider} key_len={}",
        key.len()
    );
}

pub(crate) fn take_keystore_rotation() -> Option<(String, String)> {
    keystore_pending_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

pub(crate) fn has_keystore_rotation() -> bool {
    keystore_pending_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

/// Drop a staged rotation as part of the lock/revocation fail-closed path.
/// The Accessibility service may be disconnected between staging and delivery;
/// leaving the secret in this process-global cell would defeat the cache clear.
pub(crate) fn clear_keystore_rotation() {
    *keystore_pending_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
pub(crate) fn clear_keystore_rotation_for_tests() {
    clear_keystore_rotation();
}

// ---------------------------------------------------------------------------
// Bridge state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct FocusReport {
    pub keyboard_visible: bool,
    pub has_editable_focus: bool,
}

pub(crate) struct BridgeState {
    pub token: String,
    pub app: Option<AppHandle>,
    pub focus: Mutex<FocusReport>,
    pub package_hint: Mutex<String>,
}

impl BridgeState {
    pub fn for_tests() -> Self {
        BridgeState {
            token: "test-token".to_string(),
            app: None,
            focus: Mutex::new(FocusReport::default()),
            package_hint: Mutex::new(String::new()),
        }
    }
}

/// Mint a 256-bit hex token. `rand` 0.8 is already a dependency (sync).
pub(crate) fn mint_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Minimal HTTP/1.1 handling (loopback only, one request per connection)
// ---------------------------------------------------------------------------

const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct HttpRequest {
    pub method: String,
    pub path: String,
    pub authorized: bool,
    pub body: Vec<u8>,
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    for line in head.lines().skip(1) {
        let line = line.trim_end_matches('\r');
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value.trim());
            }
        }
    }
    None
}

/// Constant-time bearer comparison. The token has 256 bits of entropy so
/// brute force over loopback is infeasible anyway; this just closes the
/// timing side-channel for free.
fn bearer_matches(presented: &str, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    let a = presented.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pure: split a raw head block into method/path/auth/content-length.
pub(crate) fn parse_head(head: &str, token: &str) -> Result<(String, String, usize, bool), String> {
    let request_line = head.lines().next().unwrap_or("").trim_end_matches('\r');
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let raw_path = parts.next().unwrap_or("").to_string();
    if method.is_empty() || raw_path.is_empty() {
        return Err("malformed request line".to_string());
    }
    // Strip query strings; reject path traversal attempts outright.
    let path = raw_path.split('?').next().unwrap_or("").to_string();
    if !path.starts_with("/v1/") || path.contains("..") {
        return Err("unknown path".to_string());
    }
    // We only implement Content-Length bodies; chunked requests are rejected
    // rather than half-parsed.
    if header_value(head, "transfer-encoding").is_some() {
        return Err("chunked requests are not supported".to_string());
    }
    let content_length = header_value(head, "content-length")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "bad content-length".to_string())?;
    if content_length > MAX_BODY_BYTES {
        return Err("body too large".to_string());
    }
    let authorized = header_value(head, "authorization").is_some_and(|v| bearer_matches(v, token));
    Ok((method, path, content_length, authorized))
}

fn http_response(status: u16, reason: &str, body: &Value) -> Vec<u8> {
    let payload = body.to_string();
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
    .into_bytes()
}

fn err(status: u16, reason: &str, message: &str) -> Vec<u8> {
    http_response(status, reason, &json!({ "ok": false, "error": message }))
}

fn ok(body: Value) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    map.insert("ok".to_string(), Value::Bool(true));
    if let Value::Object(extra) = body {
        map.extend(extra);
    }
    http_response(200, "OK", &Value::Object(map))
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

fn state_payload(state: &BridgeState) -> Value {
    let (lifecycle, dictation_active) = match &state.app {
        Some(app) => app
            .try_state::<crate::pipeline::SharedState>()
            .and_then(|shared| {
                shared.lock().ok().map(|st| {
                    // Coarse lifecycle for the overlay poller; fine-grained
                    // transcribe/clean/insert stages ride on `pillStage`
                    // below (see note_pill_stage).
                    let name = if st.lifecycle.is_idle() {
                        "idle"
                    } else if st.lifecycle.is_recording() {
                        "recording"
                    } else {
                        "processing"
                    };
                    (name.to_string(), !st.lifecycle.is_idle())
                })
            })
            .unwrap_or_else(|| ("unknown".to_string(), false)),
        None => ("unknown".to_string(), false),
    };
    let focus = state
        .focus
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let overlay = crate::commands::android_on_keyboard_visibility(
        focus.keyboard_visible,
        focus.has_editable_focus,
        None,
    );
    let pending = peek_outbox()
        .map(|snap| json!({ "seq": snap.seq, "text": snap.text, "ageMs": snap.age_ms }));
    let last_error =
        last_bridge_error().map(|(message, age_ms)| json!({ "message": message, "ageMs": age_ms }));
    json!({
        "lifecycle": lifecycle,
        "dictationActive": dictation_active,
        "pillStage": last_pill_stage(),
        "audioLevel": last_audio_level(),
        "keystorePending": has_keystore_rotation(),
        "lastError": last_error,
        "serverTimeUnixMs": now_unix_ms(),
        "overlay": {
            "state": overlay.state,
            "visible": overlay.visible,
            "dictationActive": overlay.dictation_active,
        },
        "targetPackage": state
            .package_hint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
        "pendingInsertion": pending,
    })
}

// ---------------------------------------------------------------------------
// Pill-stage mirror (fed by pipeline::pill::emit_pill_stage)
// ---------------------------------------------------------------------------

fn pill_stage_cell() -> &'static Mutex<String> {
    static STAGE: OnceLock<Mutex<String>> = OnceLock::new();
    STAGE.get_or_init(|| Mutex::new(String::new()))
}

/// Record the latest pipeline stage (`transcribing`/`cleaning`/`pasting`).
/// Called from `emit_pill_stage` on every platform; a single short-string
/// store, so desktop pays nothing measurable and the Android overlay poller
/// can render Transcribing → Cleaning → Inserting faithfully.
pub(crate) fn note_pill_stage(stage: &str) {
    *pill_stage_cell().lock().unwrap_or_else(|e| e.into_inner()) = stage.to_string();
}

pub(crate) fn last_pill_stage() -> String {
    pill_stage_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

// ---------------------------------------------------------------------------
// Error mirror (fed by pipeline::pill::show_error_pill)
// ---------------------------------------------------------------------------

struct BridgeError {
    message: String,
    at_ms: u64,
}

fn bridge_error_cell() -> &'static Mutex<Option<BridgeError>> {
    static CELL: OnceLock<Mutex<Option<BridgeError>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Record the latest user-facing pipeline error. Called from
/// `show_error_pill` on every platform (one short-string store); the payloads
/// there are already sanitized user-facing messages — never dictated text —
/// so mirroring them is safe. Lets the Android overlay render Retry instead
/// of going silently idle when the main activity is dead.
pub(crate) fn note_bridge_error(message: &str) {
    *bridge_error_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(BridgeError {
        message: message.chars().take(240).collect(),
        at_ms: now_unix_ms(),
    });
}

pub(crate) fn clear_bridge_error() {
    *bridge_error_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
}

pub(crate) fn last_bridge_error() -> Option<(String, u64)> {
    bridge_error_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|e| (e.message.clone(), now_unix_ms().saturating_sub(e.at_ms)))
}

// ---------------------------------------------------------------------------
// Audio-level mirror (fed by pipeline::session::spawn_level_emitter)
// ---------------------------------------------------------------------------

fn audio_level_cell() -> &'static std::sync::atomic::AtomicU32 {
    static LEVEL: OnceLock<std::sync::atomic::AtomicU32> = OnceLock::new();
    LEVEL.get_or_init(|| std::sync::atomic::AtomicU32::new(0))
}

/// Mirror the current mic level (0.0–1.0 display gain applied upstream).
/// Called every 50ms while recording; a single atomic store, so desktop pays
/// nothing measurable. The Android overlay poller renders its waveform from
/// this since it cannot see the `audio-level` WebView events.
pub(crate) fn note_audio_level(level: f32) {
    audio_level_cell().store(level.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn last_audio_level() -> f32 {
    f32::from_bits(audio_level_cell().load(std::sync::atomic::Ordering::Relaxed))
}

async fn handle_request(state: &BridgeState, req: HttpRequest) -> Vec<u8> {
    if !req.authorized {
        return err(401, "Unauthorized", "unauthorized");
    }
    let body_json: Value = if req.body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(_) => return err(400, "Bad Request", "malformed json"),
        }
    };
    let str_field = |name: &str| {
        body_json
            .get(name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let bool_field = |name: &str| {
        body_json
            .get(name)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/state") => ok(state_payload(state)),
        ("POST", "/v1/focus") => {
            let report = FocusReport {
                keyboard_visible: bool_field("keyboardVisible"),
                has_editable_focus: bool_field("hasEditableFocus"),
            };
            *state.focus.lock().unwrap_or_else(|e| e.into_inner()) = report.clone();
            let decision = crate::commands::android_on_keyboard_visibility(
                report.keyboard_visible,
                report.has_editable_focus,
                None,
            );
            ok(json!({
                "state": decision.state,
                "visible": decision.visible,
                "dictationActive": decision.dictation_active,
            }))
        }
        ("POST", "/v1/recording/start") => {
            let package = str_field("package");
            *state.package_hint.lock().unwrap_or_else(|e| e.into_inner()) = package;
            let Some(app) = state.app.clone() else {
                return err(
                    503,
                    "Service Unavailable",
                    "recording unavailable in this context",
                );
            };
            // A fresh dictation supersedes any previous terminal error and
            // stale visualizer stage from the prior session.
            clear_bridge_error();
            note_pill_stage("");
            note_audio_level(0.0);
            let action = app.state::<crate::pipeline::SharedState>();
            match crate::commands::start_input_recording(app.clone(), action).await {
                Ok(()) => ok(json!({})),
                Err(e) => err(500, "Internal Server Error", &e),
            }
        }
        ("POST", "/v1/recording/stop") => {
            let Some(app) = state.app.clone() else {
                return err(
                    503,
                    "Service Unavailable",
                    "recording unavailable in this context",
                );
            };
            let action = app.state::<crate::pipeline::SharedState>();
            // Android must use the shared full pipeline here. The desktop
            // `stop_and_transcribe_input` command is intentionally a
            // transcribe-only helper for the in-app mic button: it returns
            // the lifecycle to idle and hands the text back to its caller.
            // Using it for Android made the native pill reset immediately and
            // skipped cleanup, history, and the accessibility insertion
            // outbox. The full pipeline owns those stages and publishes the
            // final text through `inject_text` on Android.
            match crate::commands::stop_handless_mode(app.clone(), action).await {
                Ok(()) => ok(json!({})),
                Err(e) => {
                    // Mirror command-level failures too, so a native Android
                    // pill cannot remain on Transcribing without feedback.
                    note_bridge_error(&e);
                    err(500, "Internal Server Error", &e)
                }
            }
        }
        ("POST", "/v1/recording/cancel") => {
            let Some(app) = state.app.clone() else {
                return err(
                    503,
                    "Service Unavailable",
                    "recording unavailable in this context",
                );
            };
            let action = app.state::<crate::pipeline::SharedState>();
            match crate::commands::stop_recording(app.clone(), action).await {
                Ok(()) => ok(json!({})),
                Err(e) => err(500, "Internal Server Error", &e),
            }
        }
        ("POST", "/v1/recording/retry") => {
            let Some(app) = state.app.clone() else {
                return err(
                    503,
                    "Service Unavailable",
                    "recording unavailable in this context",
                );
            };
            let action = app.state::<crate::pipeline::SharedState>();
            clear_bridge_error();
            note_pill_stage("");
            note_audio_level(0.0);
            match crate::commands::retry_transcription(app.clone(), action).await {
                Ok(_) => ok(json!({})),
                Err(e) => err(500, "Internal Server Error", &e),
            }
        }
        ("POST", "/v1/insertion/ack") => {
            let seq = body_json.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
            let success = bool_field("success");
            let strategy = str_field("strategy");
            let error = str_field("error");
            let package = str_field("package");
            let discard = bool_field("discard");
            match ack_outbox(seq, success || discard) {
                AckOutcome::Stale { expected } => {
                    return err(
                        409,
                        "Conflict",
                        &format!(
                            "stale ack seq={seq}, expected={}",
                            expected
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "none".into())
                        ),
                    );
                }
                AckOutcome::Cleared | AckOutcome::KeptForRetry => {}
            }
            // Package + strategy are diagnostics only — never field contents.
            log::info!(
                "android bridge: insertion ack seq={seq} success={success} discard={discard} strategy={} package={} error={}",
                if strategy.is_empty() { "-" } else { &strategy },
                if package.is_empty() { "-" } else { &package },
                if error.is_empty() { "-" } else { &error },
            );
            if let Some(app) = state.app.clone() {
                let _ = app.emit(
                    "verenu:android-insertion-result",
                    json!({ "seq": seq, "success": success, "strategy": strategy, "package": package, "error": if error.is_empty() { Value::Null } else { Value::String(error) } }),
                );
                if success {
                    crate::pipeline::hide_pill(&app);
                }
            }
            ok(json!({}))
        }
        ("POST", "/v1/credential") => {
            let raw_provider = str_field("provider");
            let raw_key = str_field("key");
            let (provider, key) = match crate::android::credential_input(&raw_provider, &raw_key) {
                Ok(input) => input,
                Err(message) => return err(400, "Bad Request", &message),
            };
            // Length only in logs — never the secret itself.
            log::info!(
                "android bridge: credential push provider={provider} key_len={}",
                key.len()
            );
            crate::android::provide_credential(&provider, &key);
            ok(json!({}))
        }
        ("POST", "/v1/credentials/clear") => {
            // Lock/revocation path: clear both the active cache and any
            // not-yet-delivered Keystore rotation. Never leave a secret in
            // Rust memory just because the native service disconnected.
            crate::android::clear_credentials();
            clear_keystore_rotation();
            log::info!("android bridge: cleared in-memory credentials");
            ok(json!({}))
        }
        ("GET", "/v1/keystore/pending") => {
            match take_keystore_rotation() {
                // Single delivery: cleared on read, never persisted by Rust.
                Some((provider, key)) => ok(json!({ "provider": provider, "key": key })),
                None => ok(json!({ "provider": Value::Null })),
            }
        }
        _ => err(404, "Not Found", "unknown path"),
    }
}

/// Delay before answering an unauthenticated request. The token has 256
/// bits of entropy, so this is defense-in-depth that makes loopback probing
/// expensive for any other app that somehow reaches the port.
const UNAUTH_DELAY_MS: u64 = 150;
/// Bound for a single connection's lifetime; a client that stalls mid-head
/// or mid-body is dropped instead of holding a slot.
const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Cap on concurrent connections. The only client is the overlay service
/// (short poll connections); anything beyond this is a bug or a probe.
const MAX_CONNECTIONS: usize = 16;

async fn serve_connection(
    state: std::sync::Arc<BridgeState>,
    mut stream: tokio::net::TcpStream,
) -> io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Chunked reads (not byte-at-a-time): a poll connection's head arrives
    // in one or two packets, and junk past the bound is dropped, not stored.
    let mut head_buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_len = loop {
        if head_buf.len() >= MAX_HEAD_BYTES {
            let resp = err(413, "Payload Too Large", "head too large");
            stream.write_all(&resp).await?;
            return Ok(());
        }
        let take = chunk.len().min(MAX_HEAD_BYTES - head_buf.len());
        match stream.read(&mut chunk[..take]).await? {
            0 => return Ok(()),
            n => {
                head_buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_head_end(&head_buf) {
                    break pos;
                }
            }
        }
    };
    let head = String::from_utf8_lossy(&head_buf[..head_len]).into_owned();
    let (method, path, content_length, authorized) = match parse_head(&head, &state.token) {
        Ok(parsed) => parsed,
        Err(message) => {
            let resp = err(400, "Bad Request", &message);
            stream.write_all(&resp).await?;
            return Ok(());
        }
    };
    if !authorized {
        // Throttle first, and never spend body-read work on strangers.
        tokio::time::sleep(std::time::Duration::from_millis(UNAUTH_DELAY_MS)).await;
        let resp = err(401, "Unauthorized", "unauthorized");
        stream.write_all(&resp).await?;
        return Ok(());
    }
    // The header read may have already consumed some or all of the body in
    // the same TCP packet. Preserve those prefetched bytes before waiting
    // for the remainder; otherwise a normal small JSON request can deadlock
    // until the client timeout expires.
    let mut body = vec![0u8; content_length];
    let prefetched = &head_buf[head_len..];
    let copied = prefetched.len().min(content_length);
    body[..copied].copy_from_slice(&prefetched[..copied]);
    if copied < content_length {
        stream.read_exact(&mut body[copied..]).await?;
    }
    let req = HttpRequest {
        method,
        path,
        authorized,
        body,
    };
    let resp = handle_request(&state, req).await;
    stream.write_all(&resp).await?;
    Ok(())
}

/// Byte offset just past the `\r\n\r\n` terminator, if present.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

/// Path of the connection file Kotlin reads after boot. Same-UID private
/// storage; written `0600` on unix so no other app can steal the token.
pub fn connection_file_path() -> PathBuf {
    crate::app_data_dir().join("android_bridge.json")
}

fn write_connection_file(port: u16, token: &str) -> anyhow::Result<()> {
    let dir = crate::app_data_dir();
    std::fs::create_dir_all(&dir)?;
    let payload = serde_json::to_vec(
        &json!({ "port": port, "token": token, "updatedUnixMs": now_unix_ms() }),
    )?;
    let tmp = dir.join("android_bridge.json.tmp");
    // Atomic publish (write-temp + rename) so Kotlin never reads a torn
    // file; 0600 on unix so only this UID can steal the token.
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?
            .write_all(&payload)?;
        // Enforce 0600 even if the file already existed with wider perms.
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, &payload)?;
    }
    std::fs::rename(&tmp, connection_file_path())?;
    Ok(())
}

/// Start the bridge on 127.0.0.1 (ephemeral port), publish the connection
/// file, and spawn the accept loop. Called from `main.rs` setup on Android;
/// the protocol itself is platform-independent so tests exercise it here.
pub fn start_bridge(app: AppHandle) -> anyhow::Result<std::net::SocketAddr> {
    let token = mint_token();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    write_connection_file(addr.port(), &token)?;
    // Resolved path in the log (never the token): on Android this rests on
    // $HOME, so a surprising value here is the first sign of a storage
    // misconfiguration worth investigating before blaming the bridge.
    log::info!(
        "android bridge: listening on {addr}, connection file {}",
        connection_file_path().display()
    );
    let state = std::sync::Arc::new(BridgeState {
        token,
        app: Some(app),
        focus: Mutex::new(FocusReport::default()),
        package_hint: Mutex::new(String::new()),
    });
    tauri::async_runtime::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener);
        let listener = match listener {
            Ok(listener) => listener,
            Err(e) => {
                log::warn!("android bridge: failed to start accept loop: {e}");
                return;
            }
        };
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let permit = slots.clone().try_acquire_owned();
                    let state = state.clone();
                    tauri::async_runtime::spawn(async move {
                        // At capacity: fail fast instead of queueing strangers.
                        let Ok(_permit) = permit else {
                            use tokio::io::AsyncWriteExt as _;
                            let mut stream = stream;
                            let resp = err(503, "Service Unavailable", "busy");
                            let _ = stream.write_all(&resp).await;
                            return;
                        };
                        match tokio::time::timeout(
                            CONNECTION_TIMEOUT,
                            serve_connection(state, stream),
                        )
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                log::debug!("android bridge: connection error: {e}")
                            }
                            Err(_) => {
                                log::debug!("android bridge: connection timed out")
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("android bridge: accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Spin a test server (no AppHandle) on an ephemeral port for
    /// end-to-end protocol tests over real TCP.
    async fn spawn_test_server() -> (std::net::SocketAddr, String) {
        let token = mint_token();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = std::sync::Arc::new(BridgeState {
            token: token.clone(),
            app: None,
            focus: Mutex::new(FocusReport::default()),
            package_hint: Mutex::new(String::new()),
        });
        tauri::async_runtime::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = serve_connection(state, stream).await;
                });
            }
        });
        (addr, token)
    }

    async fn roundtrip(addr: std::net::SocketAddr, raw: &str) -> (u16, Value) {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status: u16 = text
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        let body = text.split("\r\n\r\n").nth(1).unwrap_or("{}");
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    fn authed(token: &str, method: &str, path: &str, body: &str) -> String {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn token_has_256_bits_of_entropy() {
        let a = mint_token();
        let b = mint_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_head_accepts_valid_requests() {
        let (method, path, len, auth) = parse_head(
            "POST /v1/focus HTTP/1.1\r\nAuthorization: Bearer abc\r\nContent-Length: 12\r\n\r\n",
            "abc",
        )
        .unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/v1/focus");
        assert_eq!(len, 12);
        assert!(auth);
    }

    #[test]
    fn parse_head_rejects_traversal_and_wrong_token() {
        assert!(parse_head("GET /v1/../secret HTTP/1.1\r\n\r\n", "abc").is_err());
        assert!(parse_head("GET /other HTTP/1.1\r\n\r\n", "abc").is_err());
        let (_, _, _, auth) = parse_head("GET /v1/state HTTP/1.1\r\n\r\n", "abc").unwrap();
        assert!(!auth);
    }

    #[test]
    fn bearer_matches_exact_token_only() {
        assert!(bearer_matches("Bearer abc123", "abc123"));
        assert!(!bearer_matches("Bearer abc123", "abc124"));
        assert!(!bearer_matches("Bearer abc123 ", "abc123"));
        assert!(!bearer_matches("bearer abc123", "abc123"));
        assert!(!bearer_matches("Bearer ", "abc123"));
        assert!(!bearer_matches("", ""));
    }

    #[test]
    fn parse_head_rejects_chunked_encoding() {
        assert!(parse_head(
            "POST /v1/focus HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n",
            "abc"
        )
        .is_err());
    }

    #[test]
    fn find_head_end_locates_terminator() {
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(18));
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(find_head_end(b""), None);
    }

    #[test]
    fn bridge_error_mirror_round_trips() {
        let _serial = crate::android::test_serial();
        clear_bridge_error();
        assert!(last_bridge_error().is_none());
        note_bridge_error("No speech detected — try again");
        let (message, _) = last_bridge_error().unwrap();
        assert_eq!(message, "No speech detected — try again");
        // Long messages are truncated, never balloon the poll payload.
        note_bridge_error(&"x".repeat(1000));
        assert!(last_bridge_error().unwrap().0.chars().count() <= 240);
        clear_bridge_error();
        assert!(last_bridge_error().is_none());
    }

    #[test]
    fn outbox_publish_ack_lifecycle() {
        let _serial = crate::android::test_serial();
        clear_outbox_for_tests();
        let seq = publish_android_insertion("hello");
        let snap = peek_outbox().unwrap();
        assert_eq!(snap.seq, seq);
        assert_eq!(snap.text, "hello");
        // Failed delivery keeps the text for retry.
        assert_eq!(ack_outbox(seq, false), AckOutcome::KeptForRetry);
        assert!(peek_outbox().is_some());
        // Stale acks never clear a newer handoff.
        let seq2 = publish_android_insertion("world");
        assert_eq!(
            ack_outbox(seq, true),
            AckOutcome::Stale {
                expected: Some(seq2)
            }
        );
        assert_eq!(ack_outbox(seq2, true), AckOutcome::Cleared);
        assert!(peek_outbox().is_none());
        assert_eq!(ack_outbox(seq2, true), AckOutcome::Stale { expected: None });
    }

    #[tokio::test]
    async fn bridge_rejects_unauthenticated_requests() {
        let (addr, token) = spawn_test_server().await;
        let raw = format!(
            "GET /v1/state HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer wrong\r\nConnection: close\r\n\r\n"
        );
        let (status, body) = roundtrip(addr, &raw).await;
        assert_eq!(status, 401);
        assert_eq!(body["ok"], false);
        let _ = token;
    }

    #[tokio::test]
    async fn bridge_state_reports_no_pending_insertion_initially() {
        let _serial = crate::android::test_serial();
        clear_outbox_for_tests();
        let (addr, token) = spawn_test_server().await;
        let (status, body) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert!(body["pendingInsertion"].is_null());
    }

    #[tokio::test]
    async fn bridge_accepts_body_prefetched_with_headers() {
        let (addr, token) = spawn_test_server().await;
        let raw = authed(
            &token,
            "POST",
            "/v1/focus",
            r#"{"keyboardVisible":true,"hasEditableFocus":true}"#,
        );
        let (status, body) =
            tokio::time::timeout(std::time::Duration::from_secs(2), roundtrip(addr, &raw))
                .await
                .expect("bridge request with a prefetched body timed out");
        assert_eq!(status, 200);
        assert_eq!(body["visible"], true);
    }

    #[tokio::test]
    async fn bridge_focus_drives_overlay_decision() {
        let (addr, token) = spawn_test_server().await;
        // Keyboard open over an editable field → visible.
        let (status, body) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/focus",
                r#"{"keyboardVisible":true,"hasEditableFocus":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["visible"], true);
        // Keyboard closed → hidden, and GET /state agrees (no permanent bubble).
        let (status, body) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/focus",
                r#"{"keyboardVisible":false,"hasEditableFocus":false}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["visible"], false);
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["overlay"]["visible"], false);
    }

    #[tokio::test]
    async fn bridge_insertion_handoff_round_trips() {
        let _serial = crate::android::test_serial();
        clear_outbox_for_tests();
        let (addr, token) = spawn_test_server().await;
        let seq = publish_android_insertion("dictated text");
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["pendingInsertion"]["seq"], seq);
        assert_eq!(state["pendingInsertion"]["text"], "dictated text");
        // Ack with the wrong seq is a conflict, never a silent clear.
        let (status, _) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/insertion/ack",
                r#"{"seq":9999,"success":true}"#,
            ),
        )
        .await;
        assert_eq!(status, 409);
        let body = format!(
            r#"{{"seq":{seq},"success":true,"strategy":"direct_accessibility","package":"com.slack"}}"#
        );
        let (status, ack) =
            roundtrip(addr, &authed(&token, "POST", "/v1/insertion/ack", &body)).await;
        assert_eq!(status, 200);
        assert_eq!(ack["ok"], true);
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert!(state["pendingInsertion"].is_null());
    }

    #[tokio::test]
    async fn bridge_recording_routes_need_an_app() {
        // No AppHandle in tests → honest 503, not a panic.
        let (addr, token) = spawn_test_server().await;
        for path in [
            "/v1/recording/start",
            "/v1/recording/stop",
            "/v1/recording/cancel",
        ] {
            let (status, _) = roundtrip(addr, &authed(&token, "POST", path, "{}")).await;
            assert_eq!(status, 503, "{path}");
        }
    }

    #[tokio::test]
    async fn bridge_credential_push_lands_in_memory_cache() {
        let _serial = crate::android::test_serial();
        crate::android::clear_credentials();
        clear_keystore_rotation_for_tests();
        let (addr, token) = spawn_test_server().await;
        let (status, _) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/credential",
                r#"{"provider":"groq","key":"gsk-x"}"#,
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(crate::android::cached_credential("groq"), "gsk-x");

        let (status, body) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/credential",
                r#"{"provider":"unknown","key":"should-not-cache"}"#,
            ),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(body["ok"], false);
        assert_eq!(crate::android::cached_credential("unknown"), "");

        let oversized = format!(
            r#"{{"provider":"groq","key":"{}"}}"#,
            "x".repeat(crate::android::MAX_CREDENTIAL_BYTES + 1)
        );
        let (status, body) =
            roundtrip(addr, &authed(&token, "POST", "/v1/credential", &oversized)).await;
        assert_eq!(status, 400);
        assert_eq!(body["ok"], false);

        // Lock/revocation clears both the active cache and a staged rotation;
        // neither secret may survive the authenticated native request.
        stage_keystore_rotation("openai", "sk-staged");
        let (status, body) =
            roundtrip(addr, &authed(&token, "POST", "/v1/credentials/clear", "{}")).await;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(crate::android::cached_credential("groq"), "");
        assert!(!has_keystore_rotation());
        crate::android::clear_credentials();
    }

    #[tokio::test]
    async fn bridge_keystore_rotation_delivers_once() {
        let _serial = crate::android::test_serial();
        clear_keystore_rotation_for_tests();
        let (addr, token) = spawn_test_server().await;
        // Nothing staged → null provider, and state agrees.
        let (_, empty) = roundtrip(addr, &authed(&token, "GET", "/v1/keystore/pending", "")).await;
        assert!(empty["provider"].is_null());
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["keystorePending"], false);

        stage_keystore_rotation("openai", "sk-test");
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["keystorePending"], true);
        // Single delivery: first fetch carries the rotation, second is null.
        let (_, first) = roundtrip(addr, &authed(&token, "GET", "/v1/keystore/pending", "")).await;
        assert_eq!(first["provider"], "openai");
        assert_eq!(first["key"], "sk-test");
        let (_, second) = roundtrip(addr, &authed(&token, "GET", "/v1/keystore/pending", "")).await;
        assert!(second["provider"].is_null());
        clear_keystore_rotation_for_tests();
    }

    #[tokio::test]
    async fn bridge_state_carries_audio_level_and_stage() {
        let _serial = crate::android::test_serial();
        note_audio_level(0.5);
        note_pill_stage("cleaning");
        note_bridge_error("stale test error");
        let (addr, token) = spawn_test_server().await;
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["audioLevel"], 0.5);
        assert_eq!(state["pillStage"], "cleaning");
        assert_eq!(state["lastError"]["message"], "stale test error");
        note_audio_level(0.0);
        note_pill_stage("");
        clear_bridge_error();
    }

    #[tokio::test]
    async fn bridge_rejects_malformed_and_unknown() {
        let (addr, token) = spawn_test_server().await;
        let (status, _) =
            roundtrip(addr, &authed(&token, "POST", "/v1/focus", "not-json{{{")).await;
        assert_eq!(status, 400);
        let (status, _) = roundtrip(addr, &authed(&token, "GET", "/v1/nope", "")).await;
        // Path routing happens before authZ detail: unknown non-/v1/ paths are 400,
        // unknown /v1/ paths are 404.
        assert!(status == 400 || status == 404, "{status}");
        let (status, _) = roundtrip(
            addr,
            &authed(&token, "POST", "/v1/definitely-not-a-route", "{}"),
        )
        .await;
        assert_eq!(status, 404);
    }
}
