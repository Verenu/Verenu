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
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub(crate) struct Agent {
    app: AppHandle,
    pairing: Arc<Mutex<Pairing>>,
    gate: Arc<tokio::sync::Semaphore>,
}

struct Pairing {
    token: Option<String>,
    pending: Option<(String, Instant)>,
    port: u16,
    path: Option<std::path::PathBuf>,
    instance: String,
}

impl Pairing {
    fn publish(&self, status: &str) -> anyhow::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            let directory = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Missing agent directory"))?;
            let metadata = std::fs::symlink_metadata(directory)?;
            // A writable profile directory would let another user replace the
            // descriptor even though the descriptor itself is private.
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o022 != 0
            {
                anyhow::bail!("Agent profile directory is not private to this user");
            }
            // Also reject a profile under an ancestor another user can replace.
            // Check both the visible and resolved paths, since either can
            // contain a symlink. A root-owned sticky directory such as /tmp is
            // safe to traverse when our next directory is owned by this user.
            let resolved = std::fs::canonicalize(directory)?;
            for ancestor in directory.ancestors().chain(resolved.ancestors()) {
                let entry = std::fs::symlink_metadata(ancestor)?;
                let uid = entry.uid();
                if uid != 0 && uid != unsafe { libc::geteuid() } {
                    anyhow::bail!("Agent profile has an untrusted ancestor");
                }
                if entry.is_dir()
                    && entry.mode() & 0o022 != 0
                    && !(uid == 0 && entry.mode() & 0o1000 != 0)
                {
                    anyhow::bail!("Agent profile has a writable ancestor");
                }
            }
            if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                anyhow::bail!("Agent descriptor cannot be a symbolic link");
            }
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            let payload = json!({"version": 1, "port": self.port, "status": status, "token": self.token, "instance": self.instance});
            let result = (|| -> anyhow::Result<()> {
                file.write_all(&serde_json::to_vec(&payload)?)?;
                file.sync_all()?;
                std::fs::rename(&temp, path)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(temp);
            }
            result
        }
        #[cfg(not(unix))]
        anyhow::bail!("Secure shared agent grants are unavailable on this platform")
    }
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
    if let Some(token) = &token {
        anyhow::ensure!(
            token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
            "VERENU_AGENT_TOKEN must be 32 random bytes encoded as 64 hex characters"
        );
    }
    #[cfg(not(unix))]
    if token.is_none() {
        return Ok(());
    }
    let port: u16 = if token.is_some() {
        std::env::var("VERENU_AGENT_PORT")?.parse()?
    } else {
        0
    };
    anyhow::ensure!(
        token.is_none() || port > 0,
        "VERENU_AGENT_PORT must be 1..65535"
    );
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let path = if token.is_none() {
        Some(crate::app_data_dir().join("agent-session.json"))
    } else {
        None
    };
    let pairing = Pairing {
        token,
        pending: None,
        port,
        path,
        instance: uuid::Uuid::new_v4().to_string(),
    };
    pairing.publish("idle")?;
    let state = Agent {
        app: app.clone(),
        pairing: Arc::new(Mutex::new(pairing)),
        gate: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    app.manage(state.clone());
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(_) => return,
        };
        let routes = Router::new()
            .route("/v1/invoke", post(invoke))
            .route("/v1/pair", post(pair))
            .route("/v1/status", get(status))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state);
        if axum::serve(listener, routes).await.is_err() {
            log::error!("Agent API stopped");
        }
    });
    Ok(())
}

async fn status(State(agent): State<Agent>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if headers.contains_key("origin") || headers.contains_key("sec-fetch-site") {
        return (StatusCode::FORBIDDEN, Json(json!({"error":"Forbidden"})));
    }
    match agent.pairing.lock() {
        Ok(state) if state.path.is_some() => (
            StatusCode::OK,
            Json(json!({"version":1,"instance":state.instance})),
        ),
        _ => (StatusCode::NOT_FOUND, Json(json!({"error":"Unavailable"}))),
    }
}

