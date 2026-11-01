//! Restricted startup for a replacement process. The old app remains alive.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::Manager;

struct ProbationPause(Option<crate::linux_activity::Pause>);

impl ProbationPause {
    fn guard(&self) -> &crate::linux_activity::Pause {
        self.0.as_ref().expect("probation remains paused")
    }
}

impl Drop for ProbationPause {
    fn drop(&mut self) {
        // If the coordinator panics, no child service may resume while the
        // parent is recovering. The supervising process stops this child.
        if let Some(pause) = self.0.take() {
            std::mem::forget(pause);
        }
    }
}

#[tauri::command]
fn get_api_key_status(
    availability: tauri::State<'_, crate::linux_handoff_transport::KeyAvailability>,
) -> crate::linux_handoff_transport::KeyAvailability {
    availability.inner().clone()
}

fn prepare_saved_shortcut(app: &tauri::AppHandle) -> Result<(), String> {
    let settings = app.state::<crate::data::store::SettingsHandle>();
    if let Some(value) = settings.get(crate::data::store::HOTKEY) {
        let codes: Vec<String> = serde_json::from_value(value)
            .map_err(|_| "The saved shortcut could not be read".to_owned())?;
        let normalized = crate::core::hotkey::normalize_codes(&codes)?;
        let mapped = crate::core::hotkey::mapped_codes(&normalized)?;
        crate::core::hotkey::update_keys(&mapped)?;
    }
    crate::commands::apply_linux_control_keys(app)
}

fn verify_cleanup_cache_setting(
    connection: &rusqlite::Connection,
    enabled: bool,
) -> Result<(), String> {
    let stored = crate::data::db::cleanup_cache_enabled_conn(connection)
        .map_err(|error| format!("Could not check cleanup cache state: {error}"))?;
    if stored != enabled {
        return Err("Cleanup cache settings require a manual restart to reconcile.".into());
    }
    Ok(())
}

fn fail_paused_probation(services: crate::linux_handoff_transport::ChildServices, reason: String) -> ! {
    let _ = services.fail(reason);
    // Keep the child's activity gate paused until every child thread stops.
    // The parent supervises this process group and resumes only after it exits.
    std::process::exit(1)
}

