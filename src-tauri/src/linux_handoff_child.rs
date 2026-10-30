//! Restricted startup for a replacement process. The old app remains alive.
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::Manager;

pub(crate) fn run(child: crate::linux_handoff_transport::Child) {
    let startup = match child.begin(crate::data::db::HANDOFF_SCHEMA_VERSION) {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("Could not start update replacement: {error}");
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
    let app = tauri::Builder::default()
        .manage(db)
        .manage(settings)
        .manage(crate::new_shared_state())
        .manage(crate::local_stt::LocalTranscriptionManager::new())
        .manage(crate::local_llm::LocalLlmManager::new())
        .manage(readiness.clone())
        .on_page_load(|webview, payload| {
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Started
            {
                if let Some(readiness) =
                    webview.app_handle().try_state::<crate::FrontendReadiness>()
                {
                    readiness.main_page_started();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::frontend_ready,
            crate::commands::frontend_settings_generation,
            crate::commands::frontend_settings_ready,
            crate::commands::get_setting,
            crate::commands::get_contexts,
            crate::commands::get_context_targets,
            crate::commands::get_context_websites,
            crate::commands::get_sub_apps,
            crate::commands::list_local_stt_models,
            crate::commands::list_local_llm_models,
            crate::commands::get_local_llm_runtime_info,
        ])
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
                        && readiness.pill.load(Ordering::Acquire)
                        && readiness.main_settings_ready()
                    {
                        basics_ready = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                // Settings and mount acknowledgements alone do not establish
                // that the home view and activation are ready for handoff.
                let reason = if basics_ready {
                    "Full replacement startup is not yet verified"
                } else {
                    "Replacement frontend did not initialize"
                };
                let _ = frontends.fail(reason.into());
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