pub(crate) fn clear_session(app: &AppHandle) {
    let Some(agent) = app.try_state::<Agent>() else {
        return;
    };
    let Ok(state) = agent.pairing.lock() else {
        return;
    };
    let Some(path) = &state.path else {
        return;
    };
    // A newer process may have replaced this file; never remove its grant.
    let ours = std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
        .and_then(|data| {
            data.get("instance")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|instance| instance == state.instance);
    if ours {
        let _ = std::fs::remove_file(path);
    }
}

async fn pair(State(agent): State<Agent>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if headers.contains_key("origin") || headers.contains_key("sec-fetch-site") {
        return (StatusCode::FORBIDDEN, Json(json!({"error":"Forbidden"})));
    }
    let id = {
        let Ok(mut state) = agent.pairing.lock() else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"Unavailable"})),
            );
        };
        if state.path.is_none() {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"Pairing unavailable"})),
            );
        }
        if state.token.is_some() {
            return (StatusCode::OK, Json(json!({"status":"approved"})));
        }
        if let Some((_, started)) = &state.pending {
            if started.elapsed() < Duration::from_secs(120) {
                return (StatusCode::ACCEPTED, Json(json!({"status":"pending"})));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        state.pending = Some((id.clone(), Instant::now()));
        if state.publish("pending").is_err() {
            state.pending = None;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"Unable to create request"})),
            );
        }
        id
    };
    crate::show_main_window(&agent.app);
    let _ = agent.app.emit_to("main", "verenu:agent-pair-request", &id);
    let timeout_agent = agent.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        let expired = if let Ok(mut state) = timeout_agent.pairing.lock() {
            if state.pending.as_ref().is_some_and(|(pending, started)| {
                pending == &id && started.elapsed() >= Duration::from_secs(120)
            }) {
                state.pending = None;
                let _ = state.publish("denied");
                true
            } else {
                false
            }
        } else {
            false
        };
        if expired {
            let _ = timeout_agent
                .app
                .emit_to("main", "verenu:agent-pair-cleared", &id);
        }
    });
    (StatusCode::ACCEPTED, Json(json!({"status":"pending"})))
}

#[tauri::command]
pub(crate) fn agent_pair_pending(state: tauri::State<'_, Agent>) -> Option<String> {
    let state = state.pairing.lock().ok()?;
    state
        .pending
        .as_ref()
        .filter(|(_, started)| started.elapsed() < Duration::from_secs(120))
        .map(|(id, _)| id.clone())
}

#[tauri::command]
pub(crate) fn agent_pair_respond(
    app: AppHandle,
    state: tauri::State<'_, Agent>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    use rand::RngCore;
    let mut state = state.pairing.lock().map_err(|_| "Pairing unavailable")?;
    let valid = state.pending.as_ref().is_some_and(|(id, started)| {
        id == &request_id && started.elapsed() < Duration::from_secs(120)
    });
    if !valid {
        return Err("This access request has expired. Ask the agent to try again.".into());
    }
    if approved {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        state.token = Some(bytes.iter().map(|b| format!("{b:02x}")).collect());
    }
    if let Err(error) = state.publish(if approved { "approved" } else { "denied" }) {
        state.token = None;
        return Err(format!("Could not save agent access: {error}"));
    }
    state.pending = None;
    let _ = app.emit_to("main", "verenu:agent-pair-cleared", &request_id);
    Ok(())
}

