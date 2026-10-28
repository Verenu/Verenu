//! Opt-in local script API. Native command handlers remain the write boundary.
use crate::{
    commands,
    data::{db, store},
};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub(crate) struct Agent {
    app: AppHandle,
    approvals: Arc<Mutex<Approvals>>,
    gate: Arc<tokio::sync::Semaphore>,
}

#[derive(Default)]
struct Approvals {
    pending: Option<Pending>,
    grants: HashMap<[u8; 32], Instant>,
    legacy_token: Option<String>,
}

struct Pending {
    digest: [u8; 32],
    code: String,
    expires: Instant,
}

#[derive(Serialize)]
pub struct ApprovalRequest {
    code: String,
    id: String,
}

fn request_id(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[tauri::command]
pub fn agent_pending_request(app: AppHandle) -> Option<ApprovalRequest> {
    let state = app.try_state::<Arc<Mutex<Approvals>>>()?;
    let mut approvals = state.lock().ok()?;
    if approvals
        .pending
        .as_ref()
        .is_some_and(|p| p.expires <= Instant::now())
    {
        approvals.pending = None;
    }
    approvals.pending.as_ref().map(|p| ApprovalRequest {
        code: p.code.clone(),
        id: request_id(&p.digest),
    })
}

#[tauri::command]
pub fn agent_respond(app: AppHandle, id: String, approve: bool) -> Result<(), String> {
    let state = app
        .try_state::<Arc<Mutex<Approvals>>>()
        .ok_or("Agent access unavailable")?;
    let mut approvals = state.lock().map_err(|_| "Agent access unavailable")?;
    let pending = approvals.pending.as_ref().ok_or("Request expired")?;
    if pending.expires <= Instant::now() || request_id(&pending.digest) != id {
        return Err("Request expired or changed".into());
    }
    let pending = approvals.pending.take().ok_or("Request expired")?;
    if approve {
        approvals
            .grants
            .insert(pending.digest, Instant::now() + Duration::from_secs(3600));
    }
    Ok(())
}

#[tauri::command]
pub fn agent_revoke_all(app: AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<Arc<Mutex<Approvals>>>()
        .ok_or("Agent access unavailable")?;
    let mut approvals = state.lock().map_err(|_| "Agent access unavailable")?;
    approvals.pending = None;
    approvals.grants.clear();
    approvals.legacy_token = None;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    command: String,
    #[serde(default = "empty_args")]
    args: Value,
}
fn empty_args() -> Value {
    json!({})
}

pub fn start(app: AppHandle) -> anyhow::Result<()> {
    let token = std::env::var("VERENU_AGENT_TOKEN").ok();
    let port: u16 = if let Some(ref token) = token {
        anyhow::ensure!(
            valid_token(token),
            "VERENU_AGENT_TOKEN must be 32 random bytes encoded as 64 hex characters"
        );
        let port = std::env::var("VERENU_AGENT_PORT")?.parse()?;
        anyhow::ensure!(port > 0, "VERENU_AGENT_PORT must be 1..65535");
        port
    } else {
        0
    };
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    let actual_port = listener.local_addr()?.port();
    let discovery = crate::app_data_dir().join("agent-port");
    std::fs::write(discovery, actual_port.to_string())?;
    let approvals = Arc::new(Mutex::new(Approvals {
        legacy_token: token,
        ..Approvals::default()
    }));
    app.manage(approvals.clone());
    let state = Agent {
        app,
        approvals,
        gate: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(_) => return,
        };
        let routes = Router::new()
            .route("/v1/invoke", post(invoke))
            .route("/v1/authorize", post(authorize))
            .route("/v1/authorization-status", post(authorization_status))
            .route("/v1/revoke", post(revoke))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state);
        if axum::serve(listener, routes).await.is_err() {
            log::error!("Agent API stopped");
        }
    });
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorization {
    token: String,
}

fn is_browser(headers: &HeaderMap) -> bool {
    headers.contains_key("origin") || headers.contains_key("sec-fetch-site")
}

