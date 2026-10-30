use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

#[allow(dead_code)]
pub(crate) struct PreparedHandoff {
    app: AppHandle,
    pause: Option<crate::linux_activity::Pause>,
    agent: Option<tokio::sync::OwnedSemaphorePermit>,
    shortcuts: Option<crate::core::hotkey::ShortcutPause>,
}

impl Drop for PreparedHandoff {
    fn drop(&mut self) {
        let (Some(pause), Some(agent)) = (self.pause.take(), self.agent.take()) else {
            return;
        };
        let app = self.app.clone();
        let shortcuts = self.shortcuts.take();
        tauri::async_runtime::spawn(async move {
            if let Some(shortcuts) = shortcuts {
                let restored =
                    tokio::time::timeout(Duration::from_secs(35), shortcuts.resume()).await;
                if !matches!(restored, Ok(Ok(()))) {
                    log::error!("Could not restore global shortcuts after restart preparation: {restored:?}");
                    let window_app = app.clone();
                    let _ = app.run_on_main_thread(move || crate::show_main_window(&window_app));
                    let _ = app.emit("verenu:error", "Verenu is still running, but global shortcuts could not be restored. Quit and reopen the app when convenient.");
                }
            }
            // A canceled shutdown and the new registration can each take ten seconds.
            let restored = tokio::time::timeout(
                Duration::from_secs(22),
                crate::app_tray::resume_after_handoff(&app),
            )
            .await;
            if !matches!(restored, Ok(Ok(()))) {
                log::error!(
                    "Could not restore the Linux tray after restart preparation: {restored:?}"
                );
                let window_app = app.clone();
                let _ = app.run_on_main_thread(move || crate::show_main_window(&window_app));
                let _ = app.emit(
                    "verenu:error",
                    "Verenu is still running, but its tray could not be restored. Quit and reopen the app when convenient.",
                );
            }
            drop(agent);
            drop(pause);
        });
    }
}

#[allow(dead_code)]
pub(crate) async fn prepare(app: &AppHandle) -> Result<PreparedHandoff, String> {
    tokio::time::timeout(Duration::from_secs(10), prepare_inner(app))
        .await
        .map_err(|_| {
            "Verenu is busy. Wait for the current work to finish and try again.".to_owned()
        })?
}

async fn prepare_inner(app: &AppHandle) -> Result<PreparedHandoff, String> {
    if crate::is_dev_session() {
        return Err("Automatic restart is unavailable in development sessions.".into());
    }
    let agent = app
        .try_state::<crate::agent_api::AgentDrain>()
        .ok_or("The agent service is unavailable; quit and reopen Verenu to finish updating.")?;
    let agent_permit = agent.pause().await?;
    crate::agent_api::AgentDrain::check_pending_approval(app)?;
    let gate = app
        .try_state::<Arc<crate::linux_activity::ActivityGate>>()
        .ok_or("Restart preparation is unavailable.")?
        .inner()
        .clone();
    let pause = gate.pause().await.map_err(str::to_owned)?;

    {
        let recording = app
            .try_state::<crate::pipeline::SharedState>()
            .ok_or("Recording state is unavailable.")?;
        let recording = recording
            .try_lock()
            .map_err(|_| "Recording is busy. Try restarting again shortly.")?;
        if !recording.lifecycle.is_idle()
            || recording.retry_capture.is_some()
            || recording.cancelled_capture.is_some()
            || recording.paste_failure.is_some()
        {
            return Err("Finish the current dictation or recovery before restarting.".into());
        }
    }

    let sync = app
        .try_state::<crate::sync::SyncManager>()
        .ok_or("Sync state is unavailable; quit and reopen Verenu to finish updating.")?;
    sync.check_handoff_idle(&pause)
        .map_err(|error| error.to_string())?;
    {
        let db = app
            .try_state::<crate::DbHandle>()
            .ok_or("Database is unavailable.")?;
        let conn = db
            .try_lock()
            .map_err(|_| "Database is busy. Try restarting again shortly.")?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| format!("Could not check the database for restart: {error}"))?;
        if version != crate::data::db::HANDOFF_SCHEMA_VERSION {
            return Err("This update needs a manual restart to migrate its database.".into());
        }
    }
    crate::data::store::SettingsSnapshot::read_for_handoff(
        &crate::app_data_dir().join("settings.json"),
    )?;
    let mut prepared = PreparedHandoff {
        app: app.clone(),
        pause: Some(pause),
        agent: Some(agent_permit),
        shortcuts: None,
    };
    prepared.shortcuts = Some(crate::core::hotkey::ShortcutPause::begin()?);
    prepared
        .shortcuts
        .as_ref()
        .expect("shortcut pause was just installed")
        .wait()
        .await?;
    crate::app_tray::suspend_for_handoff(app).await?;
    Ok(prepared)
}

