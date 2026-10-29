use super::*;
use crate::core::context::ResolvedContextIdentity;
use chrono::{SecondsFormat, Utc};

#[cfg(target_os = "linux")]
fn output_volume_policy(legacy_mute: bool, enabled: bool, target: u8) -> (bool, bool) {
    let mute = legacy_mute || (enabled && target == 0);
    let reduce = enabled && (1..100).contains(&target) && !mute;
    (mute, reduce)
}

#[cfg(all(test, target_os = "linux"))]
mod output_volume_policy_tests {
    use super::output_volume_policy;

    #[test]
    fn slider_and_legacy_mute_select_one_output_path() {
        assert_eq!(output_volume_policy(false, false, 25), (false, false));
        assert_eq!(output_volume_policy(false, true, 0), (true, false));
        assert_eq!(output_volume_policy(false, true, 30), (false, true));
        assert_eq!(output_volume_policy(false, true, 100), (false, false));
        assert_eq!(output_volume_policy(true, true, 30), (true, false));
    }
}

#[derive(Clone, Copy, Default)]
pub struct RecordingStartOptions {
    pub show_recording_pill: bool,
    pub emit_globally: bool,
    pub start_cue_delay_ms: Option<u64>,
    pub durable: bool,
}

pub fn start_recording_session(
    app: &AppHandle,
    state: &SharedState,
    pill_state: &str,
    handless: bool,
) {
    let start_cue_delay_ms = if start_stop_sounds_enabled(app) {
        Some(if handless {
            crate::media::sound::START_CUE_HANDSFREE_DELAY_MS
        } else {
            crate::media::sound::START_CUE_NORMAL_DELAY_MS
        })
    } else {
        None
    };

    if let Err(e) = start_recording_session_ex(
        app,
        state,
        pill_state,
        handless,
        RecordingStartOptions {
            show_recording_pill: true,
            emit_globally: false,
            start_cue_delay_ms,
            durable: pill_state == "recording" || pill_state == "handsfree",
        },
    ) {
        log::error!("start recording: {e}");
        hide_pill(app);
        app.emit(
            "verenu:error",
            format!(
                "Failed to start recording: {}",
                crate::api::user_facing_message(&e)
            ),
        )
        .ok();
    }
}

pub fn start_recording_session_ex(
    app: &AppHandle,
    state: &SharedState,
    pill_state: &str,
    handless: bool,
    options: RecordingStartOptions,
) -> Result<(), String> {
    start_recording_session_ex_with_context(app, state, pill_state, handless, options, None)
}

