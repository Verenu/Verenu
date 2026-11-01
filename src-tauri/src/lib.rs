#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(test)]
#[path = "../build_support/posthog.rs"]
mod posthog_build_support;

#[allow(dead_code)]
mod analytics;
#[cfg(desktop)]
mod agent_api;
#[cfg(desktop)]
mod agent_discovery;
mod android;
mod api;
mod app_hotkey;
mod app_setup;
#[cfg(desktop)]
mod app_tray;
#[cfg(not(desktop))]
mod app_tray {
    use tauri::AppHandle;

    pub(crate) fn apply_runtime_icons(_app: &AppHandle, _theme: Option<tauri::Theme>) {}

    pub(crate) fn setting_updates_runtime_icons(_key: &str) -> bool {
        false
    }

    pub(crate) fn appearance_mode(_app: &AppHandle) -> Option<String> {
        None
    }
}
mod commands;
mod core;
mod data;
#[cfg(all(feature = "dev-session", debug_assertions, desktop))]
mod dev_session;
mod local_llm;
mod local_stt;
mod media;
mod model_performance;
mod pipeline;
#[cfg(target_os = "windows")]
mod single_instance;
mod sync;
mod system;
#[cfg(target_os = "linux")]
pub fn run_internal_output_guardian() -> bool {
    system::output_duck::internal_entry()
}
#[cfg(target_os = "linux")]
pub fn recover_output_volume() {
    system::output_duck::start_recovery();
}
#[cfg(any(test, debug_assertions))]
mod testing;

use crate::core::window_geometry::WindowTarget;
use crate::data::db;
use crate::pipeline::{AppState, SharedState};

use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

pub type DbHandle = db::Db;

pub(crate) fn is_dev_session() -> bool {
    #[cfg(all(feature = "dev-session", debug_assertions, desktop))]
    return dev_session::enabled();
    #[cfg(not(all(feature = "dev-session", debug_assertions, desktop)))]
    false
}

#[cfg(target_os = "macos")]
pub(crate) use app_setup::hide_main_window;
pub(crate) use app_setup::{
    app_data_dir, app_db_path, fatal_startup_error, show_main_window, start_frontend_watchdog,
    FrontendReadiness,
};
#[cfg(target_os = "windows")]
pub(crate) use app_setup::{cleanup_update_helper_if_requested, wait_for_relaunch_parent_exit};
pub(crate) use app_tray::apply_runtime_icons;

fn start_storage_maintenance(
    app: tauri::AppHandle,
    db: DbHandle,
    settings: crate::data::store::SettingsHandle,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
            let retention_days = settings
                .get(crate::data::store::HISTORY_RETENTION)
                .and_then(|value| {
                    value
                        .as_str()
                        .and_then(crate::data::store::history_retention_days)
                });
            let db_for_work = db.clone();
            let settings_for_work = settings.clone();
            let app_for_work = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Err(err) = db::cleanup_cache_prune_expired(&db_for_work) {
                    log::warn!("maintenance: cleanup cache prune failed: {err}");
                }
                if let Err(err) = db::prune_auto_learn_retention(&db_for_work) {
                    log::warn!("maintenance: auto-learn retention failed: {err}");
                }
                if let Err(err) = db::prune_pending_corrections(
                    &db_for_work,
                    crate::api::auto_learn::PENDING_RETENTION_DAYS,
                ) {
                    log::warn!("maintenance: pending-correction retention failed: {err}");
                }
                if let Some(days) = retention_days {
                    match db::prune_transcriptions_for_retention(
                        &db_for_work,
                        &settings_for_work,
                        days,
                    ) {
                        Ok(deleted) if deleted > 0 => {
                            let _ = app_for_work.emit("verenu:history-pruned", ());
                        }
                        Ok(_) => {}
                        Err(err) => log::warn!("maintenance: history retention failed: {err}"),
                    }
                }
                if let Ok(conn) = db_for_work.lock() {
                    if let Err(err) = crate::sync::store::compact_log(&conn) {
                        log::warn!("maintenance: sync-log compaction failed: {err}");
                    }
                }
                if let Err(err) = db::sqlite_disk_maintenance(&db_for_work) {
                    log::warn!("maintenance: SQLite disk reclamation failed: {err}");
                }
            })
            .await;
        }
    });
}

