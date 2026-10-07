//! Development-only browser transport. All app commands use Tauri's existing
//! dispatcher; the transport never implements a parallel settings/database API.
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path as RoutePath, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody},
    AppHandle, Listener, Manager,
};
use tokio::sync::Semaphore;

const MAX_EVENTS: usize = 512;
const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024;
type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

pub(crate) fn enabled() -> bool {
    std::env::var_os("VERENU_DEV_SESSION_DIR").is_some()
}

fn session_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("VERENU_DEV_SESSION_DIR").expect("session directory"))
}

/// Snapshot only once. The installed database is always opened read-only.
pub(crate) fn prepare() -> anyhow::Result<()> {
    if !enabled() {
        return Ok(());
    }
    let source = std::env::var_os("VERENU_DEV_SEED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::app_data_dir);
    let root = session_dir();
    anyhow::ensure!(root.is_absolute(), "Session directory must be absolute");
    anyhow::ensure!(
        root != source,
        "Session directory must differ from installed app data"
    );
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    let data = root.join("data");
    fs::create_dir_all(&data)?;
    if !root.join("seeded").exists() {
        let db_path = data.join("verenu.db");
        if source.join("verenu.db").exists() {
            let conn = rusqlite::Connection::open_with_flags(
                source.join("verenu.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            conn.backup(rusqlite::DatabaseName::Main, &db_path, None)?;
            if std::env::var("VERENU_DEV_PRIVATE_HISTORY").as_deref() != Ok("1") {
                scrub_history(&db_path)?;
            }
        }
        let mut settings = if source.join("settings.json").exists() {
            serde_json::from_slice::<Value>(&fs::read(source.join("settings.json"))?)?
        } else {
            json!({})
        };
        sanitize_settings(&mut settings)?;
        fs::write(
            data.join("settings.json"),
            serde_json::to_vec_pretty(&settings)?,
        )?;
        fs::write(root.join("seeded"), b"1")?;
    }
    std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", &data);
    Ok(())
}

fn sanitize_settings(settings: &mut Value) -> anyhow::Result<()> {
    use crate::data::store;
    let settings = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Settings must be an object"))?;
    for key in [
        store::KEY_GROQ,
        store::KEY_OPENAI,
        store::KEY_GOOGLE,
        store::KEY_ASSEMBLYAI,
        store::KEY_OPENROUTER,
        store::KEY_XAI,
    ] {
        settings.remove(key);
    }
    // A cloned session never advertises the installed sync identity or phones home.
    settings.insert(store::SYNC_ENABLED.into(), json!(false));
    settings.insert(store::ANALYTICS_ENABLED.into(), json!(false));
    settings.insert(store::AUTO_LEARN_ENABLED.into(), json!(false));
    settings.insert(store::PAUSE_MEDIA_DURING_DICTATION.into(), json!(false));
    settings.insert(store::SOUND_EFFECTS_VOLUME.into(), json!(0.0));
    settings.insert(store::PLAY_START_STOP_SOUNDS.into(), json!(false));
    settings.insert(store::CLIPBOARD_PHRASE_ENABLED.into(), json!(false));
    settings.insert(store::MIC_MUTE_BUTTON_DICTATION.into(), json!(false));
    settings.insert(store::CREDENTIALS_MIGRATED.into(), json!(true));
    Ok(())
}

fn scrub_history(path: &Path) -> anyhow::Result<()> {
    // Migrate the copy before scrubbing, never the source. Preserve customization
    // tables explicitly; all transcript-derived state and sync identity is removed.
    let db = crate::data::db::open(path)?;
    let mut conn = db
        .lock()
        .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let tx = conn.transaction()?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
    for name in tables {
        if !matches!(
            name.as_str(),
            "contexts"
                | "context_targets"
                | "context_sub_apps"
                | "context_website_targets"
                | "dictionary"
                | "snippets"
                | "dictionary_contexts"
                | "snippet_contexts"
                | "seeded_defaults"
        ) {
            tx.execute(
                &format!("DELETE FROM \"{}\"", name.replace('"', "\"\"")),
                [],
            )?;
        }
    }
    tx.commit()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
    Ok(())
}

#[derive(Clone)]
struct Bridge {
    // Axum clones its state on multiple worker threads. Share the native
    // handle instead of cloning Wry's main-thread Rc state in each request.
    app: Arc<AppHandle>,
    token: String,
    origins: Vec<String>,
    events: Arc<Mutex<EventBuffer>>,
    subscriptions: Arc<Mutex<HashMap<String, tauri::EventId>>>,
    audio_gate: Arc<Semaphore>,
    runs: Arc<Mutex<u32>>,
    max_runs: u32,
    host_mic: bool,
    host_mic_lock: Arc<Mutex<Option<fs::File>>>,
}

#[derive(Default)]
struct EventBuffer {
    seq: u64,
    rows: VecDeque<Value>,
}

impl EventBuffer {
    fn push(&mut self, event: &str, payload: Value) {
        self.seq += 1;
        self.rows
            .push_back(json!({"id": self.seq, "event": event, "payload": payload}));
        while self.rows.len() > MAX_EVENTS {
            self.rows.pop_front();
        }
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": message.into()})))
}

pub(crate) fn start(app: AppHandle) -> anyhow::Result<()> {
    let access: Value = serde_json::from_slice(&fs::read(session_dir().join("access.json"))?)?;
    let token = access["token"]
        .as_str()
        .filter(|token| token.len() >= 32)
        .ok_or_else(|| anyhow::anyhow!("Invalid session access file"))?
        .to_owned();
    let port: u16 = std::env::var("VERENU_DEV_BRIDGE_PORT")?.parse()?;
    let origins: Vec<String> = std::env::var("VERENU_DEV_ORIGINS")?
        .split(',')
        .map(str::to_owned)
        .collect();
    anyhow::ensure!(
        !origins.is_empty(),
        "At least one browser origin is required"
    );
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    listener.set_nonblocking(true)?;
    let bridge = Bridge {
        app: Arc::new(app),
        token,
        origins,
        events: Default::default(),
        subscriptions: Default::default(),
        audio_gate: Arc::new(Semaphore::new(1)),
        runs: Default::default(),
        max_runs: std::env::var("VERENU_DEV_MAX_RUNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30),
        host_mic: std::env::var("VERENU_DEV_HOST_MIC").as_deref() == Ok("1"),
        host_mic_lock: Default::default(),
    };
    for event in [
        "verenu:transcribed",
        "verenu:error",
        "verenu:pipeline-failed",
        "verenu:pill-stage",
        "verenu:pill-state",
    ] {
        bridge.subscribe(event)?;
    }
    let routes = Router::new()
        .route("/session", get(session))
        .route("/invoke", post(invoke))
        .route("/listen", post(subscribe))
        .route("/events", get(events))
        .route("/logs", get(logs))
        .route("/fixtures", get(fixtures))
        .route("/fixtures/{name}", get(fixture))
        .route("/audio", post(audio))
        .route_layer(middleware::from_fn_with_state(bridge.clone(), authenticate))
        .layer(DefaultBodyLimit::max(MAX_AUDIO_BYTES))
        .with_state(bridge);
    tauri::async_runtime::spawn(async move {
        let result = async {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            axum::serve(listener, routes).await
        }
        .await;
        if let Err(err) = result {
            log::error!("Dev bridge stopped: {err}");
        }
    });
    fs::write(session_dir().join("ready"), port.to_string())?;
    Ok(())
}

async fn authenticate(
    State(bridge): State<Bridge>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let expected = format!("Bearer {}", bridge.token);
    if request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        != Some(expected.as_str())
    {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "Session authentication required",
        ));
    }
    if let Some(origin) = request.headers().get("origin") {
        if !origin
            .to_str()
            .ok()
            .is_some_and(|v| bridge.origins.iter().any(|allowed| allowed == v))
        {
            return Err(error(
                StatusCode::FORBIDDEN,
                "Browser origin is not allowed for this session",
            ));
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    Ok(response)
}

impl Bridge {
    fn subscribe(&self, name: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            (name.starts_with("verenu:")
                || name.starts_with("local-stt-")
                || name.starts_with("local-llm-")
                || name == "open-flow:open-settings-section")
                && name.len() < 100,
            "Unsupported event name"
        );
        let mut subs = self
            .subscriptions
            .lock()
            .map_err(|_| anyhow::anyhow!("Event subscription lock failed"))?;
        if subs.contains_key(name) {
            return Ok(());
        }
        anyhow::ensure!(subs.len() < 128, "Too many event subscriptions");
        let buffer = self.events.clone();
        let event_name = name.to_owned();
        let id = self.app.listen_any(name, move |event| {
            if let (Ok(mut buffer), Ok(payload)) = (
                buffer.lock(),
                serde_json::from_str::<Value>(event.payload()),
            ) {
                buffer.push(&event_name, payload);
            }
        });
        subs.insert(name.into(), id);
        Ok(())
    }

    fn spend_run(&self) -> Result<(), String> {
        let mut runs = self.runs.lock().map_err(|_| "Run counter lock failed")?;
        if *runs >= self.max_runs {
            return Err("Session live-run limit reached. Start a new session or raise --max-runs explicitly.".into());
        }
        *runs += 1;
        Ok(())
    }
}

async fn session(State(bridge): State<Bridge>) -> ApiResult {
    let runs = *bridge.runs.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Session state unavailable",
        )
    })?;
    let event_cursor = bridge
        .events
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Session events unavailable",
            )
        })?
        .seq;
    Ok(Json(json!({
        "id": std::env::var("VERENU_DEV_SESSION_ID").unwrap_or_default(),
        "branch": std::env::var("VERENU_DEV_BRANCH").unwrap_or_default(),
        "commit": std::env::var("VERENU_DEV_COMMIT").unwrap_or_default(),
        "fingerprint": env!("VERENU_BUILD_FINGERPRINT"),
        "worktree": std::env::var("VERENU_DEV_WORKTREE").unwrap_or_default(),
        "transport": "rust-live",
        "platform": std::env::consts::OS,
        "shareUrl": std::env::var("VERENU_DEV_SHARE_URL").unwrap_or_default(),
        "privateHistory": std::env::var("VERENU_DEV_PRIVATE_HISTORY").as_deref() == Ok("1"),
        "maxRuns": bridge.max_runs,
        "runs": runs,
        "eventCursor": event_cursor,
        "capabilities": {
            "productionPipeline": true, "browserAudio": true,
            "hostMicrophone": bridge.host_mic, "nativeInjection": false,
            "globalHotkeys": false, "credentialWrites": false
        }
    })))
}

