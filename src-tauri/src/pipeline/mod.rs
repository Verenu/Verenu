use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, Manager};

use crate::api::{auto_learn, cleanup, prompts, transcription, ProviderId};
use crate::core::{browser_probe, injection, window_context};
use crate::data::{db, dictionary, snippets, store};
use crate::media::audio;
use crate::system::apps::AppMapping;
use crate::system::diagnostics::{self, OperationOutcome, SpanFinish, TraceHandle};
use crate::system::number_parser;
use crate::system::text::is_number_word_token;
use crate::DbHandle;
use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, Utc};

mod cache;
mod chains;
mod clipboard_phrase;
pub(crate) mod failover;
mod finalize;
#[cfg(any(test, debug_assertions))]
mod fixture;
mod gates;
#[cfg(not(target_os = "android"))]
mod pill;
#[cfg(target_os = "android")]
mod pill {
    use tauri::{AppHandle, Emitter};

    // Android renders the pill in Kotlin. Keep the shared pipeline calls
    // intact while mirroring the state through the authenticated bridge.
    pub(crate) fn queue_pill_context(_context: &str) {}

    pub(crate) fn show_pill(_app: &AppHandle, _state: &str) {}

    pub(crate) fn update_pill_state(_app: &AppHandle, _state: &str) {}

    pub(crate) fn hide_pill(_app: &AppHandle) {}

    pub(crate) fn show_copied_pill(_app: &AppHandle, _msg: &str) {}

    pub(crate) fn emit_pill_stage(_app: &AppHandle, stage: &str) {
        crate::android::bridge::note_pill_stage(stage);
    }

    pub(crate) fn emit_pill_context(_app: &AppHandle, _context: &str) {}

    pub(crate) async fn show_error_pill(app: &AppHandle, msg: &str) {
        crate::android::bridge::note_bridge_error(msg);
        app.emit("verenu:error", msg).ok();
    }

    pub(crate) fn show_clipboard_warning_pill(_app: &AppHandle, _msg: &str) {}

    pub(crate) fn reject_with_pill(app: &AppHandle, msg: &str) {
        crate::android::bridge::note_bridge_error(msg);
        app.emit("verenu:error", msg).ok();
    }

    pub(crate) fn show_cancelled_pill(_app: &AppHandle) {}

    pub(crate) fn show_interrupted_pill(_app: &AppHandle) {}

    pub(crate) fn show_paste_failed_pill(_app: &AppHandle) {}
}
mod pill_animation;
mod pill_position;
mod session;
mod stages_cleanup;
mod stages_style;
mod stages_transcription;
mod state;
use cache::*;
use chains::*;
use finalize::{finalize_pipeline_completion, PipelineCompletionContext};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub use fixture::{
    run_pipeline_fixture, PipelineTestDictionaryEntry, PipelineTestRequest, PipelineTestResult,
    PipelineTestSnippet,
};
pub(crate) use gates::diagnostic_recording_gate_rms;
use gates::{
    effective_recording_rms, has_spoken_content, is_transcription_hallucination,
    normalize_transcription_math_artifacts, preview_text, recording_gate_rms,
    recording_gate_rms_for_sensitivity, silence_floor_gate_rms_for_sensitivity,
    strip_hallucinated_suffix, strip_trailing_hallucination, MIN_RECORDING_MS, MIN_RECORDING_RMS,
};
pub(crate) use pill::{
    emit_pill_context, emit_pill_stage, hide_pill, show_clipboard_warning_pill, show_copied_pill,
    show_pill, update_pill_state,
};
use pill::{
    reject_with_pill, show_cancelled_pill, show_error_pill, show_interrupted_pill,
    show_paste_failed_pill,
};
pub(crate) use pill_position::{
    apply_pill_placement, placement_for_current_monitor, PillPlacement,
};
pub(crate) use session::*;
use stages_cleanup::*;
use stages_style::*;
use stages_transcription::*;
pub use state::*;

/// Bounded, process-local sensitivity levels used after a rejected capture.
/// Each fresh attempt can become more permissive without changing the user's
/// saved microphone gain or making the relaxed thresholds permanent.
pub(crate) const MAX_ADAPTIVE_SENSITIVITY: u8 = 4;
pub(crate) const ADAPTIVE_SENSITIVITY_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(3);

pub(crate) fn adaptive_sensitivity_scale(level: u8) -> f32 {
    match level.min(MAX_ADAPTIVE_SENSITIVITY) {
        0 => 1.0,
        1 => 0.82,
        2 => 0.67,
        3 => 0.54,
        _ => 0.43,
    }
}

#[derive(Clone, Debug)]
pub struct CapturedAudio {
    wav_cache: Arc<Mutex<Option<bytes::Bytes>>>,
    pub samples_16k: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub duration_ms: u64,
}

