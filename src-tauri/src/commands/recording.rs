use super::*;
use crate::core::window_geometry::{capture_webview_center, WindowTarget};

fn lock_state<'a>(
    state: &'a tauri::State<'_, SharedState>,
) -> Result<std::sync::MutexGuard<'a, pipeline::AppState>, String> {
    state
        .lock()
        .map_err(|_| "Recording state lock was poisoned".to_string())
}

fn capture_in_app_target(app: &AppHandle) -> WindowTarget {
    let mut target = WindowTarget::capture_display_only();
    if let Some(window) = app.get_webview_window("main") {
        if let Some(display_point) = capture_webview_center(&window) {
            target.display_point = Some(display_point);
        }
    }
    target
}

#[tauri::command]
pub async fn get_microphones() -> Vec<String> {
    match tokio::task::spawn_blocking(audio::list_input_devices).await {
        Ok(devices) => devices,
        Err(e) => {
            log::error!("Task to get microphones panicked: {e}");
            Vec::new()
        }
    }
}

#[tauri::command]
pub async fn start_input_recording(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    pipeline::reserve_starting(state.inner())?;

    let target = capture_in_app_target(&app);
    {
        let mut st = match lock_state(&state) {
            Ok(st) => st,
            Err(err) => {
                pipeline::release_starting_reservation(state.inner());
                return Err(err);
            }
        };
        st.target = target;
        st.pill_placement_stale = true;
    }

    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let start_result = tokio::task::spawn_blocking(move || {
        pipeline::start_recording_session_ex(
            &app_clone,
            &state_clone,
            "recording",
            false,
            pipeline::RecordingStartOptions {
                show_recording_pill: true,
                emit_globally: false,
                start_cue_delay_ms: None,
                durable: false,
            },
        )
    })
    .await;

    let start_result = match start_result {
        Ok(result) => result,
        Err(e) => {
            pipeline::release_starting_reservation(state.inner());
            return Err(format!("Recording task panicked: {e}"));
        }
    };

    match start_result {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = format!(
                "Failed to start recording: {}",
                crate::api::user_facing_message(&e)
            );
            crate::pipeline::hide_pill(&app);
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn start_setup_try_recording(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    pipeline::reserve_starting(state.inner())?;

    let target = capture_in_app_target(&app);
    {
        let mut st = match lock_state(&state) {
            Ok(st) => st,
            Err(err) => {
                pipeline::release_starting_reservation(state.inner());
                return Err(err);
            }
        };
        st.target = target;
        st.pill_placement_stale = true;
    }

    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let start_result = tokio::task::spawn_blocking(move || {
        pipeline::start_recording_session_ex(
            &app_clone,
            &state_clone,
            "recording",
            false,
            pipeline::RecordingStartOptions {
                show_recording_pill: true,
                emit_globally: false,
                start_cue_delay_ms: if pipeline::start_stop_sounds_enabled(&app_clone) {
                    Some(0)
                } else {
                    None
                },
                durable: false,
            },
        )
    })
    .await;

    let start_result = match start_result {
        Ok(result) => result,
        Err(e) => {
            pipeline::release_starting_reservation(state.inner());
            return Err(format!("Recording task panicked: {e}"));
        }
    };

    match start_result {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = format!(
                "Failed to start recording: {}",
                crate::api::user_facing_message(&e)
            );
            crate::pipeline::hide_pill(&app);
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn stop_setup_try_recording(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    if pipeline::cancel_starting_reservation(state.inner()) {
        pipeline::hide_pill(&app);
        return Ok(());
    }
    crate::core::hotkey::set_handless_active(false);
    tauri::async_runtime::spawn(pipeline::run_pipeline_event_only(
        app,
        state.inner().clone(),
    ));
    Ok(())
}

#[tauri::command]
pub async fn stop_and_transcribe_input(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<String, String> {
    pipeline::transcribe_input_only(app, state.inner().clone())
        .await
        .map_err(|e| crate::api::user_facing_error(&e))
}

#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    crate::core::hotkey::set_handless_active(false);
    let taken = pipeline::take_recording_for_cancel(state.inner());
    if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() {
        manager.set_recording_active(false);
    }
    if let Some((session, exclusive_mic_session_id, prepend_audio, recording_context)) = taken {
        let app_for_cancel = app.clone();
        let state_for_cancel = state.inner().clone();
        let handle = tauri::async_runtime::spawn(async move {
            pipeline::cancel_recording_with_resume(
                &app_for_cancel,
                &state_for_cancel,
                session,
                exclusive_mic_session_id,
                prepend_audio,
                recording_context,
            )
            .await;
        });
        tauri::async_runtime::spawn(async move {
            if handle.await.is_err() {
                log::error!("cancel_recording_with_resume task panicked or was aborted");
            }
        });
    } else {
        pipeline::hide_pill(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_handless_mode(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    crate::core::hotkey::set_handless_active(false);
    crate::core::hotkey::reset_chord_state();
    let has_session = lock_state(&state)?.lifecycle.is_recording();
    if has_session {
        tauri::async_runtime::spawn(pipeline::run_pipeline(app, state.inner().clone()));
    } else {
        pipeline::cancel_starting_reservation(state.inner());
        pipeline::hide_pill(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn resume_cancelled_capture(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    pipeline::reserve_starting(state.inner())?;
    let Some(capture) = pipeline::peek_cancelled_capture_if_fresh(state.inner()) else {
        pipeline::cancel_starting_reservation(state.inner());
        return Err("Nothing to resume".to_string());
    };
    pipeline::set_starting_prepend_audio(state.inner(), capture.audio.clone());
    let target = if capture.target.id == 0 {
        WindowTarget::capture_foreground()
    } else {
        capture.target.refreshed()
    };
    {
        let mut st = match lock_state(&state) {
            Ok(st) => st,
            Err(err) => {
                pipeline::cancel_starting_reservation(state.inner());
                return Err(err);
            }
        };
        st.target = target;
        st.pill_placement_stale = true;
        st.failover_session_id = Some(capture.id.clone());
        st.failover_reuse_id = true;
        st.failover_started_at_unix = capture.started_at_unix;
    }
    let start_result = pipeline::start_recording_session_ex_with_context(
        &app,
        state.inner(),
        "handsfree",
        true,
        pipeline::RecordingStartOptions {
            show_recording_pill: true,
            emit_globally: false,
            start_cue_delay_ms: if pipeline::start_stop_sounds_enabled(&app) {
                Some(crate::media::sound::START_CUE_HANDSFREE_DELAY_MS)
            } else {
                None
            },
            durable: true,
        },
        Some(capture.context.clone()),
    );
    match start_result {
        Ok(()) => {
            crate::core::hotkey::set_handless_active(true);
            pipeline::failover::retire_committed();
            pipeline::clear_cancelled_capture(state.inner());
            pipeline::emit_cancelled_capture_cleared(&app);
            Ok(())
        }
        Err(err) => {
            pipeline::cancel_starting_reservation(state.inner());
            if let Ok(mut st) = lock_state(&state) {
                st.failover_session_id = None;
                st.failover_reuse_id = false;
                st.failover_started_at_unix = 0;
            }
            pipeline::failover::abandon_live();
            Err(err)
        }
    }
}

#[derive(serde::Serialize)]
pub struct CancelledCaptureStatus {
    pub created_at: String,
    pub kind: String,
}

#[tauri::command]
pub fn get_cancelled_capture(
    state: tauri::State<'_, SharedState>,
) -> Result<Option<CancelledCaptureStatus>, String> {
    Ok(
        pipeline::peek_cancelled_capture_if_fresh(state.inner()).map(|capture| {
            CancelledCaptureStatus {
                created_at: capture.created_at_rfc3339,
                kind: capture.origin.as_str().to_string(),
            }
        }),
    )
}

#[tauri::command]
pub async fn dismiss_cancelled_capture(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    if pipeline::take_cancelled_capture_if_fresh(state.inner()).is_some() {
        pipeline::failover::discard_durable();
        pipeline::emit_cancelled_capture_cleared(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_paste_failure_to_clipboard(
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let Some(text) = pipeline::peek_paste_failure_if_fresh(state.inner()) else {
        return Err("Nothing to copy".to_string());
    };
    crate::core::injection::copy_to_clipboard(&text)
        .await
        .map_err(|e| e.to_string())?;
    pipeline::clear_paste_failure(state.inner());
    Ok(())
}

#[tauri::command]
pub fn set_pill_interactive(
    app: AppHandle,
    interactive: bool,
    expected_state: Option<String>,
) -> Result<(), String> {
    crate::pipeline::set_pill_interactive(&app, interactive, expected_state.as_deref());
    Ok(())
}

#[cfg(all(feature = "native-testing", debug_assertions, desktop))]
#[tauri::command]
pub fn native_test_pill(
    app: AppHandle,
    state: Option<String>,
    message: Option<String>,
    context: Option<String>,
) -> Result<serde_json::Value, String> {
    crate::pipeline::native_test_pill(
        &app,
        state.as_deref(),
        message.as_deref(),
        context.as_deref(),
    )
}

#[tauri::command]
pub fn hide_dictation_pill(app: AppHandle) -> Result<(), String> {
    crate::pipeline::hide_pill(&app);
    Ok(())
}

#[tauri::command]
pub fn set_pill_size(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    {
        let Some(pill) = app.get_webview_window("pill") else {
            return Ok(());
        };

        let width_points = if width.is_finite() {
            width.clamp(60.0, 440.0).round()
        } else {
            60.0
        };
        let height_points = if height.is_finite() {
            height.clamp(44.0, 420.0).round()
        } else {
            44.0
        };

        let placement =
            crate::pipeline::placement_for_current_monitor(&pill, width_points, height_points)
                .unwrap_or_else(|| {
                    let scale = pill.scale_factor().unwrap_or(1.0).max(0.1);
                    let cur_pos = pill.outer_position().unwrap_or_default();
                    let w = (width_points * scale).round() as i32;
                    let h = (height_points * scale).round() as i32;
                    let (cur_w, cur_h) = pill
                        .outer_size()
                        .map(|size| (size.width as f64, size.height as f64))
                        .unwrap_or((w as f64, h as f64));
                    crate::pipeline::PillPlacement {
                        x: cur_pos.x + ((cur_w - w as f64) / 2.0).round() as i32,
                        y: cur_pos.y + (cur_h - h as f64).round() as i32,
                        width: w,
                        height: h,
                    }
                });
        let mut st = lock_state(&state)?;
        st.pill_width_points = width_points;
        st.pill_height_points = height_points;
        st.pill_placement = Some(placement);
        drop(st);
        crate::pipeline::apply_pill_placement(&pill, placement);
        Ok(())
    }
}

#[tauri::command]
pub fn set_pill_hit_rect(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    crate::pipeline::set_pill_hit_rect(&app, x, y, width, height);
    Ok(())
}