pub fn start_recording_session_ex_with_context(
    app: &AppHandle,
    state: &SharedState,
    pill_state: &str,
    handless: bool,
    options: RecordingStartOptions,
    captured_context: Option<ResolvedContextIdentity>,
) -> Result<(), String> {
    let settings = store::settings_snapshot(app);
    let audio_config = match settings {
        Ok(ref settings) => store::load_audio_config(settings),
        Err(e) => {
            log::warn!(
                "Failed to load settings.json store for audio config: {:?}",
                e
            );
            store::AudioConfig::default()
        }
    };

    #[cfg(target_os = "macos")]
    {
        if !crate::system::mac_app::is_accessibility_verified()
            && !crate::commands::check_accessibility_permission(false)
        {
            release_starting_reservation(state);
            return Err(
                "Accessibility permission is required for Verenu on macOS. Open System Settings > Privacy & Security > Accessibility and enable Verenu."
                    .to_string(),
            );
        }

        match crate::system::mac_app::microphone_permission_status() {
            crate::system::mac_app::MicrophonePermission::Denied
            | crate::system::mac_app::MicrophonePermission::Restricted => {
                release_starting_reservation(state);
                return Err(
                    "Microphone access is blocked on macOS. Open System Settings > Privacy & Security > Microphone and enable Verenu."
                        .to_string(),
                );
            }
            _ => {}
        }
    }

    let device = audio_config.device;
    let use_default_input_device = device.is_none();
    let noise_reduction = audio_config.noise_reduction;
    let echo_cancellation = cfg!(target_os = "linux") && audio_config.echo_cancellation;
    #[cfg(target_os = "linux")]
    let (mute_audio, reduce_audio) = output_volume_policy(
        audio_config.mute_audio,
        audio_config.duck_audio,
        audio_config.duck_target,
    );
    #[cfg(not(target_os = "linux"))]
    let mute_audio = audio_config.mute_audio;
    let exclusive_mic = audio_config.exclusive_mic;
    let pause_media = audio_config.pause_media_during_dictation;
    let mic_gain = audio_config.mic_gain;
    #[cfg(target_os = "linux")]
    let duck_settings = reduce_audio.then_some(crate::system::output_duck::DuckSettings {
        target: audio_config.duck_target,
        down: std::time::Duration::from_millis(audio_config.duck_fade_down_ms),
        up: std::time::Duration::from_millis(audio_config.duck_fade_up_ms),
        smooth_down: true,
        smooth_up: audio_config.duck_fade_up_smooth,
    });
    let exclusive_mic_session_id =
        if cfg!(target_os = "macos") && exclusive_mic && use_default_input_device {
            Some(crate::system::volume::register_session())
        } else {
            None
        };

    let target = lock_state(state).map(|st| st.target.clone()).unwrap_or_default();
    let captured_context = captured_context.or_else(|| {
        lock_state(state)
            .ok()
            .and_then(|st| st.recording_context.clone())
    });
    let has_captured_context = captured_context.is_some();
    let recording_context = captured_context.unwrap_or_else(|| {
        emit_context_for_window(app, &target).unwrap_or_else(|| {
            let context = ResolvedContextIdentity::everywhere();
            crate::pipeline::pill::queue_pill_context(&context.label);
            context
        })
    });
    if has_captured_context {
        crate::pipeline::pill::queue_pill_context(&recording_context.label);
    }

    let (durable_id, prepend_for_lifecycle, mute_claim) = {
        let mut st = match lock_state(state) {
            Ok(st) => st,
            Err(e) => {
                if let Some(session_id) = exclusive_mic_session_id {
                    crate::system::volume::release_mic(session_id);
                }
                release_starting_reservation(state);
                return Err(e.to_string());
            }
        };
        let prepend_audio = match &st.lifecycle {
            DictationLifecycle::Starting { prepend_audio } => prepend_audio.clone(),
            _ => {
                log::warn!(
                    "start_recording_session_ex: lifecycle was not Starting when installing Recording"
                );
                drop(st);
                if let Some(session_id) = exclusive_mic_session_id {
                    crate::system::volume::release_mic(session_id);
                }
                release_starting_reservation(state);
                return Err(
                    "Recording start reservation was lost before the microphone opened".to_string(),
                );
            }
        };
        st.recording_context = Some(recording_context.clone());
        let durable_id = if options.durable {
            let id = if st.failover_reuse_id {
                st.failover_reuse_id = false;
                st.failover_session_id
                    .clone()
                    .unwrap_or_else(super::failover::new_session_id)
            } else {
                super::failover::new_session_id()
            };
            st.failover_session_id = Some(id.clone());
            if st.failover_started_at_unix == 0 {
                st.failover_started_at_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
            }
            Some(id)
        } else {
            st.failover_session_id = None;
            st.failover_reuse_id = false;
            st.failover_started_at_unix = 0;
            None
        };
        let mute_claim = mute_audio.then(crate::media::sound::claim_local_mute);
        (durable_id, prepend_audio, mute_claim)
    };
    let durable_sink = durable_id.and_then(|id| {
        super::failover::open_live_writer(
            id,
            prepend_for_lifecycle
                .as_ref()
                .map(|a| a.samples_16k.as_slice()),
            app,
            state,
        )
    });

    let prepend_samples = prepend_for_lifecycle
        .as_ref()
        .map(|audio| audio.samples_16k.len());
    let max_output_samples =
        prepend_samples.map(|samples| audio::MAX_RECORDING_SAMPLES.saturating_sub(samples));
    let (started, mute_report) = crate::media::mute_gate::start_after_mute(
        mute_claim.map(|claim| claim.task()),
        crate::media::mute_gate::MUTE_CONFIRM_TIMEOUT,
        || {
            audio::RecordingSession::start(
                device,
                noise_reduction,
                mic_gain,
                durable_sink,
                max_output_samples,
                echo_cancellation,
            )
        },
    );
    match started {
        #[allow(unused_mut)]
        Ok(mut session) => {
            #[cfg(target_os = "linux")]
            if let Some(settings) = duck_settings {
                if let Err(error) = session.start_output_duck(settings) {
                    log::warn!("Unable to lower output volume: {error}");
                }
            }
            let analytics_run_id: Option<String> = {
                #[cfg(desktop)]
                {
                    app.try_state::<crate::analytics::Analytics>()
                        .map(|analytics| analytics.new_run_id())
                }
                #[cfg(not(desktop))]
                {
                    None
                }
            };
            let level_arc = session.level.clone();
            let raw_level_arc = session.raw_level.clone();
            let envelope_arc = session.envelope.clone();
            let active_arc = session.active.clone();
            let speech_detected_arc = session.speech_detected.clone();
            let stream_error_arc = session.stream_error.clone();
            {
                let mut st = match lock_state(state) {
                    Ok(st) => st,
                    Err(e) => {
                        let _ = session.stop();
                        release_starting_reservation_for_claim(state, mute_claim);
                        if let Some(claim) = mute_claim {
                            claim.release();
                        }
                        if let Some(session_id) = exclusive_mic_session_id {
                            crate::system::volume::release_mic(session_id);
                        }
                        if options.durable {
                            super::failover::abandon_live();
                        }
                        return Err(e.to_string());
                    }
                };
                if !matches!(&st.lifecycle, DictationLifecycle::Starting { .. })
                    || mute_claim.is_some_and(|claim| !claim.is_current())
                {
                    log::warn!(
                        "start_recording_session_ex: lifecycle was not Starting when installing Recording"
                    );
                    drop(st);
                    let _ = session.stop();
                    if let Some(claim) = mute_claim {
                        claim.release();
                    }
                    if let Some(session_id) = exclusive_mic_session_id {
                        crate::system::volume::release_mic(session_id);
                    }
                    if options.durable {
                        super::failover::abandon_live();
                    }
                    return Err(
                        "Recording start reservation was lost before the microphone opened"
                            .to_string(),
                    );
                }
                st.lifecycle = DictationLifecycle::Recording {
                    session,
                    mute_claim,
                    exclusive_mic_session_id,
                    handless,
                    handless_from_hold: false,
                    prepend_audio: prepend_for_lifecycle.clone(),
                };
                st.analytics_run_id = analytics_run_id.clone();
            }
            #[cfg(desktop)]
            if let (Some(analytics), Some(run_id)) = (
                app.try_state::<crate::analytics::Analytics>(),
                analytics_run_id.as_deref(),
            ) {
                analytics.dictation_started(run_id, handless, noise_reduction);
                if noise_reduction {
                    analytics.feature_used(run_id, "noise_reduction");
                }
                if pause_media {
                    analytics.feature_used(run_id, "media_pause");
                }
                if handless {
                    analytics.feature_used(run_id, "hands_free");
                }
            }
            state::note_sensitivity_for_new_recording(state);
            if options.durable
                && prepend_for_lifecycle
                    .as_ref()
                    .is_some_and(|audio| audio.duration_ms >= MIN_RECORDING_MS)
            {
                super::failover::retire_committed();
                if let Ok(mut st) = lock_state(state) {
                    let current_id = st.failover_session_id.clone();
                    let stale = st
                        .cancelled_capture
                        .as_ref()
                        .is_some_and(|c| current_id.as_ref().is_some_and(|id| &c.id != id));
                    if stale {
                        st.cancelled_capture = None;
                        emit_cancelled_capture_cleared(app);
                    }
                }
            }
            if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() {
                manager.set_recording_active(true);
            }
            if let Some(manager) = app.try_state::<crate::sync::SyncManager>() {
                manager.monitor_muting();
            }
            if options.show_recording_pill {
                show_pill(app, pill_state);
            }
            spawn_stream_error_watcher(
                app.clone(),
                state.clone(),
                stream_error_arc,
                active_arc.clone(),
            );
            spawn_level_emitter(
                app.clone(),
                level_arc,
                raw_level_arc,
                envelope_arc,
                active_arc,
                speech_detected_arc,
                options.emit_globally,
            );
            if let Some(session_id) = exclusive_mic_session_id {
                tauri::async_runtime::spawn_blocking(move || {
                    crate::system::volume::hog_mic(session_id)
                });
            }
            if pause_media {
                crate::system::media_control::begin_dictation_media_pause();
            }
            if let Some(delay_ms) = options
                .start_cue_delay_ms
                .filter(|_| !mute_report.gate.is_confirmed())
            {
                crate::media::sound::play_start_delayed(delay_ms);
            }
            Ok(())
        }
        Err(e) => {
            if let Some(session_id) = exclusive_mic_session_id {
                crate::system::volume::release_mic(session_id);
            }
            if options.durable {
                super::failover::abandon_live();
            }
            release_starting_reservation_for_claim(state, mute_claim);
            if let Some(claim) = mute_claim {
                claim.release();
            }
            Err(e.to_string())
        }
    }
}