#[derive(Deserialize)]
struct Command {
    command: String,
    #[serde(default)]
    args: Value,
}

fn allowed(command: &str) -> bool {
    // An explicit allowlist keeps new native or secret-bearing commands from
    // becoming remotely callable when someone adds them to generate_handler!.
    matches!(
        command,
        "frontend_ready"
            | "check_for_update"
            | "get_all_settings"
            | "get_setting"
            | "check_hotkey"
            | "set_hotkey_capture"
            | "save_hotkey"
            | "get_shortcut_status"
            | "save_setting"
            | "delete_custom_provider"
            | "get_api_key_status"
            | "list_provider_models"
            | "get_provider_model_catalog"
            | "get_recent"
            | "get_history_apps"
            | "get_stats"
            | "get_insights"
            | "get_insights_pricing"
            | "get_github_commits"
            | "get_github_username_suggestion"
            | "count_old_transcriptions"
            | "get_cleanup_cache_status"
            | "clear_cleanup_cache"
            | "get_default_cleanup_prompt"
            | "lint_cleanup_prompt"
            | "test_cleanup_prompt"
            | "get_microphones"
            | "get_memory_mb"
            | "get_hardware_capabilities"
            | "local_models_supported_on_this_platform"
            | "list_local_stt_models"
            | "get_model_performance"
            | "benchmark_local_models"
            | "list_local_llm_models"
            | "get_local_transcription_state"
            | "get_local_llm_state"
            | "get_local_llm_runtime_info"
            | "download_local_stt_model"
            | "download_local_llm_model"
            | "download_local_llm_runtime"
            | "cancel_local_stt_model_download"
            | "cancel_local_llm_model_download"
            | "cancel_local_llm_runtime_download"
            | "get_contexts"
            | "get_sub_apps"
            | "create_sub_app"
            | "assign_sub_app"
            | "delete_sub_app"
            | "create_context"
            | "duplicate_context"
            | "update_context"
            | "update_context_settings"
            | "update_context_color"
            | "set_context_pinned"
            | "get_context_stats"
            | "delete_context"
            | "get_context_targets"
            | "assign_context_target"
            | "remove_context_target"
            | "get_context_websites"
            | "assign_context_website"
            | "remove_context_website"
            | "get_context_dictionary"
            | "get_context_snippets"
            | "get_dictionary"
            | "get_snippets"
            | "create_dictionary_entry"
            | "edit_dictionary_entry"
            | "remove_dictionary_entry"
            | "create_snippet"
            | "edit_snippet"
            | "remove_snippet"
            | "get_dictionary_entry_contexts"
            | "get_snippet_entry_contexts"
            | "move_dictionary_entry_to_context"
            | "move_dictionary_entry_by_term_to_context"
            | "move_snippet_entry_to_context"
            | "set_dictionary_context_assignment"
            | "set_snippet_context_assignment"
            | "get_auto_learn_status_summary"
            | "get_recent_auto_learn_activity"
            | "check_domain_exists"
            | "check_provider_status"
            | "check_verenu_api_health"
            | "check_provider_status_raw"
            | "check_global_message"
            | "sync_get_status"
            | "sync_get_diagnostics"
            | "check_connectivity"
            | "get_app_mappings"
            | "save_app_mappings"
            | "get_installed_apps"
            | "get_app_icon"
            | "get_site_icon"
            | "get_cancelled_capture"
            | "set_storage_full_simulation"
            | "get_storage_full_simulation"
            | "get_diagnostics_snapshot"
            | "get_dev_logging_enabled"
            | "set_dev_logging_enabled"
            | "get_session_logs"
            | "get_recent_logs"
            | "start_input_recording"
            | "start_setup_try_recording"
            | "stop_and_transcribe_input"
            | "stop_setup_try_recording"
            | "stop_recording"
    )
}