async fn authorize(
    State(agent): State<Agent>,
    headers: HeaderMap,
    Json(request): Json<Authorization>,
) -> (StatusCode, Json<Value>) {
    if is_browser(&headers) || !valid_token(&request.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        );
    }
    let hash = digest(&request.token);
    let code = hash[..6]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    {
        let Ok(mut approvals) = agent.approvals.lock() else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"Unavailable"})),
            );
        };
        approvals
            .grants
            .retain(|_, expiry| *expiry > Instant::now());
        if approvals.grants.len() >= 128 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"Too many active agents"})),
            );
        }
        if approvals
            .pending
            .as_ref()
            .is_some_and(|p| p.expires > Instant::now())
        {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error":"Another approval is pending"})),
            );
        }
        approvals.pending = Some(Pending {
            digest: hash,
            code: code.clone(),
            expires: Instant::now() + Duration::from_secs(120),
        });
    }
    crate::app_setup::show_main_window(&agent.app);
    (
        StatusCode::OK,
        Json(json!({"code":code, "expires_in_seconds":120})),
    )
}

async fn authorization_status(
    State(agent): State<Agent>,
    headers: HeaderMap,
    Json(request): Json<Authorization>,
) -> (StatusCode, Json<Value>) {
    if is_browser(&headers) || !valid_token(&request.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        );
    }
    let hash = digest(&request.token);
    let Ok(mut approvals) = agent.approvals.lock() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"Unavailable"})),
        );
    };
    approvals
        .grants
        .retain(|_, expiry| *expiry > Instant::now());
    let status = if approvals.grants.contains_key(&hash) {
        "approved"
    } else if approvals
        .pending
        .as_ref()
        .is_some_and(|p| p.digest == hash && p.expires > Instant::now())
    {
        "pending"
    } else {
        "denied_or_expired"
    };
    (StatusCode::OK, Json(json!({"status":status})))
}

async fn revoke(
    State(agent): State<Agent>,
    headers: HeaderMap,
    Json(request): Json<Authorization>,
) -> (StatusCode, Json<Value>) {
    if is_browser(&headers) || !valid_token(&request.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Unauthorized"})),
        );
    }
    if let Ok(mut approvals) = agent.approvals.lock() {
        let hash = digest(&request.token);
        approvals.grants.remove(&hash);
        if approvals
            .pending
            .as_ref()
            .is_some_and(|pending| pending.digest == hash)
        {
            approvals.pending = None;
        }
    }
    (StatusCode::OK, Json(json!({"revoked":true})))
}

fn authenticated(headers: &HeaderMap, token: &str) -> bool {
    if is_browser(headers) {
        return false;
    }
    let Some(value) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    // Fixed-size token, constant-time byte comparison.
    value.len() == token.len()
        && value
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn invoke(
    State(agent): State<Agent>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    let allowed = !is_browser(&headers)
        && agent.approvals.lock().ok().is_some_and(|mut approvals| {
            approvals
                .grants
                .retain(|_, expiry| *expiry > Instant::now());
            approvals
                .legacy_token
                .as_ref()
                .is_some_and(|token| authenticated(&headers, token))
                || headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .filter(|v| valid_token(v))
                    .is_some_and(|token| approvals.grants.contains_key(&digest(token)))
        });
    if !allowed {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Unauthorized"})),
        );
    }
    let request: Request = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Expected {command, args} JSON"})),
            )
        }
    };
    let Ok(_permit) = agent.gate.try_acquire() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "Agent API busy; retry after the current request"})),
        );
    };
    match tokio::time::timeout(Duration::from_secs(60), dispatch(&agent.app, request)).await {
        Ok(Ok(value)) => (StatusCode::OK, Json(json!({"version": 1, "result": value}))),
        // Native errors can contain user input. Returned only to the explicitly
        // authorized local caller, never written to ordinary transport logs.
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(
                json!({"error": "Timed out; mutation outcome uncertain. Read state before retrying."}),
            ),
        ),
    }
}

