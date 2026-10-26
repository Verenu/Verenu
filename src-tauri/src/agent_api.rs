//! Opt-in local script API. Native command handlers remain the write boundary.
use crate::{
    commands,
    data::{db, store},
};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
struct Agent {
    app: AppHandle,
    token: Arc<String>,
    gate: Arc<tokio::sync::Semaphore>,
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
    let Ok(token) = std::env::var("VERENU_AGENT_TOKEN") else {
        return Ok(());
    };
    anyhow::ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "VERENU_AGENT_TOKEN must be 32 random bytes encoded as 64 hex characters"
    );
    let port: u16 = std::env::var("VERENU_AGENT_PORT")?.parse()?;
    anyhow::ensure!(port > 0, "VERENU_AGENT_PORT must be 1..65535");
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    let state = Agent {
        app,
        token: Arc::new(token),
        gate: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(_) => return,
        };
        let routes = Router::new()
            .route("/v1/invoke", post(invoke))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state);
        if axum::serve(listener, routes).await.is_err() {
            log::error!("Agent API stopped");
        }
    });
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
    if !authenticated(&headers, &agent.token) {
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
            get_context_dictionary { context_id: i64 }, get_context_snippets { context_id: i64 }
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
        "history.report" | "history.page" | "diagnostics.top" | "diagnostics.retention" => {
            let db = app.state::<crate::DbHandle>().inner().clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
                match request.command.as_str() {
                    "history.report" => {
                        db::agent_history_report(&db, serde_json::from_value(request.args)?)
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
    #[test]
    fn agent_context_commands_advertise_nullable_formatting_overrides() {
        let described = registry!(describe, app, request);
        for command in ["create_context", "update_context_settings"] {
            let entry = described.as_array().unwrap().iter()
                .find(|entry| entry["command"] == command).unwrap();
            for field in ["contextualFormattingDisabled", "contextualFormattingOverride"] {
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
}