pub async fn cancel_recording_with_resume(
    app: &AppHandle,
    state: &SharedState,
    session: audio::RecordingSession,
    exclusive_mic_session_id: Option<u64>,
    prepend_audio: Option<CapturedAudio>,
    recording_context: ResolvedContextIdentity,
) {
    crate::system::media_control::end_dictation_media_pause();

    let Some(stopped_capture) =
        stop_and_capture_audio(app, session, exclusive_mic_session_id).await
    else {
        return;
    };
    if stopped_capture.recovery_write_failed {
        log::warn!("recording: cancelled capture not offered for resume after recovery failure");
        if state_is_idle(state) {
            hide_pill(app);
        }
        return;
    }
    let StoppedCapture {
        audio: mut captured_audio,
        mut rms,
        mut raw_rms,
        stream_error,
        ..
    } = stopped_capture;
    if stream_error {
        super::failover::abandon_live();
        if state_is_idle(state) {
            super::show_error_pill(app, super::stages_transcription::AUDIO_STREAM_ERROR_MESSAGE)
                .await;
        }
        return;
    }

    let active_gain = store::settings_snapshot(app)
        .map(|s| store::load_audio_config(&s).mic_gain)
        .unwrap_or(store::default_mic_gain());

    if let Some(previous) = prepend_audio {
        match super::merge_prepend_audio(previous, captured_audio, active_gain) {
            Ok((merged, merged_rms, merged_raw_rms)) => {
                captured_audio = merged;
                rms = merged_rms;
                raw_rms = merged_raw_rms;
            }
            Err(error) => {
                log::warn!("recording: cancelled continuation merge failed: {error}");
                super::failover::abandon_live();
                return;
            }
        }
    }

    let min_rms = recording_gate_rms(active_gain);
    let gate_rms = effective_recording_rms(rms, raw_rms, active_gain);

    if captured_audio.duration_ms < MIN_RECORDING_MS || gate_rms < min_rms {
        log::info!("recording: cancelled — capture discarded (duration/quiet gate)");
        super::failover::abandon_live();
        if start_stop_sounds_enabled(app) {
            crate::media::sound::play(crate::media::sound::SoundCue::Cancel);
        }
        if state_is_idle(state) {
            hide_pill(app);
        }
        return;
    }

    log::info!("recording: cancelled — capture stashed for resume");
    stash_cancelled_capture(
        app,
        state,
        captured_audio,
        CaptureOrigin::UserCancelled,
        recording_context,
    );
}