const WRITABLE_SETTINGS: &[&str] = &[
    store::MUTE_AUDIO,
    store::MICROPHONE_DEVICE,
    store::NOISE_REDUCTION,
    store::CLEANUP_ENABLED,
    store::CONTEXTUAL_CAPS,
    store::AUTO_SPACING,
    store::CONTEXTUAL_FORMATTING,
    store::CAPS_LOCK_UPPERCASE,
    store::AUTO_LEARN_ENABLED,
    store::PAUSE_MEDIA_DURING_DICTATION,
    store::PLAY_START_STOP_SOUNDS,
    store::SOUND_EFFECTS_VOLUME,
    store::MIC_GAIN,
    store::TRANSCRIPTION_PROVIDER,
    store::TRANSCRIPTION_MODEL,
    store::TRANSCRIPTION_LANGUAGE,
    store::CLEANUP_PROVIDER,
    store::CLEANUP_MODEL,
    store::CLEANUP_PROMPT_OVERRIDE,
    store::HISTORY_RETENTION,
    store::VOICE_COMMANDS_ENABLED,
    store::CUSTOM_PROVIDERS,
    store::MODEL_SELECTION_MODE,
    store::TRANSCRIPTION_MODELS_BY_PROVIDER,
    store::CLEANUP_MODELS_BY_PROVIDER,
    store::TRANSCRIPTION_DEFAULT_MODEL,
    store::CLEANUP_DEFAULT_MODEL,
    store::TRANSCRIPTION_FALLBACK_MODELS,
    store::CLEANUP_FALLBACK_MODELS,
    store::DUAL_TRANSCRIPTION_ENABLED,
    store::CLEANUP_CACHE_ENABLED,
    store::SUB_APP_CAPTURE_HOTKEY,
    store::LINUX_CANCEL_KEY,
    store::LINUX_HANDSFREE_KEY,
    store::DEFAULT_TONE,
    store::CLEANUP_INTENSITY,
    store::MIC_MUTE_BUTTON_DICTATION,
    store::EXCLUSIVE_MIC,
    store::AUTO_LEARN_EVENT_MODE,
    store::APPEARANCE_MODE,
    store::ACCENT_COLOR,
    store::CUSTOM_THEME,
    store::CUSTOM_THEMES,
    store::ANDROID_PILL_POSITION,
    store::ANDROID_PILL_DOCK_POSITION,
    store::ANDROID_PILL_COVER_KEYBOARD_MIC,
    store::ANDROID_PILL_HIDE_OFFLINE,
    store::ADVANCED_MODEL_UI,
    store::STYLE_PROMPT_INSTRUCTIONS,
    store::BETA_UPDATES_ENABLED,
    store::ANALYTICS_ENABLED,
    store::VERENU_SERVICE_CHECKS_ENABLED,
    store::GITHUB_USERNAME,
    store::LOCAL_MODEL_MEMORY_POLICY,
    store::CLIPBOARD_PHRASE_ENABLED,
    store::CLIPBOARD_PHRASE,
    store::LEGACY_FEATURES_ENABLED,
    store::SYNC_ENABLED,
    store::SYNC_MUTING_ENABLED,
];

// One registry generates argument discovery and dispatch. Calling the existing
// native handlers preserves validation, database transactions and runtime effects.
macro_rules! native_commands {
    ($mode:ident, $app:ident, $request:ident, $( $name:ident { $( $field:ident : $ty:ty ),* $(,)? } ),* $(,)?) => {
        native_commands!(@$mode, $app, $request, $( $name { $( $field : $ty ),* } ),*)
    };
    (@describe, $app:ident, $request:ident, $( $name:ident { $( $field:ident : $ty:ty ),* } ),*) => {
        vec![$( json!({"command": stringify!($name), "arguments": { $( camel_case(stringify!($field)): stringify!($ty) ),* } }) ),*]
    };
    (@dispatch, $app:ident, $request:ident, $( $name:ident { $( $field:ident : $ty:ty ),* } ),*) => {
        match $request.command.as_str() {
            $(stringify!($name) => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Args { $( $field: $ty ),* }
                let args: Args = serde_json::from_value($request.args).map_err(|e| e.to_string())?;
                let Args { $( $field ),* } = args;
                serde_json::to_value(commands::$name($app.clone(), $( $field ),*).await?).map_err(|e| e.to_string())
            }),*
            _ => Err("Unsupported command; use describe".into())
        }
    };
}

fn camel_case(field: &str) -> String {
    let mut parts = field.split('_');
    let mut result = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            result.extend(chars);
        }
    }
    result
}

