use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) struct PreparedHandoff {
    app: AppHandle,
    pause: Option<crate::linux_activity::Pause>,
    agent: Option<tokio::sync::OwnedSemaphorePermit>,
    shortcuts: Option<crate::core::hotkey::ShortcutPause>,
    replacement_group: Option<u32>,
    callback_lock: Option<std::fs::File>,
    tray_touched: bool,
}

impl PreparedHandoff {
    pub(crate) fn register_replacement(&mut self, replacement_group: u32) -> Result<(), String> {
        if self.replacement_group.is_some()
            || !crate::linux_handoff_transport::is_replacement_group(replacement_group)
        {
            return Err("Invalid restart ownership transfer.".into());
        }
        self.replacement_group = Some(replacement_group);
        Ok(())
    }
}

impl Drop for PreparedHandoff {
    fn drop(&mut self) {
        let (Some(pause), Some(agent)) = (self.pause.take(), self.agent.take()) else {
            return;
        };
        let app = self.app.clone();
        let shortcuts = self.shortcuts.take();
        let replacement_group = self.replacement_group.take();
        // A waiting launcher may enter the paused gate while restoration runs.
        // Restoration must not wait for that launcher or acquire this lock.
        drop(self.callback_lock.take());
        let tray_touched = self.tray_touched;
        tauri::async_runtime::spawn(async move {
            if tokio::time::timeout(Duration::from_secs(5), pause.wait_drained()).await.is_err() {
                let _ = app.emit("verenu:error", "Restart was canceled. Verenu is waiting for current work to finish before restoring its services.");
                pause.wait_drained().await;
            }
            if let Some(group) = replacement_group {
                let mut warned = false;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let owner_app = app.clone();
                    let restored = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
                        if crate::linux_handoff_transport::group_has_live_members(group)? {
                            return Ok(false);
                        }
                        let owner = owner_app.try_state::<crate::linux_single_instance::InstanceOwner>()
                            .ok_or("Single-instance ownership is unavailable")?;
                        if !owner.owns_name()? {
                            return Ok(false);
                        }
                        Ok(true)
                    }).await;
                    if matches!(restored, Ok(Ok(true))) {
                        break;
                    }
                    if !warned && tokio::time::Instant::now() >= deadline {
                        warned = true;
                        log::error!("Waiting for replacement cleanup and instance ownership: {restored:?}");
                        let window_app = app.clone();
                        let _ = app.run_on_main_thread(move || crate::show_main_window(&window_app));
                        let _ = app.emit("verenu:error", "Restart could not finish. Verenu is waiting for the replacement to stop before restoring its services.");
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let restored = match app.try_state::<crate::agent_api::AgentDrain>() {
                    Some(agent) => agent.restore_discovery().await.map_err(|error| error.to_string()),
                    None => Err("Agent service is unavailable".to_string()),
                };
                if let Err(error) = restored {
                    log::error!("Could not restore agent discovery after restart: {error}");
                    let _ = app.emit("verenu:error", "Verenu is still running, but local agent access could not be restored. Quit and reopen the app when convenient.");
                }
            }
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
            if tray_touched {
                // A canceled shutdown and the new registration can each take ten seconds.
                let restored = tokio::time::timeout(
                    Duration::from_secs(22),
                    crate::app_tray::resume_after_handoff(&app),
                ).await;
                if !matches!(restored, Ok(Ok(()))) {
                    log::error!("Could not restore the Linux tray after restart preparation: {restored:?}");
                    let window_app = app.clone();
                    let _ = app.run_on_main_thread(move || crate::show_main_window(&window_app));
                    let _ = app.emit("verenu:error", "Verenu is still running, but its tray could not be restored. Quit and reopen the app when convenient.");
                }
            }
            drop(agent);
            drop(pause);
        });
    }
}

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
    let callback_lock = crate::linux_single_instance::prepare_callback_lock().await?;
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
    app.try_state::<crate::linux_single_instance::InstanceOwner>()
        .ok_or("Single-instance ownership is unavailable.")?;
    let pause = gate.begin_pause()?;
    let mut prepared = PreparedHandoff {
        app: app.clone(),
        pause: Some(pause),
        agent: Some(agent_permit),
        shortcuts: None,
        replacement_group: None,
        callback_lock: Some(callback_lock),
        tray_touched: false,
    };
    prepared.pause.as_ref().expect("pause was just installed").wait_drained().await;

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
    crate::pipeline::failover::ensure_no_pending_handoff(&crate::pipeline::failover::failover_dir())?;
    crate::media::nvidia_reuse::ensure_restart_idle()?;

    let sync = app
        .try_state::<crate::sync::SyncManager>()
        .ok_or("Sync state is unavailable; quit and reopen Verenu to finish updating.")?;
    sync.check_handoff_idle(prepared.pause.as_ref().expect("pause is active"))
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
    app.try_state::<crate::data::store::SettingsHandle>()
        .ok_or("Settings are unavailable; quit and reopen Verenu to finish updating.")?
        .validate_handoff_snapshot()?;
    prepared.shortcuts = Some(crate::core::hotkey::ShortcutPause::begin()?);
    prepared
        .shortcuts
        .as_ref()
        .expect("shortcut pause was just installed")
        .wait()
        .await?;
    prepared.tray_touched = true;
    crate::app_tray::suspend_for_handoff(app).await?;
    Ok(prepared)
}