async fn invoke(State(bridge): State<Bridge>, Json(mut command): Json<Command>) -> ApiResult {
    if !allowed(&command.command) {
        return Err(error(
            StatusCode::FORBIDDEN,
            format!(
                "{} is unavailable in browser sessions; use native verification",
                command.command
            ),
        ));
    }
    // Browser audio sessions must not probe host devices unless host access was
    // explicitly enabled. Settings mounts can otherwise enumerate ALSA devices
    // concurrently even though recording from those devices is forbidden.
    if command.command == "get_microphones" && !bridge.host_mic {
        return Ok(Json(json!([])));
    }
    if command.command == "save_setting" {
        let key = command.args["key"].as_str().unwrap_or_default();
        use crate::data::store;
        if matches!(
            key,
            store::KEY_GROQ
                | store::KEY_OPENAI
                | store::KEY_GOOGLE
                | store::KEY_ASSEMBLYAI
                | store::KEY_OPENROUTER
                | store::KEY_XAI
                | store::SYNC_ENABLED
                | store::ANALYTICS_ENABLED
                | store::AUTO_LEARN_ENABLED
                | store::PAUSE_MEDIA_DURING_DICTATION
                | store::CLIPBOARD_PHRASE_ENABLED
                | store::MIC_MUTE_BUTTON_DICTATION
        ) {
            return Err(error(
                StatusCode::FORBIDDEN,
                "This setting is protected in dev sessions",
            ));
        }
    }
    let starts_mic = matches!(
        command.command.as_str(),
        "start_input_recording" | "start_setup_try_recording"
    );
    let stops_mic = matches!(
        command.command.as_str(),
        "stop_setup_try_recording" | "stop_and_transcribe_input" | "stop_recording"
    );
    if starts_mic || stops_mic {
        if !bridge.host_mic {
            return Err(error(
                StatusCode::FORBIDDEN,
                "Host microphone is disabled. Use browser recording or launch with --host-mic.",
            ));
        }
        if starts_mic {
            let mut held = bridge.host_mic_lock.lock().map_err(|_| {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Microphone lease state unavailable",
                )
            })?;
            if held.is_some() {
                return Err(error(
                    StatusCode::CONFLICT,
                    "Host microphone is already reserved",
                ));
            }
            let file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(
                    session_dir()
                        .parent()
                        .ok_or_else(|| {
                            error(StatusCode::CONFLICT, "Session directory has no parent")
                        })?
                        .join("host-mic.lock"),
                )
                .map_err(|_| error(StatusCode::CONFLICT, "Cannot acquire host microphone lease"))?;
            file.try_lock_exclusive().map_err(|_| {
                error(
                    StatusCode::CONFLICT,
                    "Another dev session owns the host microphone",
                )
            })?;
            bridge
                .spend_run()
                .map_err(|e| error(StatusCode::TOO_MANY_REQUESTS, e))?;
            *held = Some(file);
        }
    }
    if command.command == "test_cleanup_prompt" {
        bridge
            .spend_run()
            .map_err(|e| error(StatusCode::TOO_MANY_REQUESTS, e))?;
    }
    if command.args.is_null() {
        command.args = json!({});
    }
    let result = dispatch(&bridge.app, command).await;
    if starts_mic && result.is_err() {
        if let Ok(mut lease) = bridge.host_mic_lock.lock() {
            lease.take();
        }
    }
    if stops_mic && result.is_ok() {
        let owner = bridge.clone();
        tauri::async_runtime::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
            loop {
                let idle = owner
                    .app
                    .state::<crate::pipeline::SharedState>()
                    .lock()
                    .is_ok_and(|state| state.lifecycle.is_idle());
                if idle {
                    if let Ok(mut lease) = owner.host_mic_lock.lock() {
                        lease.take();
                    }
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    log::warn!("dev session: host microphone stayed busy after stop; retaining its lease until shutdown");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    } else if stops_mic && result.is_err() {
        // Keep the cross-process lease while capture may still be active.
        let idle = bridge
            .app
            .state::<crate::pipeline::SharedState>()
            .lock()
            .is_ok_and(|state| state.lifecycle.is_idle());
        if idle {
            if let Ok(mut lease) = bridge.host_mic_lock.lock() {
                lease.take();
            }
        }
    }
    result
        .map(Json)
        .map_err(|message| error(StatusCode::BAD_REQUEST, message))
}

async fn clone_native_handle(app: &Arc<AppHandle>) -> Result<AppHandle, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let owner = Arc::clone(app);
    app.run_on_main_thread(move || {
        let _ = tx.send(owner.as_ref().clone());
    })
    .map_err(|error| error.to_string())?;
    rx.await
        .map_err(|_| "Native handle unavailable".to_string())
}