fn state_is_idle(state: &SharedState) -> bool {
    lock_state(state).is_ok_and(|st| st.lifecycle.is_idle())
}

pub fn stash_cancelled_capture(
    app: &AppHandle,
    state: &SharedState,
    audio: CapturedAudio,
    origin: CaptureOrigin,
    context: ResolvedContextIdentity,
) {
    #[allow(clippy::large_enum_variant)]
    enum StashOutcome {
        Stashed(CancelledCapture),
        LockPoisoned,
        SessionActive,
    }
    let outcome = match lock_state(state) {
        Ok(st) => {
            if !st.lifecycle.is_idle() {
                StashOutcome::SessionActive
            } else {
                let id = st
                    .failover_session_id
                    .clone()
                    .unwrap_or_else(super::failover::new_session_id);
                let started_at_unix = if st.failover_started_at_unix != 0 {
                    st.failover_started_at_unix
                } else {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0)
                };
                let kind = match origin {
                    CaptureOrigin::UserCancelled => super::failover::FailoverKind::Cancelled,
                    CaptureOrigin::Interrupted => super::failover::FailoverKind::Recording,
                };
                let target = st.target.clone();
                drop(st);
                super::failover::commit_capture_with_context(
                    &audio,
                    &id,
                    kind,
                    started_at_unix,
                    Some(context.id),
                );
                let capture = CancelledCapture {
                    audio,
                    captured_at: std::time::Instant::now(),
                    id,
                    origin,
                    created_at_rfc3339: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                    started_at_unix,
                    target,
                    context,
                };
                match lock_state(state) {
                    Ok(mut st) => {
                        if !st.lifecycle.is_idle() {
                            StashOutcome::SessionActive
                        } else {
                            st.cancelled_capture = Some(capture.clone());
                            st.failover_session_id = None;
                            st.failover_reuse_id = false;
                            st.failover_started_at_unix = 0;
                            StashOutcome::Stashed(capture)
                        }
                    }
                    Err(_) => StashOutcome::LockPoisoned,
                }
            }
        }
        Err(_) => StashOutcome::LockPoisoned,
    };
    match outcome {
        StashOutcome::Stashed(capture) => {
            #[cfg(desktop)]
            if let (Some(analytics), Some(run_id)) = (
                app.try_state::<crate::analytics::Analytics>(),
                super::state::analytics_run_id(state).as_deref(),
            ) {
                analytics.dictation_cancelled(run_id, true);
                analytics.delivery_outcome(run_id, "cancelled_user");
            }
            if start_stop_sounds_enabled(app) {
                crate::media::sound::play(crate::media::sound::SoundCue::Cancel);
            }
            emit_cancelled_capture(app, &capture);
            match capture.origin {
                CaptureOrigin::UserCancelled => show_cancelled_pill(app),
                CaptureOrigin::Interrupted => show_interrupted_pill(app),
            }
        }
        StashOutcome::SessionActive => {
            log::debug!("skipping cancelled-capture pill: a new session is active");
        }
        StashOutcome::LockPoisoned => {
            log::warn!("failed to stash cancelled capture (state lock poisoned)");
            hide_pill(app);
        }
    }
}

