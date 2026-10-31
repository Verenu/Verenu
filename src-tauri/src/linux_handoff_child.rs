//! Restricted startup for a replacement process. The old app remains alive.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::Manager;

#[tauri::command]
fn get_api_key_status(
    availability: tauri::State<'_, crate::linux_handoff_transport::KeyAvailability>,
) -> crate::linux_handoff_transport::KeyAvailability {
    availability.inner().clone()
}

pub(crate) fn run(child: crate::linux_handoff_transport::Child) {
    let startup = match child.begin(crate::data::db::HANDOFF_SCHEMA_VERSION) {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("Could not start update replacement: {error}");
            return;
        }
    };
    let key_availability = startup.key_availability().clone();
    let parent_unique_name = startup.parent_unique_name().map(str::to_owned);
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
    let normal_handler = crate::app_invoke_handler();
    let probation_handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            crate::commands::frontend_ready,
            crate::commands::frontend_settings_generation,
            crate::commands::frontend_settings_ready,
            crate::commands::frontend_home_ready,
            crate::commands::frontend_pill_generation,
            crate::commands::frontend_pill_ready,
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
        .append_invoke_initialization_script(
            "Object.defineProperty(window, '__VERENU_RESTART_PROBE__', { value: true });",
        )
        .manage(db)
        .manage(settings)
        .manage(key_availability)
        .manage(crate::new_shared_state())
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
        .invoke_handler(move |invoke| {
            if command_mode.load(Ordering::Acquire) {
                normal_handler(invoke)
            } else {
                probation_handler(invoke)
            }
        })
        .setup(move |app| {
            crate::pipeline::initialize_pill(app.handle());
            if app.get_webview_window("main").is_none() || app.get_webview_window("pill").is_none()
            {
                let _ = startup.fail("Could not create replacement windows".into());
                return Err(std::io::Error::other("replacement windows unavailable").into());
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
                let activation = match frontends.ready() {
                    Ok(activation) => activation,
                    Err(_) => {
                        handle.exit(1);
                        return;
                    }
                };
                let Some(parent_unique_name) = parent_unique_name else {
                    let _ = activation.fail("Parent single-instance identity is missing".into());
                    handle.exit(1);
                    return;
                };
                let (sent, received) = std::sync::mpsc::sync_channel(1);
                let owner = handle.clone();
                let scheduled = handle.run_on_main_thread(move || {
                    let _ = sent.send(owner.plugin(
                        crate::linux_single_instance::replacement_plugin(parent_unique_name),
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
                            let _ = services.fail(error);
                            handle.exit(1);
                            return;
                        }
                        if let Err(error) = handle
                            .state::<crate::data::store::SettingsHandle>()
                            .validate_handoff_snapshot()
                        {
                            let _ = services.fail(error);
                            handle.exit(1);
                            return;
                        }
                        let _prepared_db = match crate::data::db::probe_writable_for_handoff(
                            crate::app_db_path(),
                        ) {
                            Ok(db) => db,
                            Err(error) => {
                                let _ = services.fail(format!(
                                    "Replacement cannot prepare the database: {error}"
                                ));
                                handle.exit(1);
                                return;
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
                        let reason = if tray_started {
                            "Replacement services are not yet verified"
                        } else {
                            "Replacement tray could not start"
                        };
                        let _ = services.fail(reason.into());
                    }
                } else {
                    let reason = if acquired {
                        "Replacement frontend became unavailable during activation"
                    } else {
                        "Replacement could not acquire single-instance ownership"
                    };
                    let _ = activation.fail(reason.into());
                }
                handle.exit(1);
            });
            Ok(())
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(error) => eprintln!("Could not initialize update replacement: {error}"),
    }
}