async fn dispatch(app: &Arc<AppHandle>, command: Command) -> Result<Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let owner = Arc::clone(app);
    app.run_on_main_thread(move || {
        // Window lookup and command dispatch clone native runtime state. Keep
        // those operations on the same thread as Tauri's native IPC handler.
        let request = (|| {
            let window = owner
                .get_webview_window("main")
                .ok_or("Native command window unavailable")?;
            // Tauri's unstable custom invoke API is isolated here and exercised by the
            // bridge regression. The invoke key never crosses the browser transport.
            let request = tauri::webview::InvokeRequest {
                cmd: command.command,
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: window.url().map_err(|e| e.to_string())?,
                body: InvokeBody::Json(command.args),
                headers: Default::default(),
                invoke_key: owner.invoke_key().to_owned(),
            };
            Ok::<_, String>((window, request))
        })();
        let (window, request) = match request {
            Ok(value) => value,
            Err(error) => {
                let _ = tx.send(Err(error));
                return;
            }
        };
        window.as_ref().clone().on_message(
            request,
            Box::new(move |_, _, response, _, _| {
                let result = match response {
                    InvokeResponse::Ok(InvokeResponseBody::Json(value)) => {
                        serde_json::from_str(&value).map_err(|_| "Invalid command response".into())
                    }
                    InvokeResponse::Ok(InvokeResponseBody::Raw(_)) => {
                        Err("Binary command responses are unsupported".into())
                    }
                    InvokeResponse::Err(error) => Err(error
                        .0
                        .as_str()
                        .unwrap_or("Native command failed")
                        .to_owned()),
                };
                let _ = tx.send(result);
            }),
        );
    })
    .map_err(|error| error.to_string())?;
    tokio::time::timeout(Duration::from_secs(180), rx)
        .await
        .map_err(|_| "Native command timed out")?
        .map_err(|_| "Native command disconnected")?
}