macro_rules! registry {
    ($mode:ident, $app:ident, $request:ident) => {
        native_commands!($mode, $app, $request,
            get_all_settings {}, get_setting { key: String },
            test_cleanup_prompt { provider: String, model: String, template: String, intensity: Option<String>, tone: Option<String> },
            get_contexts {},
            create_context { name: String, icon: Option<String>, tone: Option<String>, cleanup_intensity: Option<String>, custom_instructions: Option<String>, contextual_formatting_disabled: Option<bool>, contextual_formatting_override: Option<bool>, paste_in_chunks: Option<bool> },
            duplicate_context { context_id: i64 }, update_context { context_id: i64, name: String },
            update_context_settings { context_id: i64, icon: Option<String>, tone: Option<String>, cleanup_intensity: Option<String>, custom_instructions: Option<String>, contextual_formatting_disabled: Option<bool>, contextual_formatting_override: Option<bool>, paste_in_chunks: Option<bool> },
            update_context_color { context_id: i64, color: Option<String> },
            set_context_t3_skill_mentions { context_id: i64, enabled: bool },
            set_context_pinned { context_id: i64, pinned: bool }, delete_context { context_id: i64 },
            get_context_stats { context_id: i64 }, get_context_targets { context_id: Option<i64> },
            assign_context_target { context_id: i64, executable: String, app_name: Option<String>, developer: Option<String> },
            remove_context_target { context_id: i64, executable: String },
            get_context_websites { context_id: Option<i64> },
            assign_context_website { context_id: i64, domain: String }, remove_context_website { context_id: i64, domain: String },
            get_context_dictionary { context_id: i64 }, get_context_snippets { context_id: i64 },
            get_dictionary {},
            create_dictionary_entry { term: String, mistake: Option<String>, context_id: Option<i64> },
            edit_dictionary_entry { id: i64, term: String, mistake: Option<String>, dictionary_id: Option<i64>, context_id: Option<i64>, correction_id: Option<i64>, correction_ids: Option<Vec<i64>> },
            remove_dictionary_entry { id: i64, dictionary_id: Option<i64>, context_id: Option<i64>, correction_id: Option<i64>, correction_ids: Option<Vec<i64>> },
            move_dictionary_entry_to_context { dictionary_id: i64, source_context_id: i64, target_context_id: i64, correction_id: Option<i64>, correction_ids: Option<Vec<i64>> },
            get_dictionary_entry_contexts { term: String },
            move_dictionary_entry_by_term_to_context { term: String, context_id: i64 },
            set_dictionary_context_assignment { context_id: i64, dictionary_id: i64, assigned: bool },
            get_snippets {},
            create_snippet { trigger: String, expansion: String, instructions: String, context_id: Option<i64> },
            edit_snippet { id: i64, trigger: String, expansion: String, instructions: String },
            remove_snippet { id: i64 },
            get_snippet_entry_contexts { trigger: String },
            move_snippet_entry_to_context { trigger: String, context_id: i64 },
            set_snippet_context_assignment { context_id: i64, snippet_id: i64, assigned: bool },
            get_sub_apps {},
            create_sub_app { executable: String, app_name: Option<String>, label: String, icon: Option<String>, title_pattern: String, match_mode: String },
            assign_sub_app { id: i64, context_id: Option<i64> }, delete_sub_app { id: i64 },
            save_hotkey { keys: Vec<String> }, set_autostart { enabled: bool },
            get_api_key_status {}, delete_custom_provider { provider: String },
            sync_get_status {}, sync_set_device_name { name: String },
            sync_set_peer_address { device_uuid: String, address: String },
            sync_pair_connection { details: String }, sync_start_pairing { device_uuid: String },
            sync_respond_to_pairing { code: String, approve: bool }, sync_cancel_pairing {},
            sync_remove_device { device_uuid: String }, sync_now { device_uuid: Option<String> },
            get_t3_skills {}, pull_t3_skills { force: Option<bool> },
            select_t3_catalog { catalog_id: String }, disconnect_t3 {}
        )
    };
}