#[cfg(target_os = "android")]
fn init_android_audio_context() -> Result<(), Box<dyn std::error::Error>> {
    use jni::objects::JObject;
    use jni::JavaVM;

    let context = tauri::tao::platform::android::prelude::main_android_context()
        .ok_or_else(|| std::io::Error::other("Android activity context unavailable"))?;
    let vm = unsafe { JavaVM::from_raw(context.java_vm.cast())? };
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(context.context_jobject.cast()) };
    let application = env
        .call_method(
            &activity,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[],
        )?
        .l()?;
    let global = env.new_global_ref(application)?;
    let raw = global.as_obj().as_raw();
    std::mem::forget(global);
    unsafe { ndk_context::initialize_android_context(context.java_vm, raw.cast()) };
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    crate::core::hyprland::initialize_app_identity();
    #[cfg(all(feature = "dev-session", debug_assertions, desktop))]
    dev_session::prepare().expect("failed to prepare isolated dev session");
    #[cfg(target_os = "linux")]
    configure_hyprland_webkit_renderer();

    #[cfg(target_os = "windows")]
    {
        cleanup_update_helper_if_requested();
        if crate::commands::run_update_helper_if_requested() {
            return;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = crate::system::mac_app::set_process_name("Verenu");
    }

    #[cfg(target_os = "windows")]
    wait_for_relaunch_parent_exit();

    #[cfg(target_os = "windows")]
    let single_instance = (!is_dev_session()).then(crate::single_instance::acquire);

    let shared: SharedState = Arc::new(Mutex::new(AppState {
        lifecycle: pipeline::DictationLifecycle::Idle,
        analytics_run_id: None,
        target: WindowTarget::default(),
        pill_placement: None,
        pill_placement_stale: false,
        pill_width_points: pipeline::DEFAULT_PILL_WIDTH_POINTS,
        pill_height_points: pipeline::DEFAULT_PILL_HEIGHT_POINTS,
        recording_context: None,
        retry_capture: None,
        cancelled_capture: None,
        paste_failure: None,
        adaptive_sensitivity_level: 0,
        sensitivity_retry_available: false,
        sensitivity_rejected_at: None,
        failover_session_id: None,
        failover_reuse_id: false,
        failover_started_at_unix: 0,
    }));

    crate::system::logger::init_early();
    pipeline::failover::restore_into_state(&shared);
    let local_cleanup_manager = crate::local_llm::LocalLlmManager::new();
    let local_transcription_manager = crate::local_stt::LocalTranscriptionManager::new();
    let frontend_readiness = FrontendReadiness::default();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init());
    #[cfg(all(feature = "native-testing", debug_assertions, desktop))]
    {
        builder = builder
            .plugin(tauri_plugin_wdio::init())
            .plugin(tauri_plugin_wdio_webdriver::init());
    }
    #[cfg(target_os = "android")]
    {
        builder = builder.plugin(crate::android::permissions_plugin::init());
        builder = builder.plugin(crate::android::security_plugin::init());
        builder = builder.plugin(crate::android::updater::init());
        builder = builder.append_invoke_initialization_script(
            "Object.defineProperty(window, '__VERENU_ANDROID__', { value: true, configurable: false });",
        );
    }
    #[cfg(desktop)]
    if !is_dev_session() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            #[cfg(target_os = "linux")]
            if _argv.iter().any(|arg| arg == "--verenu-hotkey-release") {
                crate::core::hotkey::notify_release();
                return;
            }
            #[cfg(target_os = "linux")]
            if _argv.iter().any(|arg| arg == "--verenu-hotkey-handsfree") {
                crate::core::hotkey::notify_handless();
                return;
            }
            #[cfg(target_os = "linux")]
            if _argv.iter().any(|arg| arg == "--verenu-capture-sub-app") {
                crate::core::hotkey::notify_capture_sub_app();
                return;
            }
            show_main_window(app);
        }));
    }

    builder = builder
        .manage(shared.clone())
        .manage(local_cleanup_manager.clone())
        .manage(local_transcription_manager.clone())
        .manage(frontend_readiness.clone());

    #[cfg(target_os = "windows")]
    if let Some(single_instance) = single_instance {
        builder = builder.manage(single_instance);
    }

    builder
        .setup(move |app| {
            #[cfg(target_os = "linux")]
            if std::env::args().any(|arg| {
                arg == "--verenu-hotkey-release"
                    || arg == "--verenu-hotkey-handsfree"
                    || arg == "--verenu-capture-sub-app"
            }) {
                app.handle().exit(0);
                return Ok(());
            }
            #[cfg(target_os = "windows")]
            if !is_dev_session() {
                crate::single_instance::listen_for_takeover(app.handle());
            }
            #[cfg(target_os = "android")]
            {
                init_android_audio_context()?;
                crate::android::local_ai::initialize()?;
            }
            #[cfg(target_os = "android")]
            {
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .map_err(std::io::Error::other)?;
                std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", data_dir);
            }
            std::fs::create_dir_all(app_data_dir()).map_err(std::io::Error::other)?;
            let db_handle: DbHandle = match db::open_with_recovery(app_db_path()) {
                Ok(db) => db,
                Err(err) => fatal_startup_error(&format!("failed to open database: {err}")),
            };
            let _ = db::cleanup_cache_prune_expired(&db_handle);
            let _ = db::prune_auto_learn_retention(&db_handle);
            if let Err(e) = db::seed_default_dictionary_entries(&db_handle) {
                log::warn!("failed to seed default dictionary entries: {e}");
            }
            crate::system::diagnostic_journal::start(db_handle.clone());
            app.manage(db_handle);
            crate::system::logger::attach_app(app.handle());
            #[cfg(target_os = "linux")]
            if !is_dev_session() {
                if let Err(error) = crate::core::hyprland::ensure_window_rules() {
                    log::error!("Linux window setup failed: {error}");
                }
            }
            let build_mode = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            log::info!(
                "verenu {} starting — {} {}, {} build — verbose logging {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                build_mode,
                if crate::system::logger::is_verbose() {
                    "on"
                } else {
                    "off"
                }
            );
            let settings = crate::data::store::SettingsHandle::open(app.handle())
                .map_err(std::io::Error::other)?;
            let cache_enabled = settings.get(crate::data::store::CLEANUP_CACHE_ENABLED)
                .and_then(|value| value.as_bool()).unwrap_or(true);
            db::cleanup_cache_set_enabled(app.state::<DbHandle>().inner(), cache_enabled)
                .map_err(std::io::Error::other)?;
            if !is_dev_session() {
                crate::data::credentials::migrate_from_store(app.handle(), &settings);
            }
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                crate::system::linux_webview::configure_window(&window);
            }
            #[cfg(all(feature = "dev-session", debug_assertions, desktop))]
            if is_dev_session() {
                app.manage(settings.clone());
                dev_session::start(app.handle().clone())?;
                if std::env::var("VERENU_AGENT_TEST_SYNTHETIC").as_deref() == Ok("1") {
                    agent_api::start(app.handle().clone())?;
                }
                return Ok(());
            }
            if let Err(error) = crate::data::store::migrate_contextual_formatting(&settings) {
                log::warn!("Failed to migrate contextual formatting setting: {error}");
            }
            let first_launch = {
                if let Some(value) = settings.get(crate::data::store::HOTKEY) {
                    if let Ok(codes) = serde_json::from_value::<Vec<String>>(value) {
                        let normalized = crate::core::hotkey::normalize_codes(&codes);
                        let ids = normalized.as_ref().ok().and_then(|codes| {
                            crate::core::hotkey::mapped_codes(codes).ok()
                        });
                        if ids.as_ref().is_none_or(|ids| crate::core::hotkey::update_keys(ids).is_err()) {
                            #[cfg(target_os = "macos")]
                            let defaults = ["AltLeft", "Space"];
                            #[cfg(not(target_os = "macos"))]
                            let defaults = ["ControlLeft", "MetaLeft"];
                            if let Err(error) = settings.save_value(
                                crate::data::store::HOTKEY,
                                serde_json::json!(defaults),
                            ) {
                                log::warn!("Failed to migrate unsupported hotkey: {error}");
                            }
                            let default_codes = defaults
                                .iter()
                                .map(|code| (*code).to_string())
                                .collect::<Vec<_>>();
                            match crate::core::hotkey::mapped_codes(&default_codes) {
                                Ok(default_ids) => {
                                    if let Err(error) =
                                        crate::core::hotkey::update_keys(&default_ids)
                                    {
                                        log::warn!("Failed to apply default hotkey after migration: {error}");
                                    }
                                }
                                Err(error) => {
                                    log::warn!("Failed to map default hotkey after migration: {error}");
                                }
                            }
                        }
                    }
                }
                let retention_value = settings.get(crate::data::store::HISTORY_RETENTION);
                let retention = retention_value
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .unwrap_or("30 days");
                if let Some(days) = crate::data::store::history_retention_days(retention) {
                    let db = app.state::<DbHandle>().inner().clone();
                    let app_handle = app.handle().clone();
                    let settings = settings.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        match db::prune_transcriptions_for_retention(&db, &settings, days) {
                            Ok(deleted) if deleted > 0 => {
                                let _ = app_handle.emit("verenu:history-pruned", ());
                            }
                            Ok(_) => {}
                            Err(e) => {
                                log::warn!(
                                    "Failed to prune old transcriptions during startup: {e:?}"
                                );
                            }
                        }
                    });
                }
                let first_launch = !settings
                    .get(crate::data::store::SETUP_COMPLETE)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                app.manage(settings.clone());
                #[cfg(desktop)]
                app.manage(crate::analytics::Analytics::new(
                    settings
                        .get(crate::data::store::ANALYTICS_ENABLED)
                        .and_then(|value| value.as_bool())
                        .unwrap_or(true),
                    crate::app_data_dir(),
                ));
                first_launch
            };

            #[cfg(desktop)]
            if let Err(error) = agent_api::start(app.handle().clone()) {
                log::warn!("Agent access is unavailable: {error}");
            }

            #[cfg(desktop)]
            if !is_dev_session() {
                match app.path().home_dir() {
                    Ok(home) => {
                        if let Err(error) = agent_discovery::register(&home, &app_data_dir()) {
                            log::warn!("Could not register agent discovery skill: {error}");
                        }
                    }
                    Err(error) => log::warn!("Could not locate home for agent discovery: {error}"),
                }
            }

            #[cfg(desktop)]
            if !crate::is_dev_session() {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::reconcile_autostart(app_handle).await {
                        log::warn!("Failed to reconcile autostart at startup: {error}");
                    }
                });
            }

            #[cfg(desktop)]
            if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
                analytics.install_panic_hook();
                let context_group_count = app
                    .try_state::<crate::DbHandle>()
                    .and_then(|db| crate::data::db::count_user_contexts(db.inner()).ok())
                    .unwrap_or(0);
                analytics.app_launched(first_launch, &settings, context_group_count);
            }

            #[cfg(target_os = "windows")]
            {
                match app_tray::prepare_windows_shell_icon(app.handle()) {
                    Ok(shell_icon) => {
                        crate::system::notify::prepare_windows_notification_identity(
                            &shell_icon,
                        );
                    }
                    Err(err) => {
                        log::warn!("Continuing without a themed shell icon: {err}");
                    }
                }
            }

            app.manage(sync::SyncManager::start(
                app.handle().clone(),
                app.state::<DbHandle>().inner().clone(),
            ));
            #[cfg(target_os = "android")]
            {
                match crate::android::bridge::start_bridge(app.handle().clone()) {
                    Ok(addr) => log::info!("android bridge listening on {addr}"),
                    Err(error) => log::warn!("android bridge failed to start: {error}"),
                }
            }
            start_storage_maintenance(
                app.handle().clone(),
                app.state::<DbHandle>().inner().clone(),
                settings.clone(),
            );

            #[cfg(desktop)]
            app_tray::setup_tray(app)?;
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                crate::system::linux_titlebar::enable(&window).map_err(|error| {
                    let source: Box<dyn std::error::Error> = Box::new(std::io::Error::other(error));
                    tauri::Error::Setup(source.into())
                })?;
            }
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                let theme = window.theme().ok();
                crate::system::windows_titlebar::enable(&window, theme)
                    .map_err(|error| {
                        let source: Box<dyn std::error::Error> = Box::new(std::io::Error::other(error));
                        tauri::Error::Setup(source.into())
                    })?;
                app_tray::apply_runtime_icons(app.handle(), theme);
            }
            app_hotkey::setup_hotkey(app, shared.clone());
            media::mic_mute_trigger::setup(app, shared.clone());
            #[cfg(target_os = "macos")]
            {
                crate::system::mac_app::set_accessory_activation_policy_on_main_thread(
                    app.handle(),
                );
            }
            start_frontend_watchdog(app.handle(), frontend_readiness.clone());
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut sigterm = match signal(SignalKind::terminate()) {
                        Ok(sigterm) => sigterm,
                        Err(err) => {
                            log::warn!("could not register SIGTERM handler: {err}");
                            return;
                        }
                    };
                    sigterm.recv().await;
                    log::info!("received SIGTERM; exiting cleanly");
                    app_handle.exit(0);
                });
            }
            let local_stt_manager = local_transcription_manager.clone();
            let local_llm_manager = local_cleanup_manager.clone();
            let capture_state = shared.clone();
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            let mut memory_reclaimer = crate::system::memory::IdleMemoryReclaimer::new(
                app.state::<DbHandle>().inner().clone(),
            );
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    pipeline::release_expired_capture_audio(&capture_state);
                    if local_stt_manager
                        .shutdown_signal
                        .load(std::sync::atomic::Ordering::Relaxed)
                        && local_llm_manager
                            .shutdown_signal
                            .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        break;
                    }
                    let idle_stt = local_stt_manager.clone();
                    let idle_llm = local_llm_manager.clone();
                    let idle_app = app_handle.clone();
                    if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
                        idle_stt.unload_if_idle(&idle_app)?;
                        idle_llm.unload_if_idle(&idle_app)?;
                        Ok::<(), anyhow::Error>(())
                    })
                    .await
                    .unwrap_or_else(|error| Err(anyhow::anyhow!(error)))
                    {
                        log::warn!("local models: idle unload maintenance failed: {error}");
                    }

                    let stt_loaded = local_stt_manager
                        .current_model_id
                        .lock()
                        .map(|guard| guard.is_some())
                        .unwrap_or(false);
                    let llm_loaded = local_llm_manager
                        .current_model_id
                        .lock()
                        .map(|guard| guard.is_some())
                        .unwrap_or(false);
                    if stt_loaded || llm_loaded {
                        let pressure = tauri::async_runtime::spawn_blocking(
                            crate::system::memory::detect_resource_pressure,
                        )
                        .await
                        .unwrap_or(None);
                        if let Some(reason) = pressure {
                            log::warn!(
                                "local models: system resource pressure detected ({reason}), unloading proactively"
                            );
                            let pressure_stt = local_stt_manager.clone();
                            let pressure_llm = local_llm_manager.clone();
                            let pressure_app = app_handle.clone();
                            let _ = tauri::async_runtime::spawn_blocking(move || {
                                pressure_stt.unload_for_resource_pressure(&pressure_app);
                                pressure_llm.unload_for_resource_pressure(&pressure_app);
                            })
                            .await;
                        }
                    }
                    #[cfg(all(target_os = "linux", target_env = "gnu"))]
                    {
                        let main_hidden = app_handle.get_webview_window("main")
                            .and_then(|window| window.is_visible().ok())
                            .is_some_and(|visible| !visible);
                        memory_reclaimer.reclaim(&capture_state, main_hidden).await;
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        if crate::system::session::system_is_shutting_down() {
                            return;
                        }
                        api.prevent_close();
                        #[cfg(target_os = "macos")]
                        {
                            hide_main_window(window.app_handle());
                        }
                        #[cfg(not(target_os = "macos"))]
                        {
                            window.hide().ok();
                        }
                    }
                    #[cfg(target_os = "macos")]
                    tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
                        crate::system::mac_app::set_regular_activation_policy_on_main_thread(
                            window.app_handle(),
                        );
                    }
                    tauri::WindowEvent::Focused(true) => {
                        #[cfg(target_os = "windows")]
                        apply_runtime_icons(window.app_handle(), window.theme().ok());
                        #[cfg(target_os = "macos")]
                        {
                            crate::system::mac_app::set_regular_activation_policy_on_main_thread(
                                window.app_handle(),
                            );
                        }
                    }
                    tauri::WindowEvent::ThemeChanged(theme) => {
                        let app = window.app_handle();
                        #[cfg(target_os = "windows")]
                        if let Some(webview) = app.get_webview_window("main") {
                            crate::system::windows_titlebar::refresh(&webview, Some(*theme));
                        }
                        if app_tray::appearance_mode(app)
                            .as_deref()
                            .unwrap_or("system")
                            == "system"
                        {
                            apply_runtime_icons(app, Some(*theme));
                        }
                    }
                    #[cfg(target_os = "windows")]
                    tauri::WindowEvent::Moved(_) => {
                    }
                    #[cfg(target_os = "windows")]
                    tauri::WindowEvent::Resized(_) => {
                        schedule_settled_titlebar_refresh(window.app_handle());
                    }
                    #[cfg(target_os = "windows")]
                    tauri::WindowEvent::ScaleFactorChanged { .. } => {
                        if let Some(webview) = window.app_handle().get_webview_window("main") {
                            crate::system::windows_titlebar::refresh(&webview, window.theme().ok());
                            apply_runtime_icons(window.app_handle(), window.theme().ok());
                        }
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            #[cfg(desktop)] agent_api::agent_pending_request,
            #[cfg(desktop)] agent_api::agent_respond,
            #[cfg(desktop)] agent_api::agent_revoke_all,
            system::windows_titlebar::get_native_titlebar_metrics,
            system::windows_titlebar::set_native_titlebar_theme,
            system::omarchy_theme::get_omarchy_theme,
            commands::save_hotkey,
            commands::save_hotkey_if_unchanged,
            core::hotkey::shortcut_status::get_shortcut_status,
            commands::check_hotkey,
            commands::set_hotkey_capture,
            commands::save_api_key,
            commands::delete_api_key,
            commands::delete_custom_provider,
            commands::get_api_key_status,
            commands::validate_api_key,
            commands::list_provider_models,
            commands::get_provider_model_catalog,
            api::apple_intelligence::get_apple_intelligence_availability,
            commands::open_notifications_settings,
            commands::request_notification_permission,
            commands::check_keychain_access,
            commands::save_setting,
            commands::save_model_settings_if_unchanged,
            commands::save_setting_if_unchanged,
            commands::analytics_setup_event,
            commands::analytics_frontend_exception,
            commands::get_setting,
            commands::set_storage_full_simulation,
            commands::get_storage_full_simulation,
            commands::get_all_settings,
            commands::get_effective_transcription_model,
            commands::list_local_stt_models,
            commands::list_local_llm_models,
            commands::download_local_stt_model,
            commands::download_local_llm_model,
            commands::cancel_local_stt_model_download,
            commands::cancel_local_llm_model_download,
            commands::delete_local_stt_model,
            commands::delete_local_llm_model,
            commands::open_local_stt_models_folder,
            commands::get_local_transcription_state,
            commands::get_model_performance,
            commands::benchmark_local_models,
            commands::get_local_llm_state,
            commands::get_local_llm_runtime_info,
            commands::download_local_llm_runtime,
            commands::cancel_local_llm_runtime_download,
            commands::delete_local_llm_runtime,
            commands::set_autostart,
            commands::set_autostart_if_unchanged,
            commands::initialize_autostart,
            commands::get_macos_permission_snapshot,
            commands::request_accessibility_permission,
            commands::open_accessibility_settings,
            commands::request_microphone_permission,
            commands::request_microphone_permission_snapshot,
            commands::open_microphone_settings,
            commands::restart_app,
            commands::frontend_ready,
            commands::reset_macos_core_permissions,
            commands::get_recent,
            commands::get_history_apps,
            commands::get_stats,
            commands::get_insights,
            commands::get_insights_pricing,
            commands::get_github_commits,
            commands::get_github_username_suggestion,
            commands::count_old_transcriptions,
            commands::get_cleanup_cache_status,
            commands::clear_cleanup_cache,
            commands::get_default_cleanup_prompt,
            commands::lint_cleanup_prompt,
            commands::test_cleanup_prompt,
            commands::get_microphones,
            commands::get_memory_mb,
            commands::get_hardware_capabilities,
            commands::set_diagnostics_monitoring,
            commands::set_diagnostics_profiling,
            commands::clear_diagnostics,
            commands::get_diagnostics_snapshot,
            commands::download_diagnostics_bundle,
            commands::local_models_supported_on_this_platform,
            commands::start_input_recording,
            commands::start_setup_try_recording,
            commands::stop_and_transcribe_input,
            commands::stop_setup_try_recording,
            commands::stop_recording,
            commands::stop_handless_mode,
            commands::resume_cancelled_capture,
            commands::dismiss_cancelled_capture,
            commands::get_cancelled_capture,
            commands::copy_paste_failure_to_clipboard,
            commands::set_pill_size,
            commands::set_pill_hit_rect,
            commands::set_pill_interactive,
            #[cfg(all(feature = "native-testing", debug_assertions, desktop))]
            commands::native_test_pill,
            commands::hide_dictation_pill,
            commands::get_installed_apps,
            commands::get_app_icon,
            commands::get_site_icon,
             commands::get_app_mappings,
             commands::save_app_mappings,
             commands::get_contexts,
             commands::set_context_t3_skill_mentions,
             commands::get_t3_skills,
             commands::connect_t3,
             commands::pull_t3_skills,
             commands::select_t3_catalog,
             commands::set_t3_excluded_skills,
             commands::disconnect_t3,
             commands::create_context,
             commands::update_context,
             commands::save_context_if_unchanged,
             commands::update_context_settings,
             commands::update_context_color,
             commands::set_context_pinned,
             commands::get_context_stats,
             commands::delete_context,
             commands::get_context_targets,
             commands::assign_context_target,
             commands::remove_context_target,
             commands::get_context_websites,
             commands::check_domain_exists,
             commands::assign_context_website,
            commands::get_sub_apps,
            commands::create_sub_app,
            commands::assign_sub_app,
            commands::delete_sub_app,
            commands::take_pending_sub_app_capture,
             commands::remove_context_website,
             commands::get_context_dictionary,
             commands::get_context_snippets,
             commands::set_dictionary_context_assignment,
             commands::set_snippet_context_assignment,
             commands::move_snippet_context_assignment,
            commands::get_snippets,
            commands::create_snippet,
            commands::edit_snippet,
            commands::remove_snippet,
            commands::get_dictionary,
            commands::create_dictionary_entry,
            commands::edit_dictionary_entry,
            commands::rename_dictionary_entry_global,
            commands::remove_dictionary_entry,
            commands::move_dictionary_entry_to_context,
            commands::duplicate_context,
            commands::get_dictionary_entry_contexts,
            commands::get_snippet_entry_contexts,
            commands::move_dictionary_entry_by_term_to_context,
            commands::move_snippet_entry_to_context,
            commands::get_auto_learn_status_summary,
            commands::get_recent_auto_learn_activity,
            commands::retry_transcription,
            commands::check_for_update,
            commands::reinstall_latest_update,
            commands::install_update,
            commands::check_provider_status,
            commands::check_provider_status_raw,
            commands::check_global_message,
            commands::check_verenu_api_health,
            commands::check_connectivity,
            commands::get_recent_logs,
            commands::subscribe_log_stream,
            commands::unsubscribe_log_stream,
            commands::download_logs,
            commands::set_dev_logging_enabled,
            commands::get_dev_logging_enabled,
            commands::notify_update_available,
            commands::notify_provider_and_global_message,
            commands::test_notifications,
            commands::export_data,
            commands::import_data,
            commands::sync_get_status,
            commands::sync_set_device_name,
            commands::sync_set_peer_address,
            commands::sync_pair_connection,
            commands::sync_start_pairing,
            commands::sync_respond_to_pairing,
            commands::sync_cancel_pairing,
            commands::sync_remove_device,
            commands::sync_now,
            commands::sync_get_diagnostics,
            commands::android_get_platform_info,
            #[cfg(all(debug_assertions, feature = "android-local-testing", target_os = "android"))]
            commands::android_test_local_audio,
            commands::android_on_keyboard_visibility,
            commands::android_decide_insertion,
            commands::android_context_for_package,
            commands::android_provide_credential,
            commands::android_keystore_save,
            commands::android_clear_credentials,
            commands::android_has_credential,
            commands::android_permission_rationale,
            commands::android_request_permission,
            commands::android_read_permissions,
            commands::android_evaluate_permissions,
            commands::android_width_class,
            commands::android_insert_text_result,
            commands::android_on_permission_revoked,
        ])
        .build(tauri::generate_context!())
        .expect("error building Verenu")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main_window(_app);
            }
            if let tauri::RunEvent::ExitRequested { .. } = _event {
                log::info!("app exit requested");
            }
            if let tauri::RunEvent::Exit = _event {
                crate::media::sound::release_all_mutes_on_exit();
                log::info!("app exiting; unloading local models");
                crate::pipeline::failover::flush_on_exit(_app);
                crate::system::shutdown_local_models(_app);
                #[cfg(target_os = "windows")]
                app_tray::cleanup_runtime_icon_files();
                log::info!("app shutdown complete");
                log::logger().flush();
            }
        });
}

