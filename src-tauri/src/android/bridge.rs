use serde_json::{json, Value};
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub struct InsertionFormat {
    pub contextual_caps: bool,
    pub auto_spacing: bool,
    pub profile: String,
    pub language: String,
    pub protected_initial_case: bool,
}

struct OutboxEntry {
    seq: u64,
    text: String,
    published_at: Instant,
    format: Option<InsertionFormat>,
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

pub fn publish_android_insertion(text: &str) -> u64 {
    publish_android_insertion_with(text, None)
}

pub fn publish_android_insertion_with(text: &str, format: Option<InsertionFormat>) -> u64 {
    let mut guard = outbox().lock().unwrap_or_else(|e| e.into_inner());
    let seq = guard.next_seq;
    guard.next_seq = guard.next_seq.wrapping_add(1).max(1);
    guard.current = Some(OutboxEntry {
        seq,
        text: text.to_string(),
        published_at: Instant::now(),
        format,
    });
    log::info!(
        "android bridge: published insertion seq={seq} chars={}",
        text.chars().count()
    );
    seq
}

pub(crate) fn format_pending_insertion(
    seq: u64,
    left: &str,
    right: &str,
    left_reliable: bool,
    right_reliable: bool,
) -> Option<String> {
    let guard = outbox().lock().unwrap_or_else(|e| e.into_inner());
    let entry = guard.current.as_ref().filter(|entry| entry.seq == seq)?;
    let Some(format) = entry.format.as_ref() else {
        return Some(entry.text.clone());
    };
    if !format.contextual_caps && !format.auto_spacing {
        return Some(entry.text.clone());
    }
    Some(crate::core::injection::format_for_caret_text(
        &entry.text,
        left,
        right,
        left_reliable,
        right_reliable,
        format.contextual_caps,
        format.auto_spacing,
        &format.profile,
        &format.language,
        format.protected_initial_case,
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AckOutcome {
    Cleared,
    KeptForRetry,
    Stale { expected: Option<u64> },
}

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

fn keystore_pending_cell() -> &'static Mutex<Option<(String, String)>> {
    static PENDING: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

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

pub(crate) fn clear_keystore_rotation() {
    *keystore_pending_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
pub(crate) fn clear_keystore_rotation_for_tests() {
    clear_keystore_rotation();
}

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

pub(crate) fn parse_head(head: &str, token: &str) -> Result<(String, String, usize, bool), String> {
    let request_line = head.lines().next().unwrap_or("").trim_end_matches('\r');
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let raw_path = parts.next().unwrap_or("").to_string();
    if method.is_empty() || raw_path.is_empty() {
        return Err("malformed request line".to_string());
    }
    let path = raw_path.split('?').next().unwrap_or("").to_string();
    if !path.starts_with("/v1/") || path.contains("..") {
        return Err("unknown path".to_string());
    }
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

fn hide_pill_offline(settings: &crate::data::store::SettingsSnapshot) -> bool {
    use crate::data::store;
    if !settings
        .get(store::ANDROID_PILL_HIDE_OFFLINE)
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        return false;
    }
    let cfg = store::load_pipeline_config(settings);
    let local_primary = store::parse_model_id(&cfg.transcription_default_model)
        .is_some_and(|(provider, _)| provider == store::LOCAL);
    let ready_local_fallback = cfg.transcription_fallback_models.iter().any(|id| {
        store::parse_model_id(id).is_some_and(|(provider, model)| {
            provider == store::LOCAL
                && crate::local_stt::model::manifest_by_id(&model).is_some_and(|manifest| {
                    manifest
                        .is_downloaded(&crate::local_stt::LocalTranscriptionManager::models_root())
                })
        })
    });
    !local_primary && !ready_local_fallback
}

fn pill_position_update(target: &str, value: &str) -> Result<&'static str, String> {
    use crate::data::store;
    let (key, allowed): (&'static str, &[&str]) = match target {
        "position" => (store::ANDROID_PILL_POSITION, &super::ANDROID_PILL_POSITIONS),
        "dock" => (
            store::ANDROID_PILL_DOCK_POSITION,
            &super::ANDROID_PILL_SCREEN_POSITIONS,
        ),
        _ => return Err("unknown pill target".to_string()),
    };
    if allowed.contains(&value) {
        Ok(key)
    } else {
        Err("unknown pill position".to_string())
    }
}

fn state_payload(state: &BridgeState) -> Value {
    let settings = state
        .app
        .as_ref()
        .and_then(|app| crate::data::store::settings_snapshot(app).ok());
    let (lifecycle, dictation_active) = match &state.app {
        Some(app) => app
            .try_state::<crate::pipeline::SharedState>()
            .and_then(|shared| {
                shared.lock().ok().map(|st| {
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
    let analytics_enabled = settings
        .as_ref()
        .map(analytics_enabled_value)
        .unwrap_or(true);
    let sync_muting_enabled = sync_muting_enabled_value(settings.as_ref());
    let pill_position = settings
        .as_ref()
        .and_then(|settings| {
            settings
                .get(crate::data::store::ANDROID_PILL_POSITION)
                .and_then(Value::as_str)
        })
        .filter(|value| super::ANDROID_PILL_POSITIONS.contains(value))
        .unwrap_or(super::DEFAULT_ANDROID_PILL_POSITION)
        .to_string();
    let pill_dock_position = settings
        .as_ref()
        .and_then(|settings| {
            settings
                .get(crate::data::store::ANDROID_PILL_DOCK_POSITION)
                .and_then(Value::as_str)
        })
        .filter(|value| super::ANDROID_PILL_SCREEN_POSITIONS.contains(value))
        .unwrap_or(super::DEFAULT_ANDROID_PILL_DOCK_POSITION)
        .to_string();
    let cover_keyboard_mic = settings
        .as_ref()
        .and_then(|settings| {
            settings
                .get(crate::data::store::ANDROID_PILL_COVER_KEYBOARD_MIC)
                .and_then(Value::as_bool)
        })
        .unwrap_or(true);
    let appearance_mode = settings
        .as_ref()
        .and_then(|settings| {
            settings
                .get(crate::data::store::APPEARANCE_MODE)
                .and_then(Value::as_str)
        })
        .filter(|value| matches!(*value, "system" | "light" | "dark"))
        .unwrap_or("system")
        .to_string();
    json!({
        "lifecycle": lifecycle,
        "dictationActive": dictation_active,
        "audioMuteRequested": crate::system::volume::android_mute_requested(),
        "syncMutingEnabled": sync_muting_enabled,
        "pillStage": last_pill_stage(),
        "audioLevel": last_audio_level(),
        "audioEnvelope": take_audio_envelope(),
        "keystorePending": has_keystore_rotation(),
        "analyticsEnabled": analytics_enabled,
        "pillPosition": pill_position,
        "pillDockPosition": pill_dock_position,
        "coverKeyboardMic": cover_keyboard_mic,
        "hidePillOffline": settings.as_ref().map(hide_pill_offline).unwrap_or(true),
        "appearanceMode": appearance_mode,
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

fn sync_muting_enabled_value(settings: Option<&crate::data::store::SettingsSnapshot>) -> bool {
    let enabled = |key| {
        settings
            .and_then(|snapshot| snapshot.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    enabled(crate::data::store::SYNC_ENABLED) && enabled(crate::data::store::SYNC_MUTING_ENABLED)
}

fn analytics_settings_from_snapshot(
    settings: &crate::data::store::SettingsSnapshot,
    context_group_count: i64,
) -> Value {
    let bool_value = |key: &str| settings.get(key).and_then(Value::as_bool);
    let category_value = |key: &str| {
        let value = settings.get(key).and_then(Value::as_str).unwrap_or("");
        let normalized =
            match key {
                crate::data::store::TRANSCRIPTION_PROVIDER
                | crate::data::store::CLEANUP_PROVIDER => match value {
                    "groq" | "openai" | "google" | "assemblyai" | "openrouter" | "xai"
                    | "local" => value,
                    _ => "unknown",
                },
                crate::data::store::CLEANUP_INTENSITY => match value {
                    "none" | "rules" | "light" | "medium" | "high" => value,
                    _ => "unknown",
                },
                crate::data::store::HISTORY_RETENTION => match value {
                    "7 days" | "30 days" | "90 days" | "Forever" => value,
                    _ => "unknown",
                },
                crate::data::store::LOCAL_MODEL_MEMORY_POLICY => match value {
                    "keep_loaded" | "unload_after_5m" | "unload_after_15m"
                    | "unload_immediately" => value,
                    _ => "unknown",
                },
                _ => "unknown",
            };
        Some(normalized)
    };
    let context_group_count = context_group_count.clamp(0, 200);
    let feature_breadth =
        crate::data::store::analytics_feature_breadth(settings, context_group_count);
    json!({
        "cleanup_enabled": bool_value(crate::data::store::CLEANUP_ENABLED),
        "voice_commands_enabled": bool_value(crate::data::store::VOICE_COMMANDS_ENABLED),
        "dual_transcription_enabled": bool_value(crate::data::store::DUAL_TRANSCRIPTION_ENABLED),
        "noise_reduction": bool_value(crate::data::store::NOISE_REDUCTION),
        "mute_audio": bool_value(crate::data::store::MUTE_AUDIO),
        "exclusive_mic": bool_value(crate::data::store::EXCLUSIVE_MIC),
        "pause_media": bool_value(crate::data::store::PAUSE_MEDIA_DURING_DICTATION),
        "sound_effects": bool_value(crate::data::store::PLAY_START_STOP_SOUNDS),
        "auto_learn_enabled": bool_value(crate::data::store::AUTO_LEARN_ENABLED),
        "contextual_formatting": bool_value(crate::data::store::CONTEXTUAL_FORMATTING),
        "contextual_caps": bool_value(crate::data::store::CONTEXTUAL_CAPS),
        "auto_spacing": bool_value(crate::data::store::AUTO_SPACING),
        "autostart_enabled": bool_value(crate::data::store::AUTOSTART_ENABLED),
        "transcription_provider": category_value(crate::data::store::TRANSCRIPTION_PROVIDER),
        "cleanup_provider": category_value(crate::data::store::CLEANUP_PROVIDER),
        "cleanup_intensity": category_value(crate::data::store::CLEANUP_INTENSITY),
        "history_retention": category_value(crate::data::store::HISTORY_RETENTION),
        "local_model_memory_policy": category_value(crate::data::store::LOCAL_MODEL_MEMORY_POLICY),
        "mic_mute_button_dictation": bool_value(crate::data::store::MIC_MUTE_BUTTON_DICTATION),
        "sync_enabled": bool_value(crate::data::store::SYNC_ENABLED),
        "context_group_count": context_group_count,
        "feature_breadth": feature_breadth,
    })
}

fn analytics_settings_payload(app: &AppHandle) -> Value {
    let Ok(settings) = crate::data::store::settings_snapshot(app) else {
        return json!({});
    };
    let context_group_count = app
        .try_state::<crate::DbHandle>()
        .and_then(|db| crate::data::db::count_user_contexts(db.inner()).ok())
        .unwrap_or(0);
    analytics_settings_from_snapshot(&settings, context_group_count)
}

fn analytics_enabled_value(settings: &crate::data::store::SettingsSnapshot) -> bool {
    settings
        .get(crate::data::store::ANALYTICS_ENABLED)
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn pill_stage_cell() -> &'static Mutex<String> {
    static STAGE: OnceLock<Mutex<String>> = OnceLock::new();
    STAGE.get_or_init(|| Mutex::new(String::new()))
}

pub(crate) fn note_pill_stage(stage: &str) {
    *pill_stage_cell().lock().unwrap_or_else(|e| e.into_inner()) = stage.to_string();
}

pub(crate) fn last_pill_stage() -> String {
    pill_stage_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}


/// What Kotlin reported when the recording started: the foreground package and,
/// in a browser, the page's host. The pipeline has no window handle on Android,
/// so Context resolution reads these instead. Hostname only, never a URL.
#[derive(Default, Clone)]
struct DictationTarget {
    package: String,
    domain: String,
}

fn dictation_target_cell() -> &'static Mutex<DictationTarget> {
    static CELL: OnceLock<Mutex<DictationTarget>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(DictationTarget::default()))
}

fn set_dictation_target(package: &str, domain: &str) {
    *dictation_target_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = DictationTarget {
        package: package.trim().to_ascii_lowercase(),
        domain: domain.trim().to_ascii_lowercase(),
    };
}

pub(crate) fn dictation_package() -> Option<String> {
    let target = dictation_target_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    (!target.package.is_empty()).then_some(target.package)
}

pub(crate) fn dictation_domain() -> Option<String> {
    let target = dictation_target_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    (!target.domain.is_empty()).then_some(target.domain)
}

struct BridgeError {
    message: String,
    at_ms: u64,
}

fn bridge_error_cell() -> &'static Mutex<Option<BridgeError>> {
    static CELL: OnceLock<Mutex<Option<BridgeError>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

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

fn audio_level_cell() -> &'static std::sync::atomic::AtomicU32 {
    static LEVEL: OnceLock<std::sync::atomic::AtomicU32> = OnceLock::new();
    LEVEL.get_or_init(|| std::sync::atomic::AtomicU32::new(0))
}

pub(crate) fn note_audio_level(level: f32) {
    audio_level_cell().store(level.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

fn audio_envelope_buffer() -> &'static Mutex<Vec<f32>> {
    static ENVELOPE: OnceLock<Mutex<Vec<f32>>> = OnceLock::new();
    ENVELOPE.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn note_audio_envelope(samples: &[f32]) {
    const MAX_BUFFERED: usize = 400;
    let mut buffer = audio_envelope_buffer()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    buffer.extend_from_slice(samples);
    if buffer.len() > MAX_BUFFERED {
        let excess = buffer.len() - MAX_BUFFERED;
        buffer.drain(..excess);
    }
}

pub(crate) fn take_audio_envelope() -> Vec<f32> {
    std::mem::take(
        &mut *audio_envelope_buffer()
            .lock()
            .unwrap_or_else(|e| e.into_inner()),
    )
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
            set_dictation_target(&package, &str_field("domain"));
            *state.package_hint.lock().unwrap_or_else(|e| e.into_inner()) = package;
            let Some(app) = state.app.clone() else {
                return err(
                    503,
                    "Service Unavailable",
                    "recording unavailable in this context",
                );
            };
            clear_bridge_error();
            note_pill_stage("");
            note_audio_level(0.0);
            let action = app.state::<crate::pipeline::SharedState>();
            match crate::commands::start_input_recording(app.clone(), action).await {
                Ok(()) => ok(json!({
                    "analyticsSettings": analytics_settings_payload(&app),
                })),
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
            match crate::commands::stop_handless_mode(app.clone(), action).await {
                Ok(()) => ok(json!({})),
                Err(e) => {
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
        ("POST", "/v1/insertion/format") => {
            let seq = body_json.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
            match format_pending_insertion(
                seq,
                &str_field("left"),
                &str_field("right"),
                bool_field("leftReliable"),
                bool_field("rightReliable"),
            ) {
                Some(text) => ok(json!({ "text": text })),
                None => err(409, "Conflict", "no pending insertion for that seq"),
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
        ("POST", "/v1/pill/position") => {
            let value = str_field("value");
            let key = match pill_position_update(&str_field("target"), &value) {
                Ok(key) => key,
                Err(message) => return err(400, "Bad Request", &message),
            };
            let Some(app) = state.app.clone() else {
                return err(503, "Service Unavailable", "backend not ready");
            };
            if let Err(message) =
                crate::commands::save_setting(app.clone(), key.to_string(), json!(value)).await
            {
                log::warn!("android bridge: pill position save failed: {message}");
                return err(500, "Internal Server Error", "could not save pill position");
            }
            let _ = app.emit(
                "verenu:android-pill-position-changed",
                json!({ "key": key, "value": value }),
            );
            ok(json!({}))
        }
        ("POST", "/v1/credential") => {
            let raw_provider = str_field("provider");
            let raw_key = str_field("key");
            let (provider, key) = match crate::android::credential_input(&raw_provider, &raw_key) {
                Ok(input) => input,
                Err(message) => return err(400, "Bad Request", &message),
            };
            log::info!(
                "android bridge: credential push provider={provider} key_len={}",
                key.len()
            );
            crate::android::provide_credential(&provider, &key);
            ok(json!({}))
        }
        ("POST", "/v1/credentials/clear") => {
            crate::android::clear_credentials();
            clear_keystore_rotation();
            log::info!("android bridge: cleared in-memory credentials");
            ok(json!({}))
        }
        ("GET", "/v1/keystore/pending") => {
            match take_keystore_rotation() {
                Some((provider, key)) => ok(json!({ "provider": provider, "key": key })),
                None => ok(json!({ "provider": Value::Null })),
            }
        }
        _ => err(404, "Not Found", "unknown path"),
    }
}

const UNAUTH_DELAY_MS: u64 = 150;
const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 16;

async fn serve_connection(
    state: std::sync::Arc<BridgeState>,
    mut stream: tokio::net::TcpStream,
) -> io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        tokio::time::sleep(std::time::Duration::from_millis(UNAUTH_DELAY_MS)).await;
        let resp = err(401, "Unauthorized", "unauthorized");
        stream.write_all(&resp).await?;
        return Ok(());
    }
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

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

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
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, &payload)?;
    }
    std::fs::rename(&tmp, connection_file_path())?;
    Ok(())
}

pub fn start_bridge(app: AppHandle) -> anyhow::Result<std::net::SocketAddr> {
    let token = mint_token();
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    write_connection_file(addr.port(), &token)?;
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
    use crate::data::store::SettingsSnapshot;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn offline_pill_policy_tracks_active_model_chains_and_override() {
        use crate::data::store;
        let check = |transcription, cleanup, cleanup_enabled, tf: Value, cf: Value, hide| {
            hide_pill_offline(&SettingsSnapshot::from_pairs([
                (
                    store::TRANSCRIPTION_DEFAULT_MODEL.into(),
                    json!(transcription),
                ),
                (store::CLEANUP_DEFAULT_MODEL.into(), json!(cleanup)),
                (store::CLEANUP_ENABLED.into(), json!(cleanup_enabled)),
                (store::TRANSCRIPTION_FALLBACK_MODELS.into(), tf),
                (store::CLEANUP_FALLBACK_MODELS.into(), cf),
                (store::ANDROID_PILL_HIDE_OFFLINE.into(), hide),
            ]))
        };
        assert!(hide_pill_offline(&SettingsSnapshot::from_pairs([])));
        assert!(!check(
            "local/moonshine-tiny",
            "local/qwen",
            true,
            json!([]),
            json!([]),
            Value::Null
        ));
        assert!(check(
            "groq/whisper",
            "local/qwen",
            true,
            json!([]),
            json!([]),
            json!(true)
        ));
        assert!(!check(
            "local/moonshine-tiny",
            "groq/llama",
            true,
            json!([]),
            json!([]),
            json!(true)
        ));
        assert!(!check(
            "local/moonshine-tiny",
            "groq/llama",
            false,
            json!([]),
            json!(["groq/llama"]),
            json!(true)
        ));
        assert!(!check(
            "local/moonshine-tiny",
            "local/qwen",
            false,
            json!(["groq/whisper"]),
            json!([]),
            json!(true)
        ));
        assert!(!check(
            "local/moonshine-tiny",
            "local/qwen",
            true,
            json!([]),
            json!(["groq/llama"]),
            json!(true)
        ));
        assert!(!check(
            "local/moonshine-tiny",
            "local/qwen",
            true,
            json!(["local/parakeet-v3"]),
            json!(["local/qwen-other"]),
            json!(true)
        ));
        assert!(!check(
            "custom-lan/speech",
            "custom-lan/cleanup",
            true,
            json!([]),
            json!([]),
            json!(false)
        ));
        let legacy = SettingsSnapshot::from_pairs([
            (
                store::TRANSCRIPTION_MODEL.into(),
                json!("local/moonshine-tiny"),
            ),
            (store::CLEANUP_MODEL.into(), json!("local/qwen")),
        ]);
        assert!(!hide_pill_offline(&legacy));
    }

    #[test]
    fn dictation_target_is_lowercased_and_replaced_each_recording() {
        let _guard = crate::android::test_serial();
        set_dictation_target(" com.Android.Chrome ", "Mail.Google.com");
        assert_eq!(dictation_package().as_deref(), Some("com.android.chrome"));
        assert_eq!(dictation_domain().as_deref(), Some("mail.google.com"));

        set_dictation_target("com.slack", "");
        assert_eq!(dictation_package().as_deref(), Some("com.slack"));
        assert_eq!(dictation_domain(), None);

        set_dictation_target("", "");
        assert_eq!(dictation_package(), None);
    }

    #[test]
    fn analytics_settings_are_allowlisted_and_normalized() {
        let settings = SettingsSnapshot::from_pairs([
            (crate::data::store::CLEANUP_ENABLED.to_string(), json!(true)),
            (
                crate::data::store::TRANSCRIPTION_PROVIDER.to_string(),
                json!("custom-model"),
            ),
            (
                crate::data::store::CLEANUP_INTENSITY.to_string(),
                json!("medium"),
            ),
            (
                crate::data::store::CLEANUP_PROMPT_OVERRIDE.to_string(),
                json!("fake transcript"),
            ),
            (
                crate::data::store::TRANSCRIPTION_MODEL.to_string(),
                json!("sk-test-api-key"),
            ),
            (
                crate::data::store::MICROPHONE_DEVICE.to_string(),
                json!("C:\\Users\\fake\\mic"),
            ),
        ]);
        let payload = analytics_settings_from_snapshot(&settings, 0);
        assert_eq!(payload["cleanup_enabled"], true);
        assert_eq!(payload["transcription_provider"], "unknown");
        assert_eq!(payload["cleanup_intensity"], "medium");
        let serialized = payload.to_string();
        assert!(!serialized.contains("fake transcript"));
        assert!(!serialized.contains("sk-test-api-key"));
        assert!(!serialized.contains("C:\\\\Users"));
        assert!(!serialized.contains("custom-model"));
    }

    #[test]
    fn analytics_defaults_on_and_preserves_an_explicit_opt_out() {
        assert!(analytics_enabled_value(&SettingsSnapshot::from_pairs([])));
        let disabled = SettingsSnapshot::from_pairs([(
            crate::data::store::ANALYTICS_ENABLED.to_string(),
            json!(false),
        )]);
        assert!(!analytics_enabled_value(&disabled));
    }

    #[test]
    fn android_fast_mute_poll_requires_sync_and_device_local_opt_in() {
        use crate::data::store;
        let settings = |sync, muting| {
            SettingsSnapshot::from_pairs([
                (store::SYNC_ENABLED.to_string(), json!(sync)),
                (store::SYNC_MUTING_ENABLED.to_string(), json!(muting)),
            ])
        };

        assert!(!sync_muting_enabled_value(None));
        assert!(!sync_muting_enabled_value(Some(&settings(true, false))));
        assert!(!sync_muting_enabled_value(Some(&settings(false, true))));
        assert!(sync_muting_enabled_value(Some(&settings(true, true))));
    }

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
        assert_eq!(ack_outbox(seq, false), AckOutcome::KeptForRetry);
        assert!(peek_outbox().is_some());
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
        let raw = "GET /v1/state HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer wrong\r\nConnection: close\r\n\r\n";
        let (status, body) = roundtrip(addr, raw).await;
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

    #[test]
    fn pill_position_updates_map_to_their_setting_and_reject_strangers() {
        use crate::data::store;
        assert_eq!(
            pill_position_update("position", "keyboard-left"),
            Ok(store::ANDROID_PILL_POSITION)
        );
        assert_eq!(
            pill_position_update("dock", "screen-bottom-right"),
            Ok(store::ANDROID_PILL_DOCK_POSITION)
        );
        assert!(pill_position_update("dock", "keyboard-left").is_err());
        assert!(pill_position_update("position", "somewhere").is_err());
        assert!(pill_position_update("cover", "screen-top").is_err());
        assert!(pill_position_update("", "").is_err());
    }

    #[tokio::test]
    async fn bridge_pill_position_validates_before_it_needs_an_app() {
        let (addr, token) = spawn_test_server().await;
        let (status, _) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/pill/position",
                r#"{"target":"dock","value":"keyboard-left"}"#,
            ),
        )
        .await;
        assert_eq!(status, 400);
        let (status, _) = roundtrip(
            addr,
            &authed(
                &token,
                "POST",
                "/v1/pill/position",
                r#"{"target":"dock","value":"screen-left"}"#,
            ),
        )
        .await;
        assert_eq!(status, 503);
    }

    #[tokio::test]
    async fn bridge_state_reports_pill_placement_defaults() {
        let (addr, token) = spawn_test_server().await;
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["pillPosition"], "keyboard-center");
        assert_eq!(state["pillDockPosition"], "screen-bottom");
        assert_eq!(state["coverKeyboardMic"], true);
    }

    #[tokio::test]
    async fn bridge_recording_routes_need_an_app() {
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
        let (_, empty) = roundtrip(addr, &authed(&token, "GET", "/v1/keystore/pending", "")).await;
        assert!(empty["provider"].is_null());
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["keystorePending"], false);

        stage_keystore_rotation("openai", "sk-test");
        let (_, state) = roundtrip(addr, &authed(&token, "GET", "/v1/state", "")).await;
        assert_eq!(state["keystorePending"], true);
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
        assert!(status == 400 || status == 404, "{status}");
        let (status, _) = roundtrip(
            addr,
            &authed(&token, "POST", "/v1/definitely-not-a-route", "{}"),
        )
        .await;
        assert_eq!(status, 404);
    }
}