#[cfg(all(test, feature = "native-testing"))]
mod tests {
    use super::*;
    use tauri::{Listener, Manager};

    fn replacement_command(
        binary: &std::ffi::OsStr,
        dir: &std::path::Path,
    ) -> std::process::Command {
        let mut command = std::process::Command::new(binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .env("VERENU_HANDOFF_TEST_SHORTCUTS", "1")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT");
        command
    }

    struct TestWatcher {
        blocked: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        entered: Arc<std::sync::atomic::AtomicBool>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl TestWatcher {
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }

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
        let dir = std::env::temp_dir().join(format!("verenu-handoff-test-{}", uuid::Uuid::new_v4()));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let old_dir = std::env::var_os("VERENU_APP_DATA_DIR_OVERRIDE");
        std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", &dir);
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        let app = tauri::Builder::default()
            .any_thread()
            .plugin(crate::linux_single_instance::plugin())
            .build(tauri::generate_context!())
            .unwrap();
        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        app.manage(gate.clone());
        app.manage(crate::local_stt::LocalTranscriptionManager::new());
        app.manage(crate::local_llm::LocalLlmManager::new());
        let agent = crate::agent_api::start_test_agent(app.handle().clone()).unwrap();
        let agent_descriptor = std::fs::read(dir.join("agent-session.json")).unwrap();
        let settings =
            crate::data::store::SettingsHandle::empty_for_test(dir.join("settings.json"));
        settings
            .set(
                crate::data::store::APPEARANCE_MODE,
                serde_json::json!("dark"),
            )
            .unwrap();
        settings.set(crate::data::store::SETUP_COMPLETE, serde_json::json!(true)).unwrap();
        settings.set(crate::data::store::ANALYTICS_ENABLED, serde_json::json!(false)).unwrap();
        settings.save().unwrap();
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
            let recovery = dir.join("dictation-failover");
            std::fs::create_dir(&recovery).unwrap();
            std::fs::write(recovery.join("pending"), b"synthetic recovery marker").unwrap();
            assert!(matches!(prepare(app.handle()).await, Err(error) if error.contains("dictation recovery")));
            drop(tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap());
            assert_eq!(std::fs::read(recovery.join("pending")).unwrap(), b"synthetic recovery marker");
            std::fs::remove_dir_all(recovery).unwrap();
            let worker = crate::media::nvidia_reuse::test_active_worker();
            assert!(matches!(prepare(app.handle()).await, Err(error) if error.contains("finishing a recording")));
            drop(worker);
            drop(tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap());
            crate::core::hotkey::test_set_gesture_active(true);
            assert!(matches!(
                prepare(app.handle()).await,
                Err(error) if error.contains("shortcut gesture")
            ));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while gate.try_enter().is_none() || !crate::core::hotkey::test_shortcuts_active() {
                assert!(tokio::time::Instant::now() < deadline, "services did not resume");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            crate::core::hotkey::test_set_gesture_active(false);
            crate::core::hotkey::test_set_capture_active(true);
            assert!(matches!(
                prepare(app.handle()).await,
                Err(error) if error.contains("changing your shortcut")
            ));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while gate.try_enter().is_none() || !crate::core::hotkey::test_shortcuts_active() {
                assert!(tokio::time::Instant::now() < deadline, "services did not resume");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            crate::core::hotkey::test_set_capture_active(false);
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
            macro_rules! assert_paused {
                ($command:ident $(, $arg:expr)*) => {
                    assert!(
                        matches!(
                            crate::commands::$command(app.handle().clone(), app.state() $(, $arg)*).await,
                            Err(error) if error.contains("preparing to restart")
                        ),
                        "restart did not block {}",
                        stringify!($command)
                    );
                };
            }
            assert_paused!(start_input_recording);
            assert_paused!(start_setup_try_recording);
            assert_paused!(resume_cancelled_capture);
            assert_paused!(stop_recording);
            assert_paused!(stop_setup_try_recording);
            assert_paused!(stop_and_transcribe_input);
            assert_paused!(stop_handless_mode);
            assert_paused!(dismiss_cancelled_capture);
            assert_paused!(copy_paste_failure_to_clipboard);
            assert!(recording.lock().unwrap().lifecycle.is_idle());
            assert_paused!(download_local_stt_model, "invalid".into());
            assert_paused!(download_local_llm_model, "invalid".into());
            assert_paused!(download_local_llm_runtime);
            assert_paused!(delete_local_stt_model, "invalid".into());
            assert_paused!(delete_local_llm_model, "invalid".into());
            assert_paused!(delete_local_llm_runtime);
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

            use std::io::{BufRead, BufReader, Write};
            use std::os::unix::process::CommandExt;
            use std::process::{Command, Stdio};
            struct Replacement(std::process::Child);
            impl Drop for Replacement {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let lease = prepare(app.handle()).await.unwrap();
            let mut replacement = Replacement(Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "linux_single_instance::tests::replacement_queue_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("VERENU_TEST_TRANSFER_NAME", format!("{}.SingleInstance", app.config().identifier))
                .env("VERENU_TEST_TRANSFER_WAIT", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap());
            let stderr = replacement.0.stderr.take().unwrap();
            let (ready, ready_rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if line == "replacement-acquired" {
                        let _ = ready.send(());
                        break;
                    }
                }
            });
            let callbacks = crate::linux_single_instance::test_callback_count();
            let registered = crate::linux_single_instance::test_launcher_registered();
            let started = crate::linux_single_instance::test_callback_started();
            let name = format!("{}.SingleInstance", app.config().identifier);
            let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
            let pending_callback = tauri::async_runtime::spawn_blocking(move || {
                crate::linux_single_instance::test_callback(&zbus::blocking::Connection::session().unwrap(),
                    &name, &path, vec!["--verenu-hotkey-release".into()]).unwrap();
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while crate::linux_single_instance::test_launcher_registered() == registered {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            let name = format!("{}.SingleInstance", app.config().identifier);
            let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
            let second_callback = tauri::async_runtime::spawn_blocking(move || {
                crate::linux_single_instance::test_callback(&zbus::blocking::Connection::session().unwrap(),
                    &name, &path, vec!["--verenu-hotkey-handsfree".into()]).unwrap();
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while crate::linux_single_instance::test_launcher_registered() < registered + 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks);
            let replacement_group = replacement.0.id();
            let app_handle = app.handle().clone();
            assert!(tauri::async_runtime::spawn_blocking(move || {
                app_handle.state::<crate::linux_single_instance::InstanceOwner>()
                    .verify_queued_replacement(replacement_group).is_err()
            }).await.unwrap());
            let lease = tauri::async_runtime::spawn_blocking(move || {
                let mut lease = lease;
                lease.register_replacement(replacement_group).unwrap();
                lease
            }).await.unwrap();
            replacement.0.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
            tokio::time::timeout(Duration::from_secs(10), ready_rx).await.unwrap().unwrap();
            let app_handle = app.handle().clone();
            assert!(tauri::async_runtime::spawn_blocking(move || {
                app_handle.state::<crate::linux_single_instance::InstanceOwner>().owns_name().unwrap()
            }).await.unwrap());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let owner = app_handle.state::<crate::linux_single_instance::InstanceOwner>();
                owner.verify_queued_replacement(replacement_group).unwrap();
            }).await.unwrap();
            assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks);
            drop(lease);
            tokio::time::timeout(Duration::from_secs(5), async {
                while crate::linux_single_instance::test_callback_started() == started {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert!(gate.try_enter().is_none(), "activity resumed while replacement was alive");
            assert!(!crate::core::hotkey::test_shortcuts_active());
            replacement.0.kill().unwrap();
            replacement.0.wait().unwrap();
            let activity = tokio::time::timeout(Duration::from_secs(15), gate.enter())
                .await
                .expect("activity did not resume after replacement stopped");
            drop(activity);
            assert!(crate::core::hotkey::test_shortcuts_active());
            tokio::time::timeout(Duration::from_secs(5), pending_callback).await.unwrap().unwrap();
            tokio::time::timeout(Duration::from_secs(5), second_callback).await.unwrap().unwrap();
            assert_eq!(crate::linux_single_instance::test_callbacks_since(callbacks), [
                crate::linux_handoff_callbacks::Callback::Release,
                crate::linux_handoff_callbacks::Callback::Handsfree,
            ]);
            let callbacks = crate::linux_single_instance::test_callback_count();
            let name = format!("{}.SingleInstance", app.config().identifier);
            let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
            tauri::async_runtime::spawn_blocking(move || {
                crate::linux_single_instance::test_callback(&zbus::blocking::Connection::session().unwrap(),
                    &name, &path, vec!["--verenu-hotkey-release".into()]).unwrap();
            }).await.unwrap();
            assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks + 1);

            let mut impostor = Command::new(std::env::current_exe().unwrap());
            impostor.args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
            ]).env("VERENU_HANDOFF_TEST_CHILD", "activate")
                .env("VERENU_HANDOFF_TEST_DESCRIPTOR", dir.join("agent-session.json"));
            let outcome = crate::linux_handoff_transport::stage_replacement(
                app.handle(), impostor, 39, Default::default(), Duration::from_secs(5),
            ).await;
            assert!(matches!(outcome, Err(ref reason) if reason.contains("not queued")));
            drop(tokio::time::timeout(Duration::from_secs(15), gate.enter()).await.unwrap());
            assert_eq!(std::fs::read(dir.join("agent-session.json")).unwrap(), agent_descriptor);
            let app_handle = app.handle().clone();
            assert!(tauri::async_runtime::spawn_blocking(move || {
                app_handle.state::<crate::linux_single_instance::InstanceOwner>().owns_name().unwrap()
            }).await.unwrap());

            let marker = dir.join("replacement-started");
            let mut hanging = Command::new(std::env::current_exe().unwrap());
            hanging.args([
                "--exact", "linux_handoff_transport::tests::replacement_process_fixture",
            ]).env("VERENU_HANDOFF_TEST_CHILD", "hang")
              .env("VERENU_HANDOFF_TEST_MARKER", &marker);
            let handle = app.handle().clone();
            let attempt = tokio::spawn(async move {
                crate::linux_handoff_transport::stage_replacement(
                    &handle, hanging, 39, Default::default(), Duration::from_secs(10),
                ).await
            });
            tokio::time::timeout(Duration::from_secs(8), async {
                while !marker.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await.expect("replacement did not start");
            let pid: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
            attempt.abort();
            let _ = attempt.await;
            assert!(gate.try_enter().is_none());
            drop(tokio::time::timeout(Duration::from_secs(20), gate.enter()).await
                .expect("canceled restart did not restore the parent"));
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "canceled replacement is still alive");

            if let Some(binary) = std::env::var_os("VERENU_HANDOFF_TEST_BINARY") {
                let disk_db = crate::data::db::open(dir.join("verenu.db")).unwrap();
                let settings_before = std::fs::read(dir.join("settings.json")).unwrap();
                let real = replacement_command(&binary, &dir);
                let outcome = crate::linux_handoff_transport::stage_replacement(
                    app.handle(), real, crate::data::db::HANDOFF_SCHEMA_VERSION,
                    Default::default(), Duration::from_secs(25),
                ).await;
                match outcome {
                    Err(reason) => assert!(reason.contains("Replacement sync could not prepare"), "unexpected restart failure: {reason}"),
                    Ok(_) => panic!("restart unexpectedly reached commit readiness"),
                }
                drop(tokio::time::timeout(Duration::from_secs(15), gate.enter()).await.unwrap());
                let app_handle = app.handle().clone();
                assert!(tauri::async_runtime::spawn_blocking(move || {
                    app_handle.state::<crate::linux_single_instance::InstanceOwner>().owns_name().unwrap()
                }).await.unwrap());
                assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), settings_before);

                let identity = crate::sync::identity::generate_for_tests();
                let key = identity.tls_key();
                crate::sync::secrets::store_identity_key(&dir, key.secret_pkcs8_der()).unwrap();
                std::fs::write(dir.join("sync-identity.der"), identity.cert_der().as_ref()).unwrap();
                crate::sync::store::ensure_self_identity(&disk_db.lock().unwrap(), &identity.uuid, &identity.name).unwrap();
                let mut broken = replacement_command(&binary, &dir);
                broken.env("VERENU_HANDOFF_TEST_STOP_MAINTENANCE", "1");
                let failed = crate::linux_handoff_transport::stage_replacement(
                    app.handle(), broken, crate::data::db::HANDOFF_SCHEMA_VERSION,
                    Default::default(), Duration::from_secs(25),
                ).await;
                assert!(matches!(failed, Err(ref reason) if reason.contains("maintenance stopped")));
                drop(tokio::time::timeout(Duration::from_secs(20), gate.enter()).await
                    .expect("maintenance failure did not restore the parent"));
                assert_eq!(std::fs::read(dir.join("agent-session.json")).unwrap(), agent_descriptor);
                let mut real = replacement_command(&binary, &dir);
                real.env("VERENU_HANDOFF_TEST_FAIL_ACTIVATION", "1");
                let callbacks = crate::linux_single_instance::test_callback_count();
                let staged = crate::linux_handoff_transport::stage_replacement(
                    app.handle(), real, crate::data::db::HANDOFF_SCHEMA_VERSION,
                    Default::default(), Duration::from_secs(25),
                ).await.expect("replacement did not prepare its services");
                assert!(gate.try_enter().is_none());
                let replacement_descriptor = std::fs::read(dir.join("agent-session.json")).unwrap();
                assert_ne!(replacement_descriptor, agent_descriptor);
                let endpoint: serde_json::Value = serde_json::from_slice(&replacement_descriptor).unwrap();
                let port = endpoint["port"].as_u64().unwrap();
                let status: serde_json::Value = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap()
                    .get(format!("http://127.0.0.1:{port}/v1/status")).send().await.unwrap().json().await.unwrap();
                assert_eq!(status["instance"], endpoint["instance"]);
                let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
                assert_eq!(client.post(format!("http://127.0.0.1:{port}/v1/authorization-status"))
                    .json(&serde_json::json!({"token": "a".repeat(64)})).send().await.unwrap().status(),
                    reqwest::StatusCode::SERVICE_UNAVAILABLE);
                let app_handle = app.handle().clone();
                assert!(tauri::async_runtime::spawn_blocking(move || {
                    app_handle.state::<crate::linux_single_instance::InstanceOwner>().owns_name().unwrap()
                }).await.unwrap());
                tauri::async_runtime::spawn_blocking(|| {
                    crate::linux_single_instance::test_callback(&zbus::blocking::Connection::session().unwrap(),
                        "com.verenu.app.SingleInstance", "/com/verenu/app/SingleInstance",
                        vec!["--verenu-hotkey-handsfree".into()]).unwrap();
                }).await.unwrap();
                assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks);
                let error = tauri::async_runtime::spawn_blocking(move || staged.activate_for_test())
                    .await.unwrap().expect_err("unfinished activation must fail safely");
                assert!(error.contains("Synthetic replacement startup failure"), "unexpected activation failure: {error}");
                drop(tokio::time::timeout(Duration::from_secs(20), gate.enter()).await
                    .expect("failed activation did not restore the parent"));
                assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks + 1);
                assert_eq!(std::fs::read(dir.join("agent-session.json")).unwrap(), agent_descriptor);
                assert_eq!(std::fs::read(dir.join("settings.json")).unwrap(), settings_before);
                drop(disk_db);
            }

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
            assert!(gate.try_enter().is_none());
            drop(activity);
            let resumed = tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap();
            drop(resumed);
            assert!(agent.clone().try_acquire_owned().is_ok());

            crate::agent_api::set_test_pending_approval(app.handle(), true);
            assert!(prepare(app.handle()).await.is_err());
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());
            crate::agent_api::set_test_pending_approval(app.handle(), false);

            recording.lock().unwrap().lifecycle = DictationLifecycle::Starting {
                prepend_audio: None,
            };
            assert!(prepare(app.handle()).await.is_err());
            drop(tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap());
            assert!(agent.clone().try_acquire_owned().is_ok());
            recording.lock().unwrap().lifecycle = DictationLifecycle::Idle;
            db.lock()
                .unwrap()
                .pragma_update(None, "user_version", 1)
                .unwrap();
            assert!(prepare(app.handle()).await.is_err());
            drop(tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap());
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
            drop(tokio::time::timeout(Duration::from_secs(10), gate.enter()).await.unwrap());
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
