use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};

#[allow(dead_code)]
pub(crate) struct PreparedHandoff {
    _pause: crate::linux_activity::Pause,
    _agent: tokio::sync::OwnedSemaphorePermit,
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
    drop(recording);

    let sync = app
        .try_state::<crate::sync::SyncManager>()
        .ok_or("Sync state is unavailable; quit and reopen Verenu to finish updating.")?;
    sync.check_handoff_idle(&pause)
        .map_err(|error| error.to_string())?;
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
    drop(conn);
    crate::data::store::SettingsSnapshot::read_for_handoff(
        &crate::app_data_dir().join("settings.json"),
    )?;
    Ok(PreparedHandoff {
        _pause: pause,
        _agent: agent_permit,
    })
}

#[cfg(all(test, feature = "native-testing"))]
mod tests {
    use super::*;
    use tauri::Manager;

    #[test]
    #[ignore = "requires an isolated graphical session"]
    fn preparation_drains_and_restores_services_on_failure() {
        use crate::pipeline::{AppState, DictationLifecycle, SharedState};
        use std::sync::Mutex;
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
        let agent = Arc::new(tokio::sync::Semaphore::new(1));
        app.manage(crate::agent_api::AgentDrain::for_test(agent.clone()));
        crate::agent_api::manage_test_approvals(app.handle());
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

        tauri::async_runtime::block_on(async {
            let active_agent = agent.clone().acquire_owned().await.unwrap();
            let preparation = prepare(app.handle());
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
            assert!(agent.clone().try_acquire_owned().is_err());
            drop(lease);
            assert!(gate.try_enter().is_some());
            assert!(agent.clone().try_acquire_owned().is_ok());

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
        });
        match old_dir {
            Some(value) => std::env::set_var("VERENU_APP_DATA_DIR_OVERRIDE", value),
            None => std::env::remove_var("VERENU_APP_DATA_DIR_OVERRIDE"),
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