impl CapturedAudio {
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32, duration_ms: u64) -> Self {
        Self {
            wav_cache: Arc::new(Mutex::new(None)),
            samples_16k: Arc::new(samples),
            sample_rate,
            duration_ms,
        }
    }

    pub fn wav_bytes(&self) -> anyhow::Result<bytes::Bytes> {
        let mut cache = self
            .wav_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("audio WAV cache lock poisoned"))?;
        if let Some(wav) = cache.as_ref() {
            return Ok(wav.clone());
        }
        let wav = audio::encode_wav(&self.samples_16k, self.sample_rate, 1)?;
        let wav = bytes::Bytes::from(wav);
        *cache = Some(wav.clone());
        Ok(wav)
    }

    pub fn wav_len(&self) -> usize {
        self.wav_cache
            .lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(bytes::Bytes::len))
            .unwrap_or(0)
    }

    pub fn clear_wav_cache(&mut self) {
        if let Ok(mut cache) = self.wav_cache.lock() {
            *cache = None;
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TranscriptCandidate {
    pub text: String,
    pub provider: String,
    pub model: String,
}

// ---------- pipeline ----------

pub async fn transcribe_input_only(app: AppHandle, state: SharedState) -> anyhow::Result<String> {
    let Some((session, exclusive_mic_session_id)) = state::take_recording_plain(&state) else {
        anyhow::bail!("No active recording");
    };

    let _media_pause_guard = crate::system::media_control::DictationMediaPauseGuard::new();
    crate::media::sound::coordinated_unmute();
    show_pill(&app, "processing");

    let settings_store = match store::settings_snapshot(&app) {
        Ok(s) => s,
        Err(e) => {
            // The session was removed from shared state above. Stop it before
            // returning, otherwise a settings read failure leaves the audio
            // thread and exclusive-mic reservation alive with no owner.
            let _ = stop_and_capture_audio(&app, session, exclusive_mic_session_id).await;
            hide_pill(&app);
            return Err(anyhow::anyhow!(e));
        }
    };
    let active_gain = store::load_audio_config(&settings_store).mic_gain;
    let sensitivity_level = state::current_sensitivity_level(&state);
    let min_rms = recording_gate_rms_for_sensitivity(active_gain, sensitivity_level);
    log::debug!(
        "pipeline: input gate active_gain={active_gain:.2} sensitivity_level={sensitivity_level} min_rms={min_rms:.6}"
    );

    let Some(stopped_capture) =
        stop_and_capture_audio(&app, session, exclusive_mic_session_id).await
    else {
        return Err(anyhow::anyhow!("Failed to stop recording"));
    };
    let StoppedCapture {
        audio: captured_audio,
        rms,
        raw_rms,
        stream_error,
        ..
    } = stopped_capture;
    if stream_error {
        show_error_pill(&app, AUDIO_STREAM_ERROR_MESSAGE).await;
        return Err(anyhow::anyhow!(AUDIO_STREAM_ERROR_MESSAGE));
    }
    let gate_rms = effective_recording_rms(rms, raw_rms, active_gain);
    if captured_audio.duration_ms < MIN_RECORDING_MS || gate_rms < min_rms {
        hide_pill(&app);
        if captured_audio.duration_ms < MIN_RECORDING_MS {
            anyhow::bail!("Recording too short");
        }
        anyhow::bail!("Audio too quiet - check your mic");
    }

    let cfg = store::load_pipeline_config(&settings_store);

    if let Err(message) = validate_transcription_chain(&cfg, None) {
        hide_pill(&app);
        anyhow::bail!(message);
    }

    emit_pill_stage(&app, "transcribing");
    let mut transcribed: Option<String> = None;
    let mut last_err: Option<anyhow::Error> = None;
    for (provider_id, model) in transcription_model_chain(&cfg) {
        let language = cfg.transcription_language.clone();
        let key = cfg.key_for(&provider_id).to_owned();
        match transcribe_any(
            &app,
            &captured_audio,
            &provider_id,
            if key.is_empty() {
                None
            } else {
                Some(key.as_str())
            },
            &language,
            &model,
            0,
        )
        .await
        {
            Ok(text) if !text.is_empty() => {
                transcribed = Some(text);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                let retryable = crate::api::is_retryable_provider_error(&e);
                log::warn!(
                    "pipeline: transcription provider failed gen=0 provider={} model={} retryable={} error={}",
                    provider_id,
                    model,
                    retryable,
                    trim_err(&e.to_string())
                );
                // A failure only tells us this candidate is unusable. It does
                // not say a configured fallback cannot work: a common setup
                // is a cloud primary with a downloaded local fallback, while
                // the cloud key is absent or stale. Match the main pipeline
                // and keep walking the configured chain regardless of whether
                // retrying this same provider would make sense.
                last_err = Some(e);
            }
        }
    }

    hide_pill(&app);

    match transcribed {
        Some(text) => {
            let normalized = normalize_transcription_math_artifacts(&text);
            let normalized = strip_trailing_hallucination(&strip_hallucinated_suffix(&normalized));
            let normalized = crate::system::text::collapse_degenerate_word_runs(&normalized);
            if !has_spoken_content(&normalized) || is_transcription_hallucination(&normalized) {
                state::note_sensitivity_rejection(&state);
                hide_pill(&app);
                anyhow::bail!("Recording was too quiet — nothing was transcribed");
            }
            state::note_sensitivity_success(&state);
            Ok(normalized)
        }
        None => Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("Transcription failed: no model in chain produced output")
        })),
    }
}