#[derive(Deserialize)]
struct Subscription {
    event: String,
}
async fn subscribe(State(bridge): State<Bridge>, Json(body): Json<Subscription>) -> ApiResult {
    bridge
        .subscribe(&body.event)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: u64,
}
async fn events(State(bridge): State<Bridge>, Query(query): Query<Cursor>) -> ApiResult {
    let buffer = bridge.events.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Session events unavailable",
        )
    })?;
    let first = buffer
        .rows
        .front()
        .and_then(|row| row["id"].as_u64())
        .unwrap_or(buffer.seq + 1);
    Ok(Json(
        json!({"cursor": buffer.seq, "gap": query.after + 1 < first, "events": buffer.rows.iter().filter(|row| row["id"].as_u64().unwrap_or(0) > query.after).collect::<Vec<_>>()}),
    ))
}

async fn logs() -> ApiResult {
    Ok(Json(
        json!({"lines": crate::system::logger::recent(Some(200))}),
    ))
}

fn fixture_path(name: &str) -> Result<PathBuf, String> {
    if !name.ends_with(".wav")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        || name.contains("..")
    {
        return Err("Invalid fixture name".into());
    }
    Ok(session_dir().join("fixtures").join(name))
}

async fn fixtures() -> ApiResult {
    let directory = session_dir().join("fixtures");
    let names = tokio::task::spawn_blocking(move || {
        fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| fixture_path(name).is_ok())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Fixtures unavailable"))?;
    Ok(Json(json!({"fixtures": names})))
}

async fn fixture(
    RoutePath(name): RoutePath<String>,
) -> Result<([(String, String); 2], Vec<u8>), (StatusCode, Json<Value>)> {
    let path = fixture_path(&name).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| error(StatusCode::NOT_FOUND, "Fixture not found"))?;
    Ok((
        [
            ("content-type".into(), "audio/wav".into()),
            ("cache-control".into(), "no-store".into()),
        ],
        bytes,
    ))
}

