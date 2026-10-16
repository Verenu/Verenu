use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use crate::core::window_geometry::WindowTarget;
use crate::pipeline::{self, hide_pill, start_recording_session, AppState, SharedState};
use crate::DbHandle;
use tauri::{Emitter, Manager};

fn lock_app_state(state: &SharedState) -> Option<MutexGuard<'_, AppState>> {
    match state.lock() {
        Ok(guard) => Some(guard),
        Err(_) => {
            log::error!("Recording state lock was poisoned");
            None
        }
    }
}

#[derive(Clone, Copy)]
enum HotkeyEvent {
    Press,
    Release,
    HandlessToggle,
    Cancel,
    #[allow(dead_code)]
    ChordCancel,
    EscapeCancel,
    CopyLast,
    CaptureSubApp,
}

const HANDSFREE_STOP_GUARD: Duration = Duration::from_millis(350);

/// Space can convert a hold-to-talk recording into hands-free while the
/// original chord is still physically held. Windows can report one of that
/// chord's release events after the Space toggle has already reached the
/// async dispatcher, so give the conversion a short handoff window in which
/// release/cancel events cannot stop the new hands-free recording.
const HANDSFREE_CONVERSION_GUARD: Duration = Duration::from_secs(3);

#[derive(Default)]
struct HandsfreeStopGuard {
    until: Option<Instant>,
}

impl HandsfreeStopGuard {
    fn arm(&mut self, now: Instant) {
        self.until = Some(now + HANDSFREE_STOP_GUARD);
    }

    fn suppresses(&mut self, event: HotkeyEvent, now: Instant) -> bool {
        let Some(until) = self.until else {
            return false;
        };
        if now >= until {
            self.until = None;
            return false;
        }
        matches!(
            event,
            HotkeyEvent::Press
                | HotkeyEvent::Release
                | HotkeyEvent::HandlessToggle
                | HotkeyEvent::Cancel
                | HotkeyEvent::ChordCancel
        )
    }
}

#[derive(Default)]
struct HandsfreeConversionGuard {
    until: Option<Instant>,
}

impl HandsfreeConversionGuard {
    fn arm(&mut self, now: Instant) {
        self.until = Some(now + HANDSFREE_CONVERSION_GUARD);
    }

    fn expired(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now >= until)
    }

    fn suppresses(&mut self, event: HotkeyEvent, now: Instant) -> bool {
        let Some(until) = self.until else {
            return false;
        };
        if now >= until {
            self.until = None;
            return false;
        }
        matches!(event, HotkeyEvent::Release | HotkeyEvent::Cancel)
    }
}