pub async fn run_pipeline(app: AppHandle, state: SharedState) {
    run_pipeline_with_delivery(app, state, false).await;
}

pub async fn run_pipeline_event_only(app: AppHandle, state: SharedState) {
    run_pipeline_with_delivery(app, state, true).await;
}

/// Waits for a cancellation signal without ever missing one that arrived
/// before this future started polling — `watch` retains its last value, so
/// checking `*rx.borrow()` first (not just relying on `changed()`) closes the
/// lost-wakeup gap a `Notify`-based signal would have had.
async fn wait_for_cancel(rx: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return; // sender dropped
        }
    }
}

/// Concatenates a previous (interrupted) dictation's audio onto a freshly
/// captured one — both are already resampled to the fixed 16kHz mono target,
/// so the sample buffers join directly. RMS is recomputed from the merged
/// samples rather than reusing either fragment's own value, since the two
/// aren't otherwise combinable.
fn merge_prepend_audio(
    prev: CapturedAudio,
    next: CapturedAudio,
    active_gain: f32,
) -> anyhow::Result<(CapturedAudio, f32, f32)> {
    let prev_duration_ms = prev.duration_ms;
    let mut samples =
        Arc::try_unwrap(prev.samples_16k).unwrap_or_else(|samples| (*samples).clone());
    if samples.len().saturating_add(next.samples_16k.len()) > audio::MAX_RECORDING_SAMPLES {
        anyhow::bail!("combined recording exceeds the duration cap");
    }
    samples.extend_from_slice(&next.samples_16k);
    let merged_rms = audio::rms_f32(&samples);
    let duration_ms = prev_duration_ms + next.duration_ms;
    let merged = CapturedAudio::from_samples(samples, 16_000, duration_ms);
    let merged_raw_rms = if active_gain > 0.0 {
        merged_rms / active_gain
    } else {
        merged_rms
    };
    Ok((merged, merged_rms, merged_raw_rms))
}