fn decode_wav(bytes: &[u8]) -> Result<crate::pipeline::CapturedAudio, String> {
    let mut wav = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|_| "Audio must be a WAV file")?;
    let spec = wav.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
    {
        return Err("Use 16 kHz mono PCM16 WAV audio".into());
    }
    if wav.duration() > 16_000 * 120 {
        return Err("Test audio is limited to 120 seconds".into());
    }
    let samples: Vec<f32> = wav
        .samples::<i16>()
        .map(|sample| sample.map(|v| f32::from(v) / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid WAV samples")?;
    let duration = samples.len() as u64 * 1000 / 16_000;
    Ok(crate::pipeline::CapturedAudio::from_samples(
        samples, 16_000, duration,
    ))
}

#[derive(Deserialize)]
struct AudioTarget {
    context: Option<i64>,
    #[serde(default = "default_process")]
    process: String,
    domain: Option<String>,
}
fn default_process() -> String {
    "browser-test".into()
}

async fn audio(
    State(bridge): State<Bridge>,
    Query(target): Query<AudioTarget>,
    _headers: HeaderMap,
    bytes: Bytes,
) -> ApiResult {
    let _permit = bridge
        .audio_gate
        .try_acquire()
        .map_err(|_| error(StatusCode::CONFLICT, "A dictation test is already running"))?;
    let audio = decode_wav(&bytes).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let db = bridge.app.state::<crate::DbHandle>();
    let context = if let Some(id) = target.context {
        crate::data::db::query_context(&db, id)
    } else {
        crate::data::db::resolve_context_for_target(&db, &target.process, target.domain.as_deref())
    }
    .map_err(|_| error(StatusCode::BAD_REQUEST, "Context could not be resolved"))?;
    bridge
        .spend_run()
        .map_err(|e| error(StatusCode::TOO_MANY_REQUESTS, e))?;
    let result = Arc::new(Mutex::new(None::<Value>));
    let capture = result.clone();
    let listener = bridge.app.listen_any("verenu:transcribed", move |event| {
        if let Ok(text) = serde_json::from_str::<Value>(event.payload()) {
            if let Ok(mut captured) = capture.lock() {
                *captured = Some(text);
            }
        }
    });
    let state = bridge
        .app
        .state::<crate::pipeline::SharedState>()
        .inner()
        .clone();
    let outcome = crate::pipeline::run_provided_audio(
        clone_native_handle(&bridge.app)
            .await
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?,
        state,
        audio,
        crate::core::context::ResolvedContextIdentity::from_context(&context),
        target.process,
        target.domain,
    )
    .await;
    bridge.app.unlisten(listener);
    outcome.map_err(|e| error(StatusCode::CONFLICT, e))?;
    let captured = result
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Dictation result unavailable",
            )
        })?
        .clone();
    let text = captured.ok_or_else(|| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Dictation failed or audio was rejected. Inspect session events and redacted logs.",
        )
    })?;
    Ok(Json(
        json!({"text": text, "delivery": "browser-field", "pipeline": "production", "providers": "live"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dev_session_snapshot_preserves_customization_without_touching_source_history() {
        let root = std::env::temp_dir().join(format!("verenu-seed-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.db");
        let copy = root.join("copy.db");
        let db = crate::data::db::open(&source).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "INSERT INTO contexts (name) VALUES ('Synthetic context')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO transcriptions (raw_text, clean_text) VALUES ('Synthetic source text', 'Synthetic source text')", []).unwrap();
            conn.execute("INSERT INTO context_sub_apps (uuid, label, executable, title_pattern, match_mode) VALUES ('synthetic-sub-app', 'Synthetic sub-app', 'fixture', 'Synthetic pattern', 'contains')", []).unwrap();
            conn.backup(rusqlite::DatabaseName::Main, &copy, None)
                .unwrap();
        }
        scrub_history(&copy).unwrap();
        let copied = rusqlite::Connection::open(&copy).unwrap();
        assert_eq!(copied.query_row("SELECT COUNT(*) FROM context_sub_apps WHERE label='Synthetic sub-app'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(
            copied
                .query_row("SELECT COUNT(*) FROM transcriptions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            copied
                .query_row(
                    "SELECT COUNT(*) FROM contexts WHERE name='Synthetic context'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM transcriptions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        drop(copied);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn dev_session_secrets_and_native_actions_are_not_exposed() {
        for command in [
            "save_api_key",
            "delete_api_key",
            "export_data",
            "import_data",
            "restart_app",
            "stop_handless_mode",
            "plugin:shell|open",
            "android_provide_credential",
        ] {
            assert!(!allowed(command), "{command}");
        }
        assert!(allowed("get_contexts"));
        assert!(allowed("get_sub_apps"));
        assert!(allowed("create_sub_app"));
        assert!(!allowed("take_pending_sub_app_capture"));
        let mut settings =
            json!({crate::data::store::KEY_GROQ: "synthetic-key", "setup_complete": true});
        sanitize_settings(&mut settings).unwrap();
        assert!(settings.get(crate::data::store::KEY_GROQ).is_none());
        assert_eq!(settings[crate::data::store::SYNC_ENABLED], false);
    }
    #[test]
    fn dev_session_events_are_bounded_and_fixture_paths_cannot_escape() {
        let mut events = EventBuffer::default();
        for _ in 0..600 {
            events.push("verenu:test", json!(null));
        }
        assert_eq!(events.rows.len(), MAX_EVENTS);
        assert_eq!(events.rows.front().unwrap()["id"], 89);
        for name in ["../private.wav", "a/b.wav", "x\\y.wav", "settings.json"] {
            assert!(fixture_path(name).is_err());
        }
    }
    #[test]
    fn dev_session_audio_rejects_bad_formats_and_accepts_bounded_pcm() {
        assert!(decode_wav(b"invalid").is_err());
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut bytes,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..16_000 {
                writer.write_sample(0i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        assert_eq!(decode_wav(bytes.get_ref()).unwrap().duration_ms, 1000);
    }
}