#[cfg(all(test, feature = "native-testing"))]
mod tests {
    use super::*;
    use tauri::{Listener, Manager};

    struct TestWatcher {
        blocked: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        entered: Arc<std::sync::atomic::AtomicBool>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl TestWatcher {
        fn register_status_notifier_item(&self, _service: &str) {
            self.entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let (lock, condition) = &*self.blocked;
            drop(
                condition
                    .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(15), |blocked| {
                        *blocked
                    })
                    .unwrap(),
            );
        }
    }

    async fn tray_names(bus: &zbus::fdo::DBusProxy<'_>) -> Vec<String> {
        let prefix = format!("org.kde.StatusNotifierItem-{}-", std::process::id());
        bus.list_names()
            .await
            .unwrap()
            .into_iter()
            .filter(|name| name.as_str().starts_with(&prefix))
            .map(|name| name.to_string())
            .collect()
    }

    #[test]
    #[ignore = "requires an isolated graphical session"]
    fn preparation_drains_and_restores_services_on_failure() {
        use crate::pipeline::{AppState, DictationLifecycle, SharedState};
        use std::sync::Mutex;
        let blocked = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _watcher = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                TestWatcher {
                    blocked: blocked.clone(),
                    entered: entered.clone(),
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("verenu-handoff-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old_dir = std::env::var_os("VERENU_APP_DATA_DIR_OVERRIDE");
        std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", &dir);
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        app.manage(gate.clone());
        app.manage(crate::local_stt::LocalTranscriptionManager::new());
        app.manage(crate::local_llm::LocalLlmManager::new());
        let agent = Arc::new(tokio::sync::Semaphore::new(1));
        app.manage(crate::agent_api::AgentDrain::for_test(agent.clone()));
        crate::agent_api::manage_test_approvals(app.handle());
        let settings =
            crate::data::store::SettingsHandle::empty_for_test(dir.join("settings.json"));
        settings
            .set(
                crate::data::store::APPEARANCE_MODE,
                serde_json::json!("dark"),
            )
            .unwrap();
        app.manage(settings);
        crate::app_tray::setup_tray(app.handle()).unwrap();
        let db = crate::data::db::open(":memory:").unwrap();
        app.manage(crate::sync::SyncManager::new_unstarted(
            app.handle().clone(),
            db.clone(),
        ));
        app.manage(db.clone());
        let recording: SharedState = Arc::new(Mutex::new(AppState {
            lifecycle: DictationLifecycle::Idle,
            analytics_run_id: None,
            target: Default::default(),
            pill_placement: None,
            pill_placement_stale: false,
            pill_width_points: crate::pipeline::DEFAULT_PILL_WIDTH_POINTS,
            pill_height_points: crate::pipeline::DEFAULT_PILL_HEIGHT_POINTS,
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
        app.manage(recording.clone());
        let (error_sent, errors) = std::sync::mpsc::channel();
        app.listen_any("verenu:error", move |event| {
            let _ = error_sent.send(event.payload().to_owned());
        });

        tauri::async_runtime::block_on(async {
            let shortcut_driver = crate::core::hotkey::install_test_shortcut_control().await;
            crate::app_tray::resume_after_handoff(app.handle())
                .await
                .unwrap();
            let connection = zbus::Connection::session().await.unwrap();
            let bus = zbus::fdo::DBusProxy::new(&connection).await.unwrap();
            let original_tray = tray_names(&bus).await;
            assert_eq!(original_tray.len(), 1);
            let active_agent = agent.clone().acquire_owned().await.unwrap();
            let preparation = prepare(app.handle());
            fn assert_send<T: Send>(_: &T) {}
            assert_send(&preparation);
            tokio::pin!(preparation);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut preparation)
                    .await
                    .is_err()
            );
            assert!(gate.try_enter().is_some());
            drop(active_agent);
            let lease = preparation.await.unwrap();
            assert!(gate.try_enter().is_none());
            assert!(!crate::core::hotkey::test_shortcuts_active());
            assert!(
                crate::commands::start_input_recording(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::start_setup_try_recording(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::resume_cancelled_capture(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::stop_recording(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::stop_setup_try_recording(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::stop_and_transcribe_input(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::stop_handless_mode(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(
                crate::commands::dismiss_cancelled_capture(app.handle().clone(), app.state())
                    .await
                    .unwrap_err()
                    .contains("preparing to restart")
            );
            assert!(crate::commands::copy_paste_failure_to_clipboard(
                app.handle().clone(),
                app.state()
            )
            .await
            .unwrap_err()
            .contains("preparing to restart"));
            assert!(recording.lock().unwrap().lifecycle.is_idle());
            assert!(crate::commands::download_local_stt_model(
                app.handle().clone(), app.state(), "invalid".into(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(crate::commands::download_local_llm_model(
                app.handle().clone(), app.state(), "invalid".into(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(crate::commands::download_local_llm_runtime(
                app.handle().clone(), app.state(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(crate::commands::delete_local_stt_model(
                app.handle().clone(), app.state(), "invalid".into(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(crate::commands::delete_local_llm_model(
                app.handle().clone(), app.state(), "invalid".into(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(crate::commands::delete_local_llm_runtime(
                app.handle().clone(), app.state(),
            ).await.unwrap_err().contains("preparing to restart"));
            assert!(agent.clone().try_acquire_owned().is_err());
            assert!(tray_names(&bus).await.is_empty());
            entered.store(false, std::sync::atomic::Ordering::SeqCst);
            *blocked.0.lock().unwrap() = true;
            crate::core::hotkey::test_block_shortcut_resume(true);
            drop(lease);
            tokio::time::timeout(Duration::from_secs(5), async {
                while !crate::core::hotkey::test_shortcut_resume_entered() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("shortcut restoration did not begin");
            assert!(gate.try_enter().is_none());
            assert!(agent.clone().try_acquire_owned().is_err());
            crate::core::hotkey::test_block_shortcut_resume(false);
            tokio::time::timeout(Duration::from_secs(10), async {
                while !entered.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("tray restoration did not begin");
            assert!(gate.try_enter().is_none());
            assert!(agent.clone().try_acquire_owned().is_err());
            *blocked.0.lock().unwrap() = false;
            blocked.1.notify_all();
            let activity = tokio::time::timeout(Duration::from_secs(15), gate.enter())
                .await
                .expect("restart cancellation did not resume application activity");
            drop(activity);
            assert!(crate::core::hotkey::test_shortcuts_active());
            assert!(agent.clone().try_acquire_owned().is_ok());
            let restored_tray = tray_names(&bus).await;
            assert_eq!(restored_tray.len(), 1);
            assert_ne!(original_tray, restored_tray);

            let lease = prepare(app.handle()).await.unwrap();
            entered.store(false, std::sync::atomic::Ordering::SeqCst);
            *blocked.0.lock().unwrap() = true;
            drop(lease);
            tokio::time::timeout(Duration::from_secs(10), async {
                while !entered.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("tray restoration did not begin");
            let resumed = tokio::time::timeout(Duration::from_secs(15), gate.enter())
                .await
                .expect("failed tray restoration left application activity paused");
            drop(resumed);
            assert!(agent.clone().try_acquire_owned().is_ok());
            assert!(errors
                .try_recv()
                .unwrap()
                .contains("tray could not be restored"));
            *blocked.0.lock().unwrap() = false;
            blocked.1.notify_all();
            crate::app_tray::resume_after_handoff(app.handle())
                .await
                .unwrap();
            assert_eq!(tray_names(&bus).await.len(), 1);

            let activity = gate.try_enter().unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), prepare(app.handle()))
                    .await
                    .is_err()
            );
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            drop(activity);

            crate::agent_api::set_test_pending_approval(app.handle(), true);
            assert!(prepare(app.handle()).await.is_err());
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            crate::agent_api::set_test_pending_approval(app.handle(), false);

            recording.lock().unwrap().lifecycle = DictationLifecycle::Starting {
                prepend_audio: None,
            };
            assert!(prepare(app.handle()).await.is_err());
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            recording.lock().unwrap().lifecycle = DictationLifecycle::Idle;
            db.lock()
                .unwrap()
                .pragma_update(None, "user_version", 1)
                .unwrap();
            assert!(prepare(app.handle()).await.is_err());
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            db.lock()
                .unwrap()
                .pragma_update(
                    None,
                    "user_version",
                    crate::data::db::HANDOFF_SCHEMA_VERSION,
                )
                .unwrap();
            std::fs::write(dir.join("settings.json"), "{").unwrap();
            assert!(prepare(app.handle()).await.is_err());
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), b"{");
            crate::app_tray::suspend_for_handoff(app.handle())
                .await
                .unwrap();
            shortcut_driver.abort();
        });
        match old_dir {
            Some(value) => std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", value),
            None => std::env::remove_var("VERENU_APP_DATA_DIR_OVERRIDE"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