async fn run_pipeline_with_delivery(app: AppHandle, state: SharedState, event_only: bool) {
    let started_at = std::time::Instant::now();
    let Some((
        session,
        target,
        exclusive_mic_session_id,
        generation,
        prepend_audio,
        resolved_context_identity,
    )) = state::take_recording_for_stopping(&state)
    else {
        log::debug!("pipeline: no session - recording never started or was already consumed");
        return;
    };
    let trace = diagnostics::start_trace("dictation", None);
    let mut trace_guard = PipelineDiagnosticsGuard::new(trace);
    let _media_pause_guard = crate::system::media_control::DictationMediaPauseGuard::new();

    // Read once, synchronously, as close to the hotkey-release moment as
    // possible — the rest of the pipeline is async and the user may keep
    // typing (toggling Caps Lock) while it runs.
    let caps_lock_on = crate::core::hotkey::caps_lock_is_on();

    // Resolve app metadata from the target that will actually receive text
    // into (captured at record-start), not the live foreground at release — the
    // two can diverge (handsfree, focus shifts) and that divergence let one
    // app's mapping style leak into another app. Issue #144. Falls back to the
    // live foreground only when the captured target id is unavailable (null/0).
    let process_name = window_context::get_process_name_for_hwnd(target.id)
        .or_else(window_context::get_active_process_name)
        .unwrap_or_else(|| "unknown".into())
        .to_lowercase();
    let db_handle = app.state::<DbHandle>().inner().clone();
    // Only probe the address bar when the foreground app is actually a
    // browser — the UIA tree walk is comparatively costly and meaningless
    // for any other window. This metadata is only used for the cleanup prompt;
    // the Context identity itself was captured before recording began.
    let browser_domain = if window_context::is_browser_exe(&process_name) {
        browser_probe::read_browser_domain_for_window(target.id)
    } else {
        None
    };
    // Rehydrate only the settings needed by the cleanup/profile stage. A
    // deleted Context must not cause a second foreground resolution or leak
    // the dictation into whichever Context now matches the active window.
    let resolved_context = db::query_context(&db_handle, resolved_context_identity.id).ok();
    let context_id = resolved_context_identity.id;
    log::info!("pipeline: start gen={generation} target_id={}", target.id);

    // Mark the session inactive before unmuting or waiting on stop() so the
    // delayed mute helper cannot wake up and re-mute the system mid-shutdown.
    session.active.store(false, Ordering::Relaxed);
    crate::media::sound::cancel_pending_start();
    crate::media::sound::coordinated_unmute();
    show_pill(&app, "processing");
    // Label the pill "Transcribing…" from the moment processing starts, not
    // once the transcription call actually begins. Audio encoding and the
    // local Silero VAD gate both run first and both scale with recording
    // length — the VAD gate alone reprocesses the *entire* buffer frame by
    // frame, so on a multi-minute hands-free dictation this stretch can run
    // several seconds. Until this moved, the pill had no stage mounted for
    // that whole span and simply went blank, then jumped straight to
    // "Transcribing…" once the real network call started.
    emit_pill_stage(&app, "transcribing");

    // Keep the quiet-audio gate permissive at high gain. Whisper recordings can
    // still have low post-denoise RMS, even after amplification.
    let audio_cfg = match store::settings_snapshot(&app) {
        Ok(s) => store::load_audio_config(&s),
        Err(e) => {
            log::warn!("pipeline: failed to load audio config, using defaults: {e}");
            store::AudioConfig::default()
        }
    };
    let active_gain = audio_cfg.mic_gain;
    let sensitivity_level = state::current_sensitivity_level(&state);
    let min_rms = recording_gate_rms_for_sensitivity(active_gain, sensitivity_level);
    log::debug!(
        "pipeline: audio gate active_gain={active_gain:.2} sensitivity_level={sensitivity_level} min_rms={min_rms:.6}"
    );

    let stage_audio = std::time::Instant::now();
    let audio_span = diagnostics::start_span(
        trace_guard.trace_id(),
        "audio_preparation",
        Some("capture"),
        None,
    );
    // Capture first, gate second: a resumed/prepended recording needs to be
    // merged with the previous session's audio before the quality gate runs,
    // so a short-but-valid continuation isn't rejected on its own merits.
    let Some(stopped_capture) =
        stop_and_capture_audio(&app, session, exclusive_mic_session_id).await
    else {
        // Stop/capture failures retain any durable prefix for crash recovery;
        // the duration-limit branch handles its deliberate cleanup itself.
        state::leave_stopping_if_owned(&state, generation);
        return;
    };
    let StoppedCapture {
        audio: mut captured_audio,
        mut rms,
        mut raw_rms,
        stream_error,
        ..
    } = stopped_capture;
    if stream_error {
        failover::abandon_live();
        show_error_pill(&app, AUDIO_STREAM_ERROR_MESSAGE).await;
        state::leave_stopping_if_owned(&state, generation);
        return;
    }
    if let Some(prev) = prepend_audio {
        let (merged, merged_rms, merged_raw_rms) =
            match merge_prepend_audio(prev, captured_audio, active_gain) {
                Ok(merged) => merged,
                Err(err) => {
                    log::error!("pipeline: {err}");
                    reject_with_pill(&app, "Failed to prepare recording audio");
                    failover::abandon_live();
                    state::leave_stopping_if_owned(&state, generation);
                    return;
                }
            };
        captured_audio = merged;
        rms = merged_rms;
        raw_rms = merged_raw_rms;
    }
    // Only reject here on duration or on RMS so low it's obviously digital
    // silence/a dead mic — cheap enough to check before paying for an API
    // call. The real "is there speech" judgment happens in the local VAD gate
    // below, before any transcription request — this early gate deliberately
    // stays permissive so quiet/distant speech gets a fair chance at VAD
    // instead of being rejected on RMS alone.
    let silence_floor = silence_floor_gate_rms_for_sensitivity(active_gain, sensitivity_level);
    if !validate_captured_audio(
        &app,
        &captured_audio,
        rms,
        raw_rms,
        silence_floor,
        active_gain,
    ) {
        state::note_sensitivity_rejection(&state);
        failover::abandon_live();
        state::leave_stopping_if_owned(&state, generation);
        return;
    }
    if audio_cfg.sound_effects_volume > 0.0 {
        crate::media::sound::set_volume(audio_cfg.sound_effects_volume);
        crate::media::sound::play(crate::media::sound::SoundCue::Stop);
    }
    log::debug!(
        "pipeline: audio accepted duration_ms={} wav_bytes={} stage_ms={}",
        captured_audio.duration_ms,
        44 + captured_audio.samples_16k.len() * 2,
        stage_audio.elapsed().as_millis()
    );
    if let Some(span) = audio_span {
        let _ = diagnostics::finish_span(&span, OperationOutcome::Success);
    }

    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    let active = ActivePipeline {
        generation,
        cancel_tx,
        captured_audio: captured_audio.clone(),
        context: resolved_context_identity.clone(),
        target,
    };
    {
        let (id, started) = lock_state(&state)
            .ok()
            .map(|st| (st.failover_session_id.clone(), st.failover_started_at_unix))
            .unwrap_or((None, 0));
        if let Some(id) = id {
            let started = if started != 0 {
                started
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            };
            failover::commit_capture_with_context(
                &captured_audio,
                &id,
                failover::FailoverKind::Processing,
                started,
                Some(resolved_context_identity.id),
            );
        }
    }
    if !state::install_processing(&state, generation, active) {
        // Superseded between Stopping and here (should not happen in
        // practice — nothing else can transition out of Stopping — but an
        // interrupt/Escape branch, if it somehow raced this, already owns
        // whatever cleanup is needed).
        return;
    }

    let stage_config = std::time::Instant::now();
    let config_span = diagnostics::start_span(
        trace_guard.trace_id(),
        "context_resolution",
        Some("context"),
        None,
    );
    let Some((cfg, profile, app_context)) = open_config_and_context(
        &app,
        &process_name,
        target.id,
        browser_domain.as_deref(),
        resolved_context.as_ref(),
    )
    .await
    else {
        // open_config_and_context already shows its own error/pill on
        // failure — no separate emit_pipeline_failed here (matches its
        // pre-existing contract).
        state::leave_processing_if_owned(&state, generation);
        return;
    };
    // Surface the resolved context so the pill can show where this dictation
    // is going (same value emitted at record start, now domain-refined).
    emit_pill_context(&app, &resolved_context_identity.label);
    log::debug!(
        "pipeline: config t_provider={} c_provider={} t_model={} c_model={} cleanup_enabled={} intensity={} app_context_hint={} profile={}",
        cfg.transcription_provider,
        cfg.cleanup_provider,
        cfg.transcription_default_model,
        cfg.cleanup_default_model,
        cfg.cleanup_enabled,
        cfg.cleanup_intensity,
        cfg.app_context_hint,
        profile
    );
    if let Some(span) = config_span {
        let _ = diagnostics::finish_span(&span, OperationOutcome::Success);
    }
    log::debug!(
        "pipeline: context resolved app_context_present={} stage_ms={}",
        app_context.is_some(),
        stage_config.elapsed().as_millis()
    );

    if *cancel_rx.borrow() {
        state::leave_processing_if_owned(&state, generation);
        return;
    }

    let retry_captured_at = std::time::Instant::now();
    if let Ok(mut st) = lock_state(&state) {
        st.retry_capture = Some(RetryCapture {
            audio: captured_audio.clone(),
            captured_at: retry_captured_at,
            target,
            process_name: process_name.clone(),
            context: resolved_context_identity.clone(),
            profile: profile.clone(),
            app_context: app_context.clone(),
            caps_lock_on,
        });
    }

    // Run local VAD before touching the transcription API. This is deliberate:
    // a post-transcription gate still pays for the API and gives Whisper a
    // chance to hallucinate text from noise. A VAD failure (model missing or
    // inference error) never blocks a dictation; the speech gate falls back to
    // the RMS threshold only in that case.
    let vad_span = diagnostics::start_span(trace_guard.trace_id(), "vad_gate", Some("vad"), None);
    let vad_samples = captured_audio.samples_16k.clone();
    let vad_handle = tokio::task::spawn_blocking(move || {
        crate::media::vad::analyze_speech_with_sensitivity(
            &vad_samples,
            active_gain,
            sensitivity_level,
        )
    });

    let vad_result = tokio::select! {
        result = vad_handle => match result {
            Ok(Ok(result)) => Some(result),
            Ok(Err(e)) => {
                log::warn!("pipeline: VAD analysis failed, falling back to RMS-only gate: {e}");
                None
            }
            Err(e) => {
                log::warn!("pipeline: VAD task panicked, falling back to RMS-only gate: {e}");
                None
            }
        },
        _ = wait_for_cancel(&mut cancel_rx) => {
            log::info!("pipeline: cancelled gen={generation} (awaiting VAD gate)");
            state::leave_processing_if_owned(&state, generation);
            return;
        }
    };
    if !passes_speech_gate(
        &app,
        rms,
        raw_rms,
        min_rms,
        active_gain,
        vad_result.as_ref(),
    ) {
        state::note_sensitivity_rejection(&state);
        state::leave_processing_if_owned(&state, generation);
        return;
    }
    if let Some(span) = vad_span {
        let _ = diagnostics::finish_span(&span, OperationOutcome::Success);
    }

    // Stage already emitted right after entering "processing" above (see
    // comment there) — this Instant is purely for the timing log below.
    let stage_transcribe = std::time::Instant::now();
    let transcription_span =
        if cfg.dual_transcription_enabled && transcription_model_chain(&cfg).len() > 1 {
            diagnostics::start_parallel_span(
                trace_guard.trace_id(),
                "transcription",
                Some("transcription"),
                None,
                Some("dual-transcription"),
            )
        } else {
            diagnostics::start_span(
                trace_guard.trace_id(),
                "transcription",
                Some("transcription"),
                None,
            )
        };
    let transcribe_race = tokio::select! {
        r = run_transcription(&app, &captured_audio, &cfg, generation) => Some(r),
        _ = wait_for_cancel(&mut cancel_rx) => {
            log::info!("pipeline: cancelled gen={generation} (during transcription)");
            None
        }
    };
    let Some(transcribe_outcome) = transcribe_race else {
        state::leave_processing_if_owned(&state, generation);
        return;
    };
    let Some((raw_unorm, api_used, alternate)) = transcribe_outcome else {
        if state::leave_processing_if_owned(&state, generation) {
            emit_pipeline_failed(&app);
        }
        return;
    };

    let raw = normalize_transcription_math_artifacts(&raw_unorm);
    let raw_chars_before_strip = raw.chars().count();
    let raw = if alternate
        .as_ref()
        .is_some_and(|candidate| candidate.text.trim().eq_ignore_ascii_case(raw.trim()))
    {
        raw
    } else {
        strip_trailing_hallucination(&strip_hallucinated_suffix(&raw))
    };
    if raw.chars().count() != raw_chars_before_strip {
        log::warn!(
            "pipeline: trimmed trailing hallucination provider={} chars_before={} chars_after={}",
            api_used,
            raw_chars_before_strip,
            raw.chars().count()
        );
    }
    let raw_chars_before_collapse = raw.chars().count();
    let raw = crate::system::text::collapse_degenerate_word_runs(&raw);
    if raw.chars().count() != raw_chars_before_collapse {
        log::warn!(
            "pipeline: collapsed degenerate word run provider={} chars_before={} chars_after={}",
            api_used,
            raw_chars_before_collapse,
            raw.chars().count()
        );
    }
    log::debug!(
        "pipeline: transcription ok provider={} raw_chars={} raw_preview=\"{}\"",
        api_used,
        raw.chars().count(),
        preview_text(&raw, 140)
    );
    let raw_words = raw.split_whitespace().count();
    if let Some(span) = transcription_span {
        let _ = diagnostics::finish_span_with(
            &span,
            SpanFinish {
                outcome: OperationOutcome::Success,
                provider: Some(api_used.clone()),
                measurements: [("raw_words".to_owned(), raw_words as f64)]
                    .into_iter()
                    .collect(),
                ..SpanFinish::default()
            },
        );
    }
    // Diagnostic only, no behavioral effect — counts and a ratio, never the
    // text. Average conversational speech runs ~2-3 words/sec; a ratio well
    // under that on a recording long enough to judge reliably (rules out
    // pauses/silence at the start dominating a short clip) is a signal worth
    // having on hand if a future report of "words missing" turns out to be
    // the transcription itself dropping content rather than cleanup, which
    // today has no equivalent completeness check of its own.
    if captured_audio.duration_ms >= 3000 {
        let words_per_sec = raw_words as f64 / (captured_audio.duration_ms as f64 / 1000.0);
        log::debug!(
            "pipeline: transcription completeness words={} duration_ms={} words_per_sec={:.2}",
            raw_words,
            captured_audio.duration_ms,
            words_per_sec
        );
    }
    log::debug!(
        "pipeline: transcription stage_ms={}",
        stage_transcribe.elapsed().as_millis()
    );

    // Post-transcription hallucination gate — silently drop prompt-echoes and
    // known silent-audio artifacts before they reach cleanup or the cache.
    // (A trailing hallucinated sentence has already been trimmed above; this
    // catches the case where the whole transcription is still one.)
    if !has_spoken_content(&raw) || is_transcription_hallucination(&raw) {
        log::warn!(
            "pipeline: transcription had no spoken content or matched a hallucination pattern, dropping silently raw=\"{}\"",
            preview_text(&raw, 60)
        );
        if state::leave_processing_if_owned(&state, generation) {
            hide_pill(&app);
        }
        return;
    }

    if *cancel_rx.borrow() {
        state::leave_processing_if_owned(&state, generation);
        return;
    }

    let (raw_for_cleanup, clipboard_plan, clipboard_warning) =
        prepare_clipboard_phrase(&cfg, &raw).await;
    let clipboard_instruction = clipboard_plan
        .as_ref()
        .map(clipboard_phrase::cleanup_instruction);

    let stage_cleanup = std::time::Instant::now();
    let cleanup_span = diagnostics::start_span(
        trace_guard.trace_id(),
        "cleanup_and_snippets",
        Some("cleanup"),
        None,
    );
    // Only advertise the cleaning stage when the cleanup LLM will actually run
    // (cleanup enabled + intensity + a key in the chain). When cleanup is off
    // the cleanup call resolves to local snippet/dictionary work that is
    // effectively instant, so advertising it would just flash the label.
    let cleanup_will_run_llm = should_run_cleanup_llm(
        cfg.cleanup_enabled,
        has_cleanup_key_in_chain(&cfg),
        true,
        &cfg.cleanup_intensity,
        &profile,
        cfg.cleanup_intensity == "none" && alternate.is_some(),
    );
    if cleanup_will_run_llm {
        emit_pill_stage(&app, "cleaning");
    }
    let cleanup_race = tokio::select! {
        r = run_cleanup_and_snippets(&app, &raw_for_cleanup, alternate.as_ref(), &cfg, &profile, app_context.as_deref(), context_id, clipboard_instruction.as_deref(), generation) => Some(r),
        _ = wait_for_cancel(&mut cancel_rx) => {
            log::info!("pipeline: cancelled gen={generation} (during cleanup)");
            None
        }
    };
    let Some(cleanup_outcome) = cleanup_race else {
        state::leave_processing_if_owned(&state, generation);
        return;
    };
    let Some((final_text, dict_entries, cleanup_cache_key, cleanup_api_used)) = cleanup_outcome
    else {
        if state::leave_processing_if_owned(&state, generation) {
            emit_pipeline_failed(&app);
        }
        return;
    };
    let api_used = append_cleanup_api_used(api_used, &cleanup_api_used);
    log::debug!(
        "pipeline: cleanup/snippets ok final_chars={} final_preview=\"{}\" dict_entries={}",
        final_text.chars().count(),
        preview_text(&final_text, 140),
        dict_entries.len()
    );
    if let Some(span) = cleanup_span {
        let _ = diagnostics::finish_span_with(
            &span,
            SpanFinish {
                outcome: OperationOutcome::Success,
                provider: (!cleanup_api_used.is_empty()).then_some(cleanup_api_used.clone()),
                measurements: [("dictionary_entries".to_owned(), dict_entries.len() as f64)]
                    .into_iter()
                    .collect(),
                ..SpanFinish::default()
            },
        );
    }
    log::debug!(
        "pipeline: cleanup stage_ms={}",
        stage_cleanup.elapsed().as_millis()
    );

    // Point of no return: once this succeeds, finalize runs unconditionally,
    // never raced against cancellation — finalize touches the clipboard, and
    // a preempted clipboard operation is worse than an un-cancellable late
    // finalize (see core/injection's serialized critical section). Failure
    // here means an interrupt/Escape branch already took ownership of this
    // generation; abandon without inserting anything.
    emit_pill_stage(&app, "pasting");
    if !state::enter_finalizing(&state, generation) {
        return;
    }

    let words = raw.split_whitespace().count() as i64;
    let finalize_span = diagnostics::start_span(
        trace_guard.trace_id(),
        "finalize_and_inject",
        Some("injection"),
        None,
    );
    if let Err(e) = finalize_pipeline_completion(
        &app,
        &state,
        PipelineCompletionContext {
            raw: &raw,
            final_text_before_dict: &final_text,
            clipboard_plan: clipboard_plan.as_ref(),
            clipboard_warning,
            dict_entries: &dict_entries,
            duration_ms: captured_audio.duration_ms,
            api_used: &api_used,
            target_hwnd: target.id,
            cfg: &cfg,
            profile: &profile,
            process_name: process_name.clone(),
            cleanup_cache_key,
            captured_at: retry_captured_at,
            event_only,
            caps_lock_on,
            context: resolved_context_identity,
        },
    )
    .await
    {
        if let Some(ref span) = finalize_span {
            let _ = diagnostics::finish_span(span, OperationOutcome::Failure);
        }
        log::error!("pipeline finalize failed: {e}");
        state::leave_finalizing(&state, generation);
        return;
    }
    if let Some(ref span) = finalize_span {
        let _ = diagnostics::finish_span(span, OperationOutcome::Success);
    }
    state::leave_finalizing(&state, generation);
    state::note_sensitivity_success(&state);

    log::info!(
        "pipeline: completed gen={generation} words={} duration_ms={} elapsed_ms={}",
        words,
        captured_audio.duration_ms,
        started_at.elapsed().as_millis()
    );
    trace_guard.success();
}