pub(crate) fn release_starting_reservation(state: &SharedState) {
    release_starting_reservation_for_claim(state, None);
}

pub(super) fn release_starting_reservation_for_claim(
    state: &SharedState,
    claim: Option<crate::media::sound::LocalMuteClaim>,
) {
    let mut st = match state.lock() {
        Ok(st) => st,
        Err(poisoned) => {
            log::warn!(
                "Recording state lock was poisoned while releasing start reservation; recovering"
            );
            poisoned.into_inner()
        }
    };
    if matches!(st.lifecycle, DictationLifecycle::Starting { .. })
        && claim.is_none_or(|claim| claim.is_current())
    {
        st.lifecycle = DictationLifecycle::Idle;
    }
}

fn spawn_stream_error_watcher(
    app: AppHandle,
    state: SharedState,
    stream_error: Arc<std::sync::atomic::AtomicBool>,
    active: Arc<std::sync::atomic::AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            if stream_error.load(Ordering::Acquire) {
                log::warn!(
                    "pipeline: input stream ended unexpectedly; stopping the active dictation"
                );
                crate::core::hotkey::set_handless_active(false);
                super::run_pipeline(app, state).await;
                break;
            }
            if !active.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });
}

pub fn spawn_level_emitter(
    app: AppHandle,
    level: Arc<std::sync::atomic::AtomicU32>,
    raw_level: Arc<std::sync::atomic::AtomicU32>,
    envelope: Arc<crate::media::audio::EnvelopeTap>,
    active: Arc<std::sync::atomic::AtomicBool>,
    speech_detected: Arc<std::sync::atomic::AtomicBool>,
    emit_globally: bool,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let emit_raw_level = |raw_level_val: f32| {
            if emit_globally {
                let _ = app.emit("audio-level-raw", raw_level_val);
            } else if let Some(pill) = app.get_webview_window("pill") {
                pill.emit("audio-level-raw", raw_level_val).ok();
            }
        };

        let emit_speech_detected = || {
            if emit_globally {
                let _ = app.emit("pill-speech-detected", ());
            } else if let Some(pill) = app.get_webview_window("pill") {
                pill.emit("pill-speech-detected", ()).ok();
            }
        };

        let mut speech_emitted = false;
        let mut emit_level = |level_val: f32, raw_level_val: f32| {
            if !speech_emitted && speech_detected.load(Ordering::Acquire) {
                emit_speech_detected();
                speech_emitted = true;
            }
            if emit_globally {
                let _ = app.emit("audio-level", level_val);
            } else if let Some(pill) = app.get_webview_window("pill") {
                pill.emit("audio-level", level_val).ok();
            }
            emit_raw_level(raw_level_val);
        };

        let emit_envelope = |batch: &[f32]| {
            if batch.is_empty() {
                return;
            }
            #[cfg(target_os = "android")]
            crate::android::bridge::note_audio_envelope(batch);
            if let Some(pill) = app.get_webview_window("pill") {
                pill.emit("audio-envelope", batch).ok();
            }
        };

        let mut envelope_batch = Vec::with_capacity(8);
        loop {
            if !active.load(Ordering::Relaxed) {
                break;
            }
            let level_val = f32::from_bits(level.load(Ordering::Relaxed));
            let raw_level_val = f32::from_bits(raw_level.load(Ordering::Relaxed));
            crate::android::bridge::note_audio_level(level_val);
            emit_level(level_val, raw_level_val);
            envelope.drain_into(&mut envelope_batch);
            emit_envelope(&envelope_batch);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        emit_envelope(&envelope.drain());
        crate::android::bridge::note_audio_level(0.0);
        emit_level(0.0, 0.0);
    });
}
pub(crate) fn start_stop_sounds_enabled(app: &AppHandle) -> bool {
    store::settings_snapshot(app)
        .map(|s| {
            let config = store::load_audio_config(&s);
            crate::media::sound::set_volume(config.sound_effects_volume);
            config.sound_effects_volume > 0.0
        })
        .unwrap_or(true)
}