pub(crate) fn run(child: crate::linux_handoff_transport::Child) {
    crate::system::logger::init_for_handoff();
    let startup = match child.begin(crate::data::db::HANDOFF_SCHEMA_VERSION) {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("Could not start update replacement: {error}");
            return;
        }
    };
    let key_availability = startup.key_availability().clone();
    let parent_unique_name = startup.parent_unique_name().map(str::to_owned);
    let Some(callback_path) = startup.callback_queue_path() else {
        let _ = startup.fail("Restart callback queue is missing".into());
        return;
    };
    let callback_queue = match crate::linux_handoff_callbacks::CallbackQueue::open(callback_path) {
        Ok(queue) => Arc::new(queue),
        Err(error) => {
            let _ = startup.fail(format!("Could not open restart callback queue: {error}"));
            return;
        }
    };
    let db = match crate::data::db::open_for_handoff(crate::app_db_path()) {
        Ok(db) => db,
        Err(error) => {
            let _ = startup.fail(format!("Could not open the database read-only: {error}"));
            return;
        }
    };
    let db_version = match crate::data::db::handoff_data_version(&db) {
        Ok(version) => version,
        Err(error) => {
            let _ = startup.fail(format!("Could not check the database for restart: {error}"));
            return;
        }
    };
    let settings = match crate::data::store::SettingsHandle::open_for_handoff(
        crate::app_data_dir().join("settings.json"),
    ) {
        Ok(settings) => settings,
        Err(error) => {
            let _ = startup.fail(error);
            return;
        }
    };
    crate::core::hyprland::initialize_app_identity();
    crate::configure_hyprland_webkit_renderer();
    let readiness = crate::FrontendReadiness::default();
    let normal_commands = Arc::new(AtomicBool::new(false));
    let command_mode = normal_commands.clone();
    let window_mode = normal_commands.clone();
    let run_mode = normal_commands.clone();
    let normal_handler = crate::app_invoke_handler();
    let probation_handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            crate::commands::frontend_ready,
            crate::commands::frontend_settings_generation,
            crate::commands::frontend_settings_ready,
            crate::commands::frontend_home_ready,
            crate::commands::frontend_pill_generation,
            crate::commands::frontend_pill_ready,
            crate::system::windows_titlebar::get_native_titlebar_metrics,
            crate::commands::get_setting,
            get_api_key_status,
            crate::api::apple_intelligence::get_apple_intelligence_availability,
            crate::commands::get_contexts,
            crate::commands::get_context_targets,
            crate::commands::get_context_websites,
            crate::commands::get_sub_apps,
            crate::commands::list_local_stt_models,
            crate::commands::list_local_llm_models,
            crate::commands::get_local_llm_runtime_info,
        ]);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .append_invoke_initialization_script(
            "Object.defineProperty(window, '__VERENU_RESTART_PROBE__', { value: true });",
        )
        .manage(db)
        .manage(settings)
        .manage(key_availability)
        .manage(crate::new_shared_state())
        .manage(Arc::new(crate::linux_activity::ActivityGate::default()))
        .manage(crate::local_stt::LocalTranscriptionManager::new())
        .manage(crate::local_llm::LocalLlmManager::new())
        .manage(readiness.clone())
        .manage(normal_commands)
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                if let Some(readiness) =
                    webview.app_handle().try_state::<crate::FrontendReadiness>()
                {
                    match webview.label() {
                        "main" => readiness.main_page_started(),
                        "pill" => readiness.pill_page_started(),
                        _ => {}
                    }
                }
            }
        })
        .on_window_event(move |window, event| {
            if window_mode.load(Ordering::Acquire) {
                crate::handle_window_event(window, event);
            }
        })
        .invoke_handler(move |invoke| {
            if command_mode.load(Ordering::Acquire) {
                normal_handler(invoke)
            } else {
                probation_handler(invoke)
            }
        })
        .setup(move |app| {
            crate::system::logger::attach_app(app.handle());
            app.manage(crate::sync::SyncManager::new_unstarted(
                app.handle().clone(),
                app.state::<crate::DbHandle>().inner().clone(),
            ));
            crate::pipeline::initialize_pill(app.handle());
            if app.get_webview_window("main").is_none() || app.get_webview_window("pill").is_none()
            {
                let _ = startup.fail("Could not create replacement windows".into());
                return Err(std::io::Error::other("replacement windows unavailable").into());
            }
            let window = app.get_webview_window("main").expect("main window was checked");
            crate::system::linux_webview::configure_window(&window);
            if let Err(error) = crate::system::linux_titlebar::enable(&window) {
                let _ = startup.fail(format!("Could not configure replacement window: {error}"));
                return Err(std::io::Error::other("replacement window setup failed").into());
            }
            let frontends = startup.backend_ready().map_err(std::io::Error::other)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
                let mut basics_ready = false;
                while Instant::now() < deadline {
                    if readiness.main.load(Ordering::Acquire)
                        && readiness.current_pill_ready()
                        && readiness.main_and_home_ready()
                    {
                        basics_ready = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                if !basics_ready {
                    let _ = frontends.fail("Replacement frontend did not initialize".into());
                    handle.exit(1);
                    return;
                }
                let gate = handle
                    .state::<Arc<crate::linux_activity::ActivityGate>>()
                    .inner()
                    .clone();
                let child_pause = tauri::async_runtime::block_on(async {
                    tokio::time::timeout(Duration::from_secs(10), gate.pause()).await
                });
                let child_pause = match child_pause {
                    Ok(Ok(pause)) => ProbationPause(Some(pause)),
                    _ => {
                        let _ = frontends.fail("Replacement is still busy starting".into());
                        handle.exit(1);
                        return;
                    }
                };
                let activation = match frontends.ready() {
                    Ok(activation) => activation,
                    Err(_) => std::process::exit(1),
                };
                let Some(parent_unique_name) = parent_unique_name else {
                    let _ = activation.fail("Parent single-instance identity is missing".into());
                    std::process::exit(1);
                };
                let (sent, received) = std::sync::mpsc::sync_channel(1);
                let owner = handle.clone();
                let scheduled = handle.run_on_main_thread(move || {
                    let _ = sent.send(owner.plugin(
                        crate::linux_single_instance::replacement_plugin(parent_unique_name, callback_queue),
                    ));
                });
                let acquired = scheduled.is_ok()
                    && matches!(received.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
                let ready = readiness.main.load(Ordering::Acquire)
                    && readiness.current_pill_ready()
                    && readiness.main_and_home_ready();
                if acquired && ready {
                    if let Ok(services) = activation.ownership_acquired() {
                        let current_version = crate::data::db::handoff_data_version(
                            handle.state::<crate::DbHandle>().inner(),
                        );
                        let database_error = match current_version {
                            Ok(version) if version == db_version => None,
                            Ok(_) => Some(
                                "Database changed while preparing the update. Try again.".into(),
                            ),
                            Err(error) => {
                                Some(format!("Could not check the database for restart: {error}"))
                            }
                        };
                        if let Some(error) = database_error {
                            fail_paused_probation(services, error);
                        }
                        if let Err(error) = handle
                            .state::<crate::data::store::SettingsHandle>()
                            .validate_handoff_snapshot()
                        {
                            fail_paused_probation(services, error);
                        }
                        let prepared_db = match crate::data::db::probe_writable_for_handoff(
                            crate::app_db_path(),
                        ) {
                            Ok(db) => db,
                            Err(error) => {
                                fail_paused_probation(services, format!(
                                    "Replacement cannot prepare the database: {error}"
                                ));
                            }
                        };
                        let (sent, received) = std::sync::mpsc::sync_channel(1);
                        let owner = handle.clone();
                        let scheduled = handle.run_on_main_thread(move || {
                            let _ = sent.send(crate::app_tray::setup_probation_tray(&owner));
                        });
                        let tray_setup = scheduled.is_ok()
                            && matches!(received.recv_timeout(Duration::from_secs(10)), Ok(Ok(())));
                        let tray_started = tray_setup
                            && tauri::async_runtime::block_on(
                                crate::app_tray::resume_after_handoff(&handle),
                            )
                            .is_ok();
                        if !tray_started {
                            fail_paused_probation(services, "Replacement tray could not start".into());
                        }
                        let prepared_agent = match crate::agent_api::prepare_for_handoff() {
                            Ok(agent) => agent,
                            Err(error) => {
                                fail_paused_probation(services, format!("Replacement agent service could not prepare: {error}"));
                            }
                        };
                        let staged_agent = match tauri::async_runtime::block_on(
                            prepared_agent.stage_for_handoff(handle.clone()),
                        ) {
                            Ok(agent) => agent,
                            Err(error) => {
                                fail_paused_probation(services, format!("Replacement agent service could not start safely: {error}"));
                            }
                        };
                        let analytics_enabled = handle.state::<crate::data::store::SettingsHandle>()
                            .get(crate::data::store::ANALYTICS_ENABLED)
                            .and_then(|value| value.as_bool()).unwrap_or(true);
                        let analytics = match crate::analytics::Analytics::prepare_for_handoff(analytics_enabled, crate::app_data_dir()) {
                            Ok(analytics) => analytics,
                            Err(error) => fail_paused_probation(services, format!("Replacement analytics could not prepare: {error}")),
                        };
                        if !handle.manage(analytics) {
                            fail_paused_probation(services, "Replacement analytics was already initialized".into());
                        }
                        if let Err(error) = prepare_saved_shortcut(&handle) {
                            fail_paused_probation(services, format!("Replacement shortcut settings are unavailable: {error}"));
                        }
                        let (sent, received) = std::sync::mpsc::sync_channel(1);
                        let hotkey_app = handle.clone();
                        let scheduled = handle.run_on_main_thread(move || {
                            let shared = hotkey_app.state::<crate::pipeline::SharedState>().inner().clone();
                            crate::app_hotkey::setup_hotkey(&hotkey_app, shared);
                            let _ = sent.send(());
                        });
                        if scheduled.is_err() || received.recv_timeout(Duration::from_secs(10)).is_err() {
                            fail_paused_probation(services, "Replacement shortcuts could not start".into());
                        }
                        if let Err(error) = tauri::async_runtime::block_on(
                            crate::core::hotkey::wait_for_initial_shortcut_registration(),
                        ) {
                            fail_paused_probation(services, format!("Replacement shortcuts could not start: {error}"));
                        }
                        let prepared_sync = match handle.state::<crate::sync::SyncManager>().prepare_for_handoff() {
                            Ok(sync) => sync,
                            Err(error) => {
                                fail_paused_probation(services, format!("Replacement sync could not prepare: {error}"));
                            }
                        };
                        if let Err(error) = tauri::async_runtime::block_on(prepared_sync.verify_transport()) {
                            fail_paused_probation(services, format!("Replacement sync transport could not prepare: {error}"));
                        }
                        let prepared_microphone = match crate::media::mic_mute_trigger::prepare_for_handoff(
                            &handle, handle.state::<crate::pipeline::SharedState>().inner().clone(),
                        ) {
                            Ok(microphone) => microphone,
                            Err(error) => {
                                fail_paused_probation(services, format!("Replacement microphone watcher could not prepare: {error}"));
                            }
                        };
                        let active_microphone = match prepared_microphone.activate_while_paused(child_pause.guard()) {
                            Ok(microphone) => microphone,
                            Err(error) => fail_paused_probation(services, format!("Replacement microphone watcher could not start: {error}")),
                        };
                        let storage_maintenance = crate::start_storage_maintenance(
                            handle.clone(), handle.state::<crate::DbHandle>().inner().clone(),
                            handle.state::<crate::data::store::SettingsHandle>().inner().clone(),
                        );
                        let runtime_maintenance = crate::start_runtime_maintenance(
                            handle.clone(), handle.state::<crate::pipeline::SharedState>().inner().clone(),
                            handle.state::<crate::local_stt::LocalTranscriptionManager>().inner().clone(),
                            handle.state::<crate::local_llm::LocalLlmManager>().inner().clone(),
                        );
                        let diagnostic_worker = match crate::system::diagnostic_journal::start(
                            handle.state::<crate::DbHandle>().inner().clone(), gate.clone(),
                        ) {
                            Some(worker) => worker,
                            None => fail_paused_probation(services, "Replacement diagnostics was already initialized".into()),
                        };
                        #[cfg(all(feature = "native-testing", debug_assertions))]
                        if std::env::var_os("VERENU_HANDOFF_TEST_STOP_MAINTENANCE").is_some() {
                            runtime_maintenance.abort();
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while !runtime_maintenance.inner().is_finished() && Instant::now() < deadline {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        }
                        let db = handle.state::<crate::DbHandle>();
                        let locked_db = match prepared_db.lock_for_activation(db.inner()) {
                            Ok(db) => db,
                            Err(error) => {
                                fail_paused_probation(services, format!("Replacement database could not prepare activation: {error}"));
                            }
                        };
                        let cache_enabled = handle.state::<crate::data::store::SettingsHandle>()
                            .get(crate::data::store::CLEANUP_CACHE_ENABLED)
                            .and_then(|value| value.as_bool())
                            .unwrap_or(true);
                        if let Err(error) = verify_cleanup_cache_setting(locked_db.connection(), cache_enabled) {
                            fail_paused_probation(services, error);
                        }
                        let sync_change = match prepared_sync.prepare_database(locked_db.connection()) {
                            Ok(change) => change,
                            Err(error) => fail_paused_probation(services, format!("Replacement sync database could not prepare: {error}")),
                        };
                        let staged_sync = match tauri::async_runtime::block_on(async {
                            handle.state::<crate::sync::SyncManager>().start_while_paused(prepared_sync, child_pause.guard(), sync_change)
                        }) {
                            Ok(sync) => sync,
                            Err(error) => fail_paused_probation(services, format!("Replacement sync listener could not start: {error}")),
                        };
                        if let Err(error) = tauri::async_runtime::block_on(staged_sync.verify()) {
                            fail_paused_probation(services, format!("Replacement sync listener could not verify: {error}"));
                        }
                        let tray_check = tauri::async_runtime::block_on(async {
                            tokio::time::timeout(Duration::from_secs(5), crate::app_tray::verify_probation_tray(&handle)).await
                        });
                        if !matches!(tray_check, Ok(Ok(()))) {
                            fail_paused_probation(services, "Replacement tray became unavailable during preparation".into());
                        }
                        if let Err(error) = tauri::async_runtime::block_on(staged_agent.verify()) {
                            fail_paused_probation(services, format!("Replacement agent service became unavailable: {error}"));
                        }
                        if let Err(error) = active_microphone.verify() {
                            fail_paused_probation(services, format!("Replacement microphone watcher became unavailable: {error}"));
                        }
                        if let Err(error) = tauri::async_runtime::block_on(staged_sync.verify()) {
                            fail_paused_probation(services, format!("Replacement sync listener became unavailable: {error}"));
                        }
                        if diagnostic_worker.is_finished() {
                            fail_paused_probation(services, "Replacement diagnostics stopped during preparation".into());
                        }
                        if storage_maintenance.inner().is_finished() || runtime_maintenance.inner().is_finished() {
                            fail_paused_probation(services, "Replacement maintenance stopped during preparation".into());
                        }
                        let published_agent = match staged_agent.publish_while_paused(&handle, child_pause.guard()) {
                            Ok(agent) => agent,
                            Err(error) => fail_paused_probation(services, format!("Replacement agent discovery could not be prepared: {error}")),
                        };
                        if let Err(error) = tauri::async_runtime::block_on(published_agent.verify()) {
                            fail_paused_probation(services, format!("Replacement agent discovery could not be verified: {error}"));
                        }
                        let prepared = match services.prepared() {
                            Ok(prepared) => prepared,
                            Err(_) => std::process::exit(1),
                        };
                        let services = match prepared.wait_for_activation() {
                            Ok(services) => services,
                            Err(_) => std::process::exit(1),
                        };
                        if !readiness.main.load(Ordering::Acquire)
                            || !readiness.current_pill_ready()
                            || !readiness.main_and_home_ready()
                        {
                            fail_paused_probation(services, "Replacement frontend became unavailable before activation".into());
                        }
                        if let Err(error) = active_microphone.verify() {
                            fail_paused_probation(services, format!("Replacement microphone watcher became unavailable before activation: {error}"));
                        }
                        if let Err(error) = tauri::async_runtime::block_on(published_agent.verify()) {
                            fail_paused_probation(services, format!("Replacement agent service became unavailable before activation: {error}"));
                        }
                        if let Err(error) = tauri::async_runtime::block_on(staged_sync.verify()) {
                            fail_paused_probation(services, format!("Replacement sync listener became unavailable before activation: {error}"));
                        }
                        if diagnostic_worker.is_finished() {
                            fail_paused_probation(services, "Replacement diagnostics stopped before activation".into());
                        }
                        if storage_maintenance.inner().is_finished() || runtime_maintenance.inner().is_finished() {
                            fail_paused_probation(services, "Replacement maintenance stopped before activation".into());
                        }
                        let tray_check = tauri::async_runtime::block_on(async {
                            tokio::time::timeout(Duration::from_secs(5), crate::app_tray::verify_probation_tray(&handle)).await
                        });
                        if !matches!(tray_check, Ok(Ok(()))) {
                            fail_paused_probation(services, "Replacement tray became unavailable before activation".into());
                        }
                        fail_paused_probation(services, "Replacement services are not yet verified".into());
                    }
                } else {
                    let reason = if acquired {
                        "Replacement frontend became unavailable during activation"
                    } else {
                        "Replacement could not acquire single-instance ownership"
                    };
                    let _ = activation.fail(reason.into());
                }
                std::process::exit(1);
            });
            Ok(())
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(move |app, event| {
            if run_mode.load(Ordering::Acquire) {
                crate::handle_run_event(app, event);
            }
        }),
        Err(error) => eprintln!("Could not initialize update replacement: {error}"),
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn coordinator_panic_keeps_probation_paused() {
        let gate = std::sync::Arc::new(crate::linux_activity::ActivityGate::default());
        let pause = gate.pause().await.unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _pause = super::ProbationPause(Some(pause));
            panic!("synthetic restart failure");
        }));
        assert!(result.is_err());
        assert!(gate.try_enter().is_none());
    }

    #[test]
    fn rejects_cleanup_cache_state_that_needs_startup_reconciliation() {
        let db = crate::data::db::open(":memory:").unwrap();
        crate::data::db::cleanup_cache_set_enabled(&db, false).unwrap();
        let conn = db.lock().unwrap();
        assert!(super::verify_cleanup_cache_setting(&conn, false).is_ok());
        assert!(super::verify_cleanup_cache_setting(&conn, true).is_err());
    }
}