/// Finishes a trace on every early-return path. An incomplete stage is marked
/// cancelled by the diagnostics store, preventing active-trace leaks when a
/// user cancels during VAD, transcription, or cleanup.
struct PipelineDiagnosticsGuard {
    trace: TraceHandle,
    outcome: OperationOutcome,
}

impl PipelineDiagnosticsGuard {
    fn new(trace: TraceHandle) -> Self {
        Self {
            trace,
            outcome: OperationOutcome::Failure,
        }
    }
    fn trace_id(&self) -> &str {
        &self.trace.trace_id
    }
    fn success(&mut self) {
        self.outcome = OperationOutcome::Success;
    }
}

impl Drop for PipelineDiagnosticsGuard {
    fn drop(&mut self) {
        let _ = diagnostics::finish_trace(&self.trace.trace_id, self.outcome);
    }
}

async fn prepare_clipboard_phrase(
    cfg: &store::PipelineConfig,
    raw: &str,
) -> (
    String,
    Option<clipboard_phrase::ClipboardPhrasePlan>,
    Option<&'static str>,
) {
    if !cfg.clipboard_phrase_enabled
        || clipboard_phrase::replace_phrase_with_marker(raw, &cfg.clipboard_phrase, String::new())
            .is_none()
    {
        return (raw.to_string(), None, None);
    }

    match injection::read_current_clipboard_text()
        .await
        .filter(|text| !text.trim().is_empty())
    {
        Some(text) => {
            match clipboard_phrase::replace_phrase_with_marker(raw, &cfg.clipboard_phrase, text) {
                Some(plan) => (plan.pre_cleanup.clone(), Some(plan), None),
                None => {
                    log::debug!("pipeline: clipboard phrase match disappeared before planning");
                    (raw.to_string(), None, None)
                }
            }
        }
        None => (
            clipboard_phrase::remove_phrase(raw, &cfg.clipboard_phrase),
            None,
            Some("No text in clipboard"),
        ),
    }
}