fn authenticated(headers: &HeaderMap, token: &str) -> bool {
    if headers.contains_key("origin") || headers.contains_key("sec-fetch-site") {
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
    let authorized = agent
        .pairing
        .lock()
        .ok()
        .and_then(|state| {
            state
                .token
                .as_ref()
                .map(|token| authenticated(&headers, token))
        })
        .unwrap_or(false);
    if !authorized {
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
];

// One registry generates argument discovery and dispatch. Calling the existing
// native handlers preserves validation, database transactions and runtime effects.
macro_rules! native_commands {
    ($mode:ident, $app:ident, $request:ident, $( $name:ident { $( $field:ident : $ty:ty ),* $(,)? } ),* $(,)?) => {
        native_commands!(@$mode, $app, $request, $( $name { $( $field : $ty ),* } ),*)
    };
    (@describe, $app:ident, $request:ident, $( $name:ident { $( $field:ident : $ty:ty ),* } ),*) => {
        json!([$( {"command": stringify!($name), "arguments": { $( camel_case(stringify!($field)): stringify!($ty) ),* } } ),*])
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
            set_context_pinned { context_id: i64, pinned: bool }, delete_context { context_id: i64 },
            get_context_stats { context_id: i64 }, get_context_targets { context_id: Option<i64> },
            assign_context_target { context_id: i64, executable: String, app_name: Option<String>, developer: Option<String> },
            remove_context_target { context_id: i64, executable: String },
            get_context_websites { context_id: Option<i64> },
            assign_context_website { context_id: i64, domain: String }, remove_context_website { context_id: i64, domain: String },
            get_dictionary {}, get_snippets {},
            get_context_dictionary { context_id: i64 }, get_context_snippets { context_id: i64 },
            get_dictionary_entry_contexts { term: String }, get_snippet_entry_contexts { trigger: String },
            set_dictionary_context_assignment { context_id: i64, dictionary_id: i64, assigned: bool },
            set_snippet_context_assignment { context_id: i64, snippet_id: i64, assigned: bool },
            get_sub_apps {}
        )
    };
}

async fn dispatch(app: &AppHandle, request: Request) -> Result<Value, String> {
    if !request.args.is_object() {
        return Err("args must be an object".into());
    }
    if request.command == "describe" {
        return Ok(
            json!({"version": 1, "native_commands": registry!(describe, app, request),
            "argument_naming": "Native command fields use camelCase; optional fields may be omitted. update_context_settings replaces all fields; read then modify.",
            "writable_settings": WRITABLE_SETTINGS,
            "additional_commands": {
                "save_setting": {"key": "writable setting", "value": "native validated JSON"},
                "get_microphones": {}, "diagnostics.snapshot": {},
                "diagnostics.logs": {"since_ms": "optional inclusive UTC milliseconds", "until_ms": "optional exclusive UTC milliseconds", "subsystem": "optional substring", "level": "optional exact level: error, warn, info, debug, trace", "limit": "1..1000 (default 200); newest matches"},
                "prompts.lint": {"template": "string", "intensity": "optional string", "tone": "optional string"},
                "history.report": {"start": "YYYY-MM-DD UTC inclusive", "end": "YYYY-MM-DD UTC exclusive"},
                "history.usage": {"start": "YYYY-MM-DD UTC inclusive", "end": "YYYY-MM-DD UTC exclusive"},
                "history.page": {"start": "date", "end": "date", "after_id": "optional integer", "limit": "1..500 (default 100)", "include_text": "boolean, default false"},
                "diagnostics.top": {"start": "date", "end": "date"},
                "diagnostics.retention": {"days": "optional 0..365; 0 disables and clears"}
            },
            "security": "Token grants settings/context writes and explicit history text access. Loopback only; no credential, SQL, shell, audio capture, or injection operations."}),
        );
    }
    match request.command.as_str() {
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
            commands::save_setting(app.clone(), args.key, args.value).await?;
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
        _ => registry!(dispatch, app, request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn session_descriptor_is_private_and_contains_no_token_before_approval() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("verenu-agent-pair-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("agent-session.json");
        let mut state = Pairing {
            token: None,
            pending: None,
            port: 14201,
            path: Some(path.clone()),
            instance: "synthetic".into(),
        };
        state.publish("idle").unwrap();
        let descriptor: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(descriptor["status"], "idle");
        assert!(descriptor["token"].is_null());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        state.token = Some("a".repeat(64));
        state.publish("approved").unwrap();
        let descriptor: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(descriptor["status"], "approved");
        assert_eq!(descriptor["token"], "a".repeat(64));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn agent_context_commands_advertise_nullable_formatting_overrides() {
        let described = registry!(describe, app, request);
        for command in ["create_context", "update_context_settings"] {
            let entry = described
                .as_array()
                .unwrap()
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