async fn dispatch(app: &AppHandle, request: Request) -> Result<Value, String> {
    if !request.args.is_object() {
        return Err("args must be an object".into());
    }
    if matches!(
        request.command.as_str(),
        "describe" | "get_microphones" | "diagnostics.snapshot"
    ) && request
        .args
        .as_object()
        .is_some_and(|args| !args.is_empty())
    {
        return Err("This command does not accept arguments".into());
    }
    if request.command == "describe" {
        return Ok(
            json!({"version": 1, "native_commands": registry!(describe, app, request),
            "argument_naming": "Native command fields use camelCase; optional fields may be omitted. update_context_settings replaces all fields; read then modify.",
            "writable_settings": WRITABLE_SETTINGS,
            "additional_commands": {
                "save_setting": {"key": "writable setting", "value": "native validated JSON"},
                "credentials.save": {"provider": "string", "key": "secret; supply through private stdin"},
                "credentials.delete": {"provider": "string"},
                "t3.connect": {"pairingLink": "private pairing link; supply through stdin"},
                "context.patch_settings": {"contextId": "integer", "settings": "object: icon, tone, cleanupIntensity, customInstructions, contextualFormattingOverride, pasteInChunks; omitted fields are preserved, null clears nullable fields"},
                "get_microphones": {}, "diagnostics.snapshot": {},
                "diagnostics.logs": {"since_ms": "optional inclusive UTC milliseconds", "until_ms": "optional exclusive UTC milliseconds", "subsystem": "optional substring", "level": "optional exact level: error, warn, info, debug, trace", "limit": "1..1000 (default 200); newest matches"},
                "prompts.lint": {"template": "string", "intensity": "optional string", "tone": "optional string"},
                "history.report": {"start": "YYYY-MM-DD UTC inclusive", "end": "YYYY-MM-DD UTC exclusive"},
                "history.usage": {"start": "YYYY-MM-DD UTC inclusive", "end": "YYYY-MM-DD UTC exclusive"},
                "history.page": {"start": "date", "end": "date", "after_id": "optional integer", "limit": "1..500 (default 100)", "include_text": "boolean, default false"},
                "diagnostics.top": {"start": "date", "end": "date"},
                "diagnostics.retention": {"days": "optional 0..365; 0 disables and clears"}
            },
            "security": "Token grants broad settings and data writes, credential replacement/deletion, sync control and explicit history text access. Loopback only; no credential reads, SQL, shell, audio capture, or injection operations."}),
        );
    }
    match request.command.as_str() {
        "t3.connect" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                pairing_link: String,
            }
            let args: Args = serde_json::from_value(request.args)
                .map_err(|_| "Expected pairingLink string".to_string())?;
            let result = commands::connect_t3(app.clone(), args.pairing_link)
                .await
                .map_err(|_| "Could not connect T3".to_string())?;
            serde_json::to_value(result).map_err(|_| "Could not encode T3 status".to_string())
        }
        "credentials.save" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                provider: String,
                key: String,
            }
            let args: Args = serde_json::from_value(request.args)
                .map_err(|_| "Expected provider and key strings".to_string())?;
            commands::save_api_key(app.clone(), args.provider, args.key)
                .await
                .map_err(|_| "Could not save credential".to_string())?;
            let _ = app.emit("verenu:agent-settings-changed", ());
            Ok(Value::Null)
        }
        "credentials.delete" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                provider: String,
            }
            let args: Args = serde_json::from_value(request.args)
                .map_err(|_| "Expected provider string".to_string())?;
            commands::delete_api_key(app.clone(), args.provider)
                .await
                .map_err(|_| "Could not delete credential".to_string())?;
            let _ = app.emit("verenu:agent-settings-changed", ());
            Ok(Value::Null)
        }
        "context.patch_settings" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                context_id: i64,
                settings: db::ContextSettingsPatch,
            }
            let args: Args = serde_json::from_value(request.args).map_err(|e| e.to_string())?;
            let db = app.state::<crate::DbHandle>().inner().clone();
            let context = tokio::task::spawn_blocking(move || {
                db::patch_context_settings(&db, args.context_id, args.settings)
            })
            .await
            .map_err(|_| "Context worker unavailable".to_string())?
            .map_err(|e| e.to_string())?;
            let _ = app.emit("verenu:agent-contexts-changed", ());
            serde_json::to_value(context).map_err(|e| e.to_string())
        }
        "save_setting" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                key: String,
                value: Value,
            }
            let args: Args = serde_json::from_value(request.args).map_err(|e| e.to_string())?;
            if !WRITABLE_SETTINGS.contains(&args.key.as_str()) {
                return Err("Setting is not agent-writable; use describe".into());
            }
            if args.key == store::CUSTOM_PROVIDERS {
                let settings = store::settings_snapshot(app)?;
                let before =
                    crate::api::custom::parse_stored(settings.get(store::CUSTOM_PROVIDERS));
                let after = crate::api::custom::normalize_list(&args.value)?;
                if before
                    .iter()
                    .any(|old| !after.iter().any(|new| new.id == old.id))
                {
                    return Err("Remove providers with delete_custom_provider so model selections are repaired".into());
                }
            }
            commands::save_setting(app.clone(), args.key.clone(), args.value).await?;
            let _ = app.emit("verenu:agent-settings-changed", vec![args.key]);
            Ok(Value::Null)
        }
        "get_microphones" => {
            serde_json::to_value(commands::get_microphones().await).map_err(|e| e.to_string())
        }
        "prompts.lint" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                template: String,
                intensity: Option<String>,
                tone: Option<String>,
            }
            let args: Args = serde_json::from_value(request.args).map_err(|e| e.to_string())?;
            Ok(json!(commands::lint_cleanup_prompt(
                args.template,
                args.intensity,
                args.tone
            )))
        }
        "diagnostics.snapshot" => serde_json::to_value(
            commands::get_diagnostics_snapshot(app.clone(), app.state::<crate::SharedState>())
                .await?,
        )
        .map_err(|e| e.to_string()),
        "diagnostics.logs" => crate::system::diagnostics::query_logs(
            serde_json::from_value(request.args).map_err(|e| e.to_string())?,
        ),
        "history.report"
        | "history.usage"
        | "history.page"
        | "diagnostics.top"
        | "diagnostics.retention" => {
            let db = app.state::<crate::DbHandle>().inner().clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
                match request.command.as_str() {
                    "history.report" => {
                        db::agent_history_report(&db, serde_json::from_value(request.args)?)
                    }
                    "history.usage" => {
                        db::agent_history_usage(&db, serde_json::from_value(request.args)?)
                    }
                    "history.page" => {
                        db::agent_history_page(&db, serde_json::from_value(request.args)?)
                    }
                    "diagnostics.top" => {
                        let mut result =
                            db::diagnostic_top(&db, serde_json::from_value(request.args)?)?;
                        result["dropped_this_process"] =
                            json!(crate::system::diagnostic_journal::dropped());
                        Ok(result)
                    }
                    _ => {
                        #[derive(Deserialize)]
                        #[serde(deny_unknown_fields)]
                        struct Retention {
                            days: Option<u32>,
                        }
                        let args: Retention = serde_json::from_value(request.args)?;
                        Ok(json!({"retention_days": db::diagnostic_retention(&db, args.days)?}))
                    }
                }
            })
            .await
            .map_err(|_| "History worker unavailable".to_string())?
            .map_err(|e| e.to_string())
        }
        _ => {
            let refresh_settings = request.command == "delete_custom_provider";
            let refresh_contexts = matches!(request.command.as_str(),
                "create_context" | "duplicate_context" | "update_context" | "update_context_settings"
                | "update_context_color" | "set_context_t3_skill_mentions" | "set_context_pinned" | "delete_context"
                | "assign_context_target" | "remove_context_target" | "assign_context_website" | "remove_context_website"
                | "create_dictionary_entry" | "edit_dictionary_entry" | "remove_dictionary_entry" | "move_dictionary_entry_to_context"
                | "move_dictionary_entry_by_term_to_context" | "set_dictionary_context_assignment"
                | "create_snippet" | "edit_snippet" | "remove_snippet" | "move_snippet_entry_to_context" | "set_snippet_context_assignment"
                | "create_sub_app" | "assign_sub_app" | "delete_sub_app");
            let result = registry!(dispatch, app, request);
            if refresh_contexts && result.is_ok() {
                let _ = app.emit("verenu:agent-contexts-changed", ());
            }
            if refresh_settings && result.is_ok() {
                let _ = app.emit("verenu:agent-settings-changed", ());
            }
            result
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_context_commands_advertise_nullable_formatting_overrides() {
        let described = registry!(describe, app, request);
        for command in ["create_context", "update_context_settings"] {
            let entry = described
                .iter()
                .find(|entry| entry["command"] == command)
                .unwrap();
            for field in [
                "contextualFormattingDisabled",
                "contextualFormattingOverride",
            ] {
                let argument_type = entry["arguments"][field].as_str().unwrap();
                assert_eq!(argument_type.replace(' ', ""), "Option<bool>");
            }
        }
    }

    #[test]
    fn agent_api_rejects_browser_origins_and_invalid_tokens() {
        let token = "a".repeat(64);
        let mut headers = HeaderMap::new();
        assert!(!authenticated(&headers, &token));
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(authenticated(&headers, &token));
        headers.insert("origin", "http://localhost".parse().unwrap());
        assert!(!authenticated(&headers, &token));
        headers.remove("origin");
        headers.insert(
            "authorization",
            format!("Bearer {}", "b".repeat(64)).parse().unwrap(),
        );
        assert!(!authenticated(&headers, &token));
    }

    #[test]
    fn agent_context_library_operations_are_discoverable() {
        let described = registry!(describe, unused_app, unused_request);
        let commands = described.as_array().unwrap();
        for command in [
            "get_dictionary",
            "get_snippets",
            "get_dictionary_entry_contexts",
            "get_snippet_entry_contexts",
            "set_dictionary_context_assignment",
            "set_snippet_context_assignment",
        ] {
            assert!(
                commands.iter().any(|entry| entry["command"] == command),
                "missing {command}"
            );
        }
    }
}