#[cfg(test)]
mod tests;

fn append_cleanup_api_used(api_used: String, cleanup_api_used: &str) -> String {
    if cleanup_api_used.is_empty() {
        api_used
    } else {
        format!("{api_used};cleanup={cleanup_api_used}")
    }
}

pub async fn retry_transcription_impl(
    app: &AppHandle,
    state: &SharedState,
) -> anyhow::Result<db::RecentEntry> {
    state::reserve_starting(state).map_err(anyhow::Error::msg)?;
    let _retry_reservation = RetryReservation { state };
    let mut retry_expired = false;
    let capture = {
        let mut st = lock_state(state)?;
        match &st.retry_capture {
            Some(retry) => {
                if retry.captured_at.elapsed() > RETRY_WINDOW {
                    st.retry_capture = None;
                    retry_expired = true;
                    None
                } else {
                    Some(retry.clone())
                }
            }
            None => None,
        }
    };
    if retry_expired {
        show_error_pill(app, "Retry window expired").await;
        anyhow::bail!("Retry window expired");
    }
    let Some(mut capture) = capture else {
        hide_pill(app);
        anyhow::bail!("No retry available");
    };
    // Count only a real Retry action with a still-available capture. The boost
    // applies to the next fresh take; a missing or expired retry must not
    // change microphone sensitivity.
    state::note_sensitivity_retry(state);
    capture.target = capture.target.refreshed();
    if let Ok(mut st) = lock_state(state) {
        st.target = capture.target;
        st.pill_placement_stale = true;
    }
    show_pill(app, "processing");

    let settings_store = match store::settings_snapshot(app) {
        Ok(settings) => settings,
        Err(error) => {
            hide_pill(app);
            return Err(anyhow::Error::msg(error));
        }
    };
    let mut cfg = store::load_pipeline_config(&settings_store);

    if let Err(message) = validate_transcription_chain(&cfg, None) {
        show_error_pill(app, &message).await;
        anyhow::bail!(message);
    }

    let mapping = resolve_app_mapping(Some(&settings_store), &capture.process_name);
    let db_handle = app.state::<DbHandle>().inner().clone();
    let context = db::query_context(&db_handle, capture.context.id).ok();
    capture.profile = apply_app_style_overrides(&mut cfg, mapping.as_ref(), context.as_ref());
    emit_pill_context(app, &capture.context.label);

    emit_pill_stage(app, "transcribing");
    let Some((raw_unorm, api_used, alternate)) =
        run_transcription(app, &capture.audio, &cfg, 0).await
    else {
        hide_pill(app);
        anyhow::bail!("Retry transcription failed");
    };
    let raw = normalize_transcription_math_artifacts(&raw_unorm);
    let raw = if alternate
        .as_ref()
        .is_some_and(|candidate| candidate.text.trim().eq_ignore_ascii_case(raw.trim()))
    {
        raw
    } else {
        strip_trailing_hallucination(&strip_hallucinated_suffix(&raw))
    };
    let raw = crate::system::text::collapse_degenerate_word_runs(&raw);
    if !has_spoken_content(&raw) || is_transcription_hallucination(&raw) {
        log::warn!(
            "pipeline: retry transcription had no spoken content or matched a hallucination pattern, dropping raw=\"{}\"",
            preview_text(&raw, 60)
        );
        hide_pill(app);
        anyhow::bail!("Recording was too quiet — nothing was transcribed");
    }
    if should_run_cleanup_llm(
        cfg.cleanup_enabled,
        has_cleanup_key_in_chain(&cfg),
        true,
        &cfg.cleanup_intensity,
        &capture.profile,
        cfg.cleanup_intensity == "none" && alternate.is_some(),
    ) {
        emit_pill_stage(app, "cleaning");
    }
    let (raw_for_cleanup, clipboard_plan, clipboard_warning) =
        prepare_clipboard_phrase(&cfg, &raw).await;
    let clipboard_instruction = clipboard_plan
        .as_ref()
        .map(clipboard_phrase::cleanup_instruction);
    let Some((final_text, dict_entries, cleanup_cache_key, cleanup_api_used)) =
        run_cleanup_and_snippets(
            app,
            &raw_for_cleanup,
            alternate.as_ref(),
            &cfg,
            &capture.profile,
            capture.app_context.as_deref(),
            capture.context.id,
            clipboard_instruction.as_deref(),
            0,
        )
        .await
    else {
        hide_pill(app);
        anyhow::bail!("Retry cleanup failed");
    };
    let api_used = append_cleanup_api_used(api_used, &cleanup_api_used);

    emit_pill_stage(app, "pasting");
    let result = finalize_pipeline_completion(
        app,
        state,
        PipelineCompletionContext {
            raw: &raw,
            final_text_before_dict: &final_text,
            clipboard_plan: clipboard_plan.as_ref(),
            clipboard_warning,
            dict_entries: &dict_entries,
            duration_ms: capture.audio.duration_ms,
            api_used: &api_used,
            target_hwnd: capture.target.id,
            cfg: &cfg,
            profile: &capture.profile,
            process_name: capture.process_name,
            cleanup_cache_key,
            captured_at: capture.captured_at,
            event_only: false,
            caps_lock_on: capture.caps_lock_on,
            context: capture.context,
        },
    )
    .await?;
    Ok(result)
}

struct RetryReservation<'a> {
    state: &'a SharedState,
}

impl Drop for RetryReservation<'_> {
    fn drop(&mut self) {
        state::cancel_starting_reservation(self.state);
    }
}