pub(crate) fn setup_hotkey(app: &mut tauri::App, shared: SharedState) {
    #[cfg(target_os = "linux")]
    crate::core::hotkey::shortcut_status::initialize(app.handle().clone());
    #[cfg(target_os = "linux")]
    if let Err(error) = crate::commands::apply_linux_control_keys(app.handle()) {
        log::warn!("Could not load secondary shortcuts: {error}");
    }
    #[cfg(target_os = "linux")]
    crate::core::atspi::ensure_accessibility_enabled();
    // The WH_KEYBOARD_LL hook callback must return within Windows' hook timeout
    // (~300ms) or the hook is silently removed. All real work happens in a Tokio
    // task below; callbacks only send a lightweight channel message.
    let (hotkey_tx, mut hotkey_rx) = tokio::sync::mpsc::unbounded_channel::<HotkeyEvent>();
    let tx_press = hotkey_tx.clone();
    let tx_handless = hotkey_tx.clone();
    let tx_cancel = hotkey_tx.clone();
    let tx_escape = hotkey_tx.clone();
    let tx_copy_last = hotkey_tx.clone();
    let tx_sub_app = hotkey_tx.clone();
    let tx_release = hotkey_tx;

    #[cfg(target_os = "linux")]
    {
        let tx_chord_cancel = tx_cancel.clone();
        crate::core::hotkey::set_chord_cancel_callback(move || {
            let _ = tx_chord_cancel.send(HotkeyEvent::ChordCancel);
        });
    }

    match crate::core::hotkey::start(
        move || {
            let _ = tx_press.send(HotkeyEvent::Press);
        },
        move || {
            let _ = tx_release.send(HotkeyEvent::Release);
        },
        move || {
            let _ = tx_handless.send(HotkeyEvent::HandlessToggle);
        },
        move || {
            let _ = tx_cancel.send(HotkeyEvent::Cancel);
        },
        move || {
            let _ = tx_escape.send(HotkeyEvent::EscapeCancel);
        },
        move || {
            let _ = tx_copy_last.send(HotkeyEvent::CopyLast);
        },
        move || {
            let _ = tx_sub_app.send(HotkeyEvent::CaptureSubApp);
        },
    ) {
        Ok(_handle) => {
            log::info!("hotkey: hook installed");
            crate::commands::apply_sub_app_capture_hotkey(app.handle());
        }
        Err(e) => {
            log::error!("Hotkey hook failed to start: {e}");
            let app_h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                app_h
                    .emit(
                        "verenu:error",
                        format!("Keyboard hook failed to install — hotkey unavailable. {e}"),
                    )
                    .ok();
            });
            return;
        }
    }

    let app_hk = app.handle().clone();
    let state_hk = shared;

    tauri::async_runtime::spawn(async move {
        let mut handsfree_stop_guard = HandsfreeStopGuard::default();
        let mut handsfree_conversion_guard = HandsfreeConversionGuard::default();
        while let Some(event) = hotkey_rx.recv().await {
            let now = Instant::now();
            if handsfree_conversion_guard.expired(now) {
                pipeline::clear_handless_hold_marker(&state_hk);
            }
            if handsfree_conversion_guard.suppresses(event, now) {
                pipeline::clear_handless_hold_marker(&state_hk);
                log::debug!("hotkey: ignored stale input after Space hands-free conversion");
                continue;
            }
            if handsfree_stop_guard.suppresses(event, now) {
                log::debug!("hotkey: ignored follow-up input after hands-free stop");
                continue;
            }
            match event {
                HotkeyEvent::Press => {
                    pipeline::clear_handless_hold_marker(&state_hk);

                    #[allow(clippy::large_enum_variant)]
                    enum PressAction {
                        None,
                        Fresh,
                        Interrupt(pipeline::ActivePipeline),
                    }
                    let action = {
                        let Some(st) = lock_app_state(&state_hk) else {
                            continue;
                        };
                        match &st.lifecycle {
                            pipeline::DictationLifecycle::Idle => PressAction::Fresh,
                            pipeline::DictationLifecycle::Processing(_) => {
                                drop(st);
                                match pipeline::take_active_pipeline_for_interrupt(&state_hk) {
                                    Some(active) => PressAction::Interrupt(active),
                                    None => PressAction::None,
                                }
                            }
                            _ => PressAction::None,
                        }
                    };
                    match action {
                        PressAction::None => {}
                        PressAction::Fresh => {
                            if pipeline::reserve_starting(&state_hk).is_ok() {
                                let target = WindowTarget::capture_foreground();
                                if let Some(mut st) = lock_app_state(&state_hk) {
                                    st.target = target;
                                    st.pill_placement_stale = true;
                                }
                                start_recording_session(&app_hk, &state_hk, "recording", false);
                            }
                        }
                        PressAction::Interrupt(active) => {
                            let _ = active.cancel_tx.send(true);
                            let target = WindowTarget::capture_foreground();
                            if let Some(mut st) = lock_app_state(&state_hk) {
                                st.target = target;
                                st.pill_placement_stale = true;
                            }
                            start_recording_session(&app_hk, &state_hk, "recording", false);
                        }
                    }
                }

                HotkeyEvent::Release => {
                    if pipeline::consume_handless_hold_stop(&state_hk) {
                        log::debug!(
                            "hotkey: ignored stale hold release after Space hands-free conversion"
                        );
                        continue;
                    }

                    tauri::async_runtime::spawn(pipeline::run_pipeline(
                        app_hk.clone(),
                        state_hk.clone(),
                    ));
                }

                HotkeyEvent::HandlessToggle => {
                    let (is_handless, has_session) = {
                        let Some(st) = lock_app_state(&state_hk) else {
                            continue;
                        };
                        (
                            st.lifecycle.is_handless_recording(),
                            st.lifecycle.is_recording(),
                        )
                    };
                    if is_handless {
                        crate::core::hotkey::set_handless_active(false);
                        if has_session {
                            tauri::async_runtime::spawn(pipeline::run_pipeline(
                                app_hk.clone(),
                                state_hk.clone(),
                            ));
                        } else {
                            crate::system::media_control::end_dictation_media_pause();
                        }
                    } else if has_session && pipeline::promote_recording_to_handless(&state_hk) {
                        handsfree_conversion_guard.arm(Instant::now());
                        crate::core::hotkey::set_handless_active(true);
                        pipeline::update_pill_state(&app_hk, "handsfree");
                    } else if !has_session {
                        let started = {
                            let Some(st) = lock_app_state(&state_hk) else {
                                continue;
                            };
                            match &st.lifecycle {
                                pipeline::DictationLifecycle::Idle => {
                                    drop(st);
                                    pipeline::reserve_starting(&state_hk).is_ok()
                                }
                                pipeline::DictationLifecycle::Processing(_) => {
                                    drop(st);
                                    match pipeline::take_active_pipeline_for_interrupt(&state_hk) {
                                        Some(active) => {
                                            let _ = active.cancel_tx.send(true);
                                            true
                                        }
                                        None => false,
                                    }
                                }
                                _ => false,
                            }
                        };
                        if started {
                            let carried = pipeline::peek_cancelled_capture_if_fresh(&state_hk)
                                .filter(|capture| {
                                    capture.captured_at.elapsed()
                                        < std::time::Duration::from_secs(5)
                                });
                            if let Some(capture) = &carried {
                                pipeline::set_starting_prepend_audio(
                                    &state_hk,
                                    capture.audio.clone(),
                                );
                            }
                            let target = WindowTarget::capture_foreground();
                            if let Some(mut st) = lock_app_state(&state_hk) {
                                st.target = target;
                                st.pill_placement_stale = true;
                            }
                            start_recording_session(&app_hk, &state_hk, "handsfree", true);
                            crate::core::hotkey::set_handless_active(true);
                            if carried.is_some()
                                && lock_app_state(&state_hk)
                                    .is_some_and(|st| st.lifecycle.is_recording())
                            {
                                pipeline::clear_cancelled_capture(&state_hk);
                                pipeline::emit_cancelled_capture_cleared(&app_hk);
                            }
                        }
                    }
                }

                HotkeyEvent::ChordCancel
                    if lock_app_state(&state_hk).is_none_or(|st| {
                        !st.lifecycle.is_recording() || st.lifecycle.is_handless_recording()
                    }) =>
                {
                    continue;
                }

                HotkeyEvent::Cancel | HotkeyEvent::ChordCancel => {
                    crate::media::sound::cancel_pending_start();
                    if pipeline::consume_handless_hold_stop(&state_hk) {
                        log::debug!(
                            "hotkey: ignored stale hold cancel after Space hands-free conversion"
                        );
                        continue;
                    }
                    let is_handless = {
                        let Some(st) = lock_app_state(&state_hk) else {
                            continue;
                        };
                        st.lifecycle.is_handless_recording()
                    };
                    if is_handless {
                        handsfree_stop_guard.arm(Instant::now());
                        crate::core::hotkey::set_handless_active(false);
                        crate::core::hotkey::reset_chord_state();
                        tauri::async_runtime::spawn(pipeline::run_pipeline(
                            app_hk.clone(),
                            state_hk.clone(),
                        ));
                    } else {
                        let taken = pipeline::take_recording_plain_with_prepend(&state_hk);
                        if let Some((
                            session,
                            exclusive_mic_session_id,
                            prepend_audio,
                            recording_context,
                        )) = taken
                        {
                            let app_for_cancel = app_hk.clone();
                            let state_for_cancel = state_hk.clone();
                            tauri::async_runtime::spawn(async move {
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
                        } else {
                            hide_pill(&app_hk);
                        }
                    }
                }

                HotkeyEvent::EscapeCancel => {
                    crate::core::hotkey::set_handless_active(false);
                    crate::media::sound::cancel_pending_start();

                    if let Some(active) = pipeline::take_active_pipeline_for_escape(&state_hk) {
                        let _ = active.cancel_tx.send(true);
                        pipeline::stash_cancelled_capture(
                            &app_hk,
                            &state_hk,
                            active.captured_audio,
                            pipeline::CaptureOrigin::UserCancelled,
                            active.context,
                        );
                        continue;
                    }

                    let taken = pipeline::take_recording_plain_with_prepend(&state_hk);
                    if let Some((
                        session,
                        exclusive_mic_session_id,
                        prepend_audio,
                        recording_context,
                    )) = taken
                    {
                        let app_for_cancel = app_hk.clone();
                        let state_for_cancel = state_hk.clone();
                        tauri::async_runtime::spawn(async move {
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
                    } else {
                        if pipeline::take_cancelled_capture_if_fresh(&state_hk).is_some() {
                            pipeline::failover::discard_durable();
                            pipeline::emit_cancelled_capture_cleared(&app_hk);
                        }
                        hide_pill(&app_hk);
                    }
                }

                HotkeyEvent::CopyLast => {
                    let Some(db_handle) = app_hk.try_state::<DbHandle>() else {
                        log::warn!("CopyLast hotkey: DbHandle not managed, skipping");
                        continue;
                    };
                    let db_handle = db_handle.inner().clone();
                    let app_for_copy = app_hk.clone();
                    tauri::async_runtime::spawn(async move {
                        let recent = tokio::task::spawn_blocking(move || {
                            crate::data::db::query_recent_page(&db_handle, 1, 0, None, None)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .unwrap_or_default();
                        match recent.into_iter().next() {
                            Some(entry) if !entry.clean_text.trim().is_empty() => {
                                if let Err(e) =
                                    crate::core::injection::copy_to_clipboard(&entry.clean_text)
                                        .await
                                {
                                    log::warn!(
                                        "hotkey: copy-last-dictation clipboard write failed: {e}"
                                    );
                                    pipeline::show_copied_pill(
                                        &app_for_copy,
                                        "Failed to copy last dictation",
                                    );
                                    return;
                                }
                                pipeline::show_copied_pill(
                                    &app_for_copy,
                                    "Copied last dictation to clipboard",
                                );
                            }
                            _ => {
                                pipeline::show_copied_pill(&app_for_copy, "Nothing to copy yet");
                            }
                        }
                    });
                }

                HotkeyEvent::CaptureSubApp => {
                    let app = app_hk.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        crate::commands::handle_capture_hotkey(&app);
                    });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handsfree_stop_guard_consumes_the_followup_click_but_not_escape() {
        let now = Instant::now();
        let mut guard = HandsfreeStopGuard::default();
        guard.arm(now);

        for event in [
            HotkeyEvent::Press,
            HotkeyEvent::Release,
            HotkeyEvent::HandlessToggle,
            HotkeyEvent::Cancel,
            HotkeyEvent::ChordCancel,
        ] {
            assert!(guard.suppresses(event, now + Duration::from_millis(1)));
        }
        assert!(!guard.suppresses(HotkeyEvent::EscapeCancel, now + Duration::from_millis(1)));
        assert!(!guard.suppresses(HotkeyEvent::CopyLast, now + Duration::from_millis(1)));
    }

    #[test]
    fn handsfree_stop_guard_expires_after_the_double_tap_window() {
        let now = Instant::now();
        let mut guard = HandsfreeStopGuard::default();
        guard.arm(now);

        assert!(!guard.suppresses(
            HotkeyEvent::Press,
            now + HANDSFREE_STOP_GUARD + Duration::from_millis(1),
        ));
        assert!(!guard.suppresses(
            HotkeyEvent::Press,
            now + HANDSFREE_STOP_GUARD + Duration::from_millis(2),
        ));
    }

    #[test]
    fn handsfree_conversion_guard_swallows_stale_stop_events_for_three_seconds() {
        let now = Instant::now();
        let mut guard = HandsfreeConversionGuard::default();
        guard.arm(now);

        assert!(guard.suppresses(HotkeyEvent::Release, now + Duration::from_secs(1)));
        assert!(guard.suppresses(HotkeyEvent::Cancel, now + Duration::from_secs(2)));
        assert!(!guard.suppresses(
            HotkeyEvent::Cancel,
            now + HANDSFREE_CONVERSION_GUARD + Duration::from_millis(1),
        ));
    }

    #[test]
    fn handsfree_conversion_guard_does_not_block_escape_or_toggle_events() {
        let now = Instant::now();
        let mut guard = HandsfreeConversionGuard::default();
        guard.arm(now);

        assert!(!guard.suppresses(HotkeyEvent::EscapeCancel, now + Duration::from_millis(1)));
        assert!(!guard.suppresses(HotkeyEvent::HandlessToggle, now + Duration::from_millis(1)));
        assert!(!guard.suppresses(HotkeyEvent::CopyLast, now + Duration::from_millis(1)));
    }
}