#[cfg(target_os = "linux")]
fn configure_hyprland_webkit_renderer() {
    if should_force_hyprland_webkit_shm(
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some(),
        std::env::var_os("WEBKIT_DMABUF_RENDERER_FORCE_SHM").is_some(),
    ) {
        // WebKit 2.52 returns an empty buffer-transport set when
        // WEBKIT_DISABLE_DMABUF_RENDERER is enabled, so accelerated
        // compositing can create a null backing store. Force the supported
        // shared-memory transport instead. This runs before GTK/WebKit init.
        unsafe { std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1") };
    }
}

#[cfg(target_os = "linux")]
fn should_force_hyprland_webkit_shm(
    on_hyprland: bool,
    on_wayland: bool,
    disable_dmabuf_set: bool,
    force_shm_set: bool,
) -> bool {
    on_hyprland && on_wayland && !disable_dmabuf_set && !force_shm_set
}

#[cfg(all(test, target_os = "linux"))]
mod linux_webkit_renderer_tests {
    use super::should_force_hyprland_webkit_shm;

    #[test]
    fn defaults_hyprland_to_shared_memory_without_overriding_user_renderer_flags() {
        assert!(should_force_hyprland_webkit_shm(true, true, false, false));
        assert!(!should_force_hyprland_webkit_shm(false, true, false, false));
        assert!(!should_force_hyprland_webkit_shm(true, false, false, false));
        assert!(!should_force_hyprland_webkit_shm(true, true, true, false));
        assert!(!should_force_hyprland_webkit_shm(true, true, false, true));
        assert!(!should_force_hyprland_webkit_shm(true, true, true, true));
    }
}
#[cfg(target_os = "windows")]
static TITLEBAR_REFRESH_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(target_os = "windows")]
fn schedule_settled_titlebar_refresh(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    let gen = TITLEBAR_REFRESH_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        if TITLEBAR_REFRESH_GEN.load(Ordering::Relaxed) != gen {
            return;
        }
        if let Some(webview) = app.get_webview_window("main") {
            crate::system::windows_titlebar::refresh(&webview, webview.theme().ok());
        }
    });
}
