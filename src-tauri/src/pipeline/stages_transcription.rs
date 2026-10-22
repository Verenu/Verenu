use super::stages_style::{apply_app_style_overrides, resolve_app_mapping};
use super::*;
struct ExclusiveMicReleaseGuard(Option<u64>);

impl Drop for ExclusiveMicReleaseGuard {
    fn drop(&mut self) {
        if let Some(session_id) = self.0.take() {
            crate::system::volume::release_mic(session_id);
        }
    }
}

pub(crate) struct StoppedCapture {
    pub(crate) audio: CapturedAudio,
    pub(crate) rms: f32,
    pub(crate) raw_rms: f32,
    pub(crate) recovery_write_failed: bool,
    pub(crate) stream_error: bool,
}

pub(crate) const AUDIO_STREAM_ERROR_MESSAGE: &str =
    "Microphone input stopped unexpectedly. Check that your microphone is connected, then try again.";

fn disable_recovery_after_write_failure(app: &AppHandle) {
    super::failover::abandon_live();
    if let Some(state) = app.try_state::<SharedState>() {
        if let Ok(mut st) = lock_state(state.inner()) {
            st.failover_session_id = None;
            st.failover_reuse_id = false;
            st.failover_started_at_unix = 0;
        }
    }
}

pub(super) async fn stop_and_capture_audio(
    app: &AppHandle,
    session: audio::RecordingSession,
    exclusive_mic_session_id: Option<u64>,
) -> Option<StoppedCapture> {
    let stop_result = tokio::task::spawn_blocking(move || {
        let _mic_release = ExclusiveMicReleaseGuard(exclusive_mic_session_id);
        session.stop()
    })
    .await;
    if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() {
        manager.set_recording_active(false);
    }
    let audio::RecordingResult {
        samples_16k,
        sample_rate,
        duration_ms,
        rms,
        raw_rms,
        termination,
    } = match stop_result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            log::error!("audio stop: {e}");
            hide_pill(app);
            return None;
        }
        Err(e) => {
            log::error!("audio stop task panicked: {e}");
            hide_pill(app);
            return None;
        }
    };
    match termination {
        audio::RecordingTermination::Complete => {}
        audio::RecordingTermination::StreamError => {
            log::warn!("pipeline: microphone input stream stopped unexpectedly");
        }
        audio::RecordingTermination::DurationLimit => {
            log::warn!(
                "pipeline: rejected recording that exceeded max duration limit max_seconds={}",
                audio::MAX_RECORDING_SECONDS
            );
            show_error_pill(
                app,
                &format!(
                    "Recording exceeded the {} minute limit. Please split it into shorter dictations.",
                    audio::MAX_RECORDING_SECONDS / 60
                ),
            )
            .await;
            super::failover::abandon_live();
            return None;
        }
        audio::RecordingTermination::DroppedSamples => {
            log::warn!("pipeline: rejected recording because audio samples were dropped");
            show_error_pill(
                app,
                "Recording was interrupted because audio fell behind and samples were dropped. Please try again.",
            )
            .await;
            return None;
        }
        audio::RecordingTermination::RecoveryWriteFailed => {
            log::warn!("pipeline: recovery recording could not be kept durable");
            disable_recovery_after_write_failure(app);
        }
    }
    Some(StoppedCapture {
        audio: CapturedAudio::from_samples(samples_16k, sample_rate, duration_ms),
        rms,
        raw_rms,
        recovery_write_failed: termination == audio::RecordingTermination::RecoveryWriteFailed,
        stream_error: termination == audio::RecordingTermination::StreamError,
    })
}

pub(super) fn validate_captured_audio(
    app: &AppHandle,
    audio: &CapturedAudio,
    rms: f32,
    raw_rms: f32,
    min_rms: f32,
    active_gain: f32,
) -> bool {
    let gate_rms = effective_recording_rms(rms, raw_rms, active_gain);
    if audio.duration_ms < MIN_RECORDING_MS || gate_rms < min_rms {
        let msg = if audio.duration_ms < MIN_RECORDING_MS {
            "Recording too short"
        } else {
            "Audio too quiet — check your mic"
        };
        log::debug!(
            "pipeline: rejected — duration={}ms rms={rms:.4} gate_rms={gate_rms:.4} min_rms={min_rms:.4}",
            audio.duration_ms
        );
        reject_with_pill(app, msg);
        return false;
    }
    true
}

pub(super) fn passes_speech_gate(
    app: &AppHandle,
    rms: f32,
    raw_rms: f32,
    min_rms: f32,
    active_gain: f32,
    vad_result: Option<&crate::media::vad::SpeechDetectionResult>,
) -> bool {
    let gate_rms = effective_recording_rms(rms, raw_rms, active_gain);
    if speech_gate_accepts(vad_result, gate_rms, min_rms) {
        log::debug!(
            "pipeline: speech gate accepted gate_rms={gate_rms:.4} min_rms={min_rms:.4} vad={:?}",
            vad_result
        );
        return true;
    }
    log::debug!(
        "pipeline: speech gate rejected — no speech detected rms={rms:.4} min_rms={min_rms:.4} vad={:?}",
        vad_result
    );
    reject_with_pill(app, "No speech detected");
    false
}

pub(super) fn speech_gate_accepts(
    vad_result: Option<&crate::media::vad::SpeechDetectionResult>,
    gate_rms: f32,
    min_rms: f32,
) -> bool {
    vad_result.map_or(gate_rms >= min_rms, |result| result.contains_speech)
}

pub(super) async fn open_config_and_context(
    app: &AppHandle,
    process_name: &str,
    context: Option<&db::Context>,
) -> Option<(store::PipelineConfig, String)> {
    let settings_store = match store::settings_snapshot(app) {
        Ok(s) => s,
        Err(e) => {
            log::error!("store: {e}");
            hide_pill(app);
            return None;
        }
    };
    let mut cfg = store::load_pipeline_config(&settings_store);
    if let Err(message) = validate_transcription_chain(&cfg, None) {
        show_error_pill(app, &message).await;
        return None;
    }
    let mapping = resolve_app_mapping(Some(&settings_store), process_name);
    let profile = apply_app_style_overrides(&mut cfg, mapping.as_ref(), context);
    if let Some(context) = context {
        cfg.speech_vocabulary = crate::local_stt::vocabulary::Vocabulary::for_context(
            app.state::<DbHandle>().inner(), context.id,
        ).unwrap_or_default();
    }
    log::debug!(
        "pipeline: app mapping resolved process={process_name} matched={} profile={profile} cleanup_intensity={} default_tone={}",
        mapping.as_ref().map(|m| m.exe.as_str()).unwrap_or("none"),
        cfg.cleanup_intensity,
        cfg.default_tone,
    );
    Some((cfg, profile))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn transcribe_any(
    app: &AppHandle,
    audio: &CapturedAudio,
    provider_id: &str,
    customs: &[crate::api::custom::CustomProvider],
    api_key: Option<&str>,
    language: &str,
    model: &str,
    gen: u64,
    vocabulary: &crate::local_stt::vocabulary::Vocabulary,
) -> anyhow::Result<String> {
    let started = std::time::Instant::now();
    if provider_id == store::LOCAL {
        let manager = app
            .state::<crate::local_stt::LocalTranscriptionManager>()
            .inner()
            .clone();
        let state = manager.state();
        let already_warm = state.is_loaded && state.current_model_id.as_deref() == Some(model);
        if !already_warm {
            show_pill(app, "loading_local_model");
        }
        let result = crate::local_stt::transcribe::transcribe(
            manager,
            app.clone(),
            model.to_string(),
            Arc::clone(&audio.samples_16k),
            audio.sample_rate,
            language.to_string(),
            vocabulary.clone(),
        )
        .await;
        if already_warm {
            crate::model_performance::record(
                "transcription",
                provider_id,
                model,
                started.elapsed().as_secs_f64() * 1000.0 * 10_000.0
                    / audio.duration_ms.max(1) as f64,
                result.is_ok(),
            );
        }
        if !already_warm {
            show_pill(app, "processing");
        }
        return result;
    }

    let target = crate::api::Target::resolve(provider_id, customs)
        .ok_or_else(|| anyhow::anyhow!("The custom provider for this model was removed"))?;
    let key = match (&target, api_key) {
        (crate::api::Target::Custom(p), _) if !p.requires_key => api_key.unwrap_or_default(),
        (_, Some(key)) => key,
        _ => anyhow::bail!("No API key saved for {provider_id}"),
    };
    let timeout = std::time::Duration::from_secs((20 + audio.duration_ms / 500).min(120));
    let result = tokio::time::timeout(
        timeout,
        transcription::transcribe(audio.wav_bytes()?, target, key, language, model, gen),
    )
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("cloud transcription timeout")));
    if result.is_ok() && !crate::api::custom::is_custom_id(provider_id) {
        crate::system::connectivity::note_online();
    }
    crate::model_performance::record(
        "transcription",
        provider_id,
        model,
        started.elapsed().as_secs_f64() * 1000.0 * 10_000.0 / audio.duration_ms.max(1) as f64,
        result.is_ok(),
    );
    result
}

pub(super) async fn confirm_offline(app: &AppHandle) -> bool {
    let checks_enabled = store::settings_snapshot(app)
        .map(|s| {
            s.get(store::VERENU_SERVICE_CHECKS_ENABLED)
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        })
        .unwrap_or(true);
    crate::system::connectivity::confirm_offline(checks_enabled).await
}

pub(super) async fn run_transcription(
    app: &AppHandle,
    audio: &CapturedAudio,
    cfg: &store::PipelineConfig,
    gen: u64,
    telemetry: &PipelineTelemetry,
) -> Option<(String, String, Option<TranscriptCandidate>)> {
    log::debug!(
        "pipeline: transcription stage start gen={} provider={} model={} language={} wav_bytes={} pcm_samples={}",
        gen,
        cfg.transcription_provider,
        cfg.transcription_default_model,
        cfg.transcription_language,
        audio.wav_len(),
        audio.samples_16k.len()
    );

    let dual_enabled = cfg.dual_transcription_enabled && transcription_model_chain(cfg).len() > 1;
    let (raw, provider_id, model, alternate_result) = match if dual_enabled {
        run_dual_transcription_candidates(app, audio, cfg, gen).await
    } else {
        run_primary_transcription_chain(app, audio, cfg, gen, telemetry)
            .await
            .map(|(raw, provider, model)| (raw, provider, model, None))
    } {
        Ok((raw, provider_id, model, alternate)) => (raw, provider_id, model, alternate),
        Err(error) => {
            let user_msg = crate::api::user_facing_error(&error);
            log::error!(
                "pipeline: transcription failed gen={} error={}",
                gen,
                trim_err(&error.to_string())
            );
            if crate::api::is_connectivity_error(&error) && confirm_offline(app).await {
                emit_connectivity_recheck(app);
                show_error_pill(
                    app,
                    "No internet connection — check your network and try again.",
                )
                .await;
                return None;
            }
            if crate::api::is_retryable_provider_error(&error) {
                emit_provider_recheck(app);
            }
            show_error_pill(app, &user_msg).await;
            return None;
        }
    };

    let corroborated_candidate = alternate_result.as_ref().is_some_and(|(alternate, _, _)| {
        let primary = prepare_transcript_text(alternate, false, true);
        let alternate = prepare_transcript_text(&raw, false, true);
        !primary.trim().is_empty() && primary.trim().eq_ignore_ascii_case(alternate.trim())
    });
    let primary_text =
        prepare_transcript_text(&raw, alternate_result.is_some(), corroborated_candidate);
    if primary_text.is_empty() {
        show_error_pill(
            app,
            "Nothing transcribed - please try speaking more clearly",
        )
        .await;
        return None;
    }
    let alternate = alternate_result.and_then(|(text, provider, model)| {
        let text = prepare_transcript_text(&text, true, corroborated_candidate);
        (!text.is_empty()).then_some(TranscriptCandidate {
            text,
            provider,
            model,
        })
    });
    let mut notices = Vec::new();
    if provider_id == store::LOCAL && cfg.transcription_provider != store::LOCAL {
        notices.push("Used local speech models because cloud transcription was unavailable.");
    }
    if cfg.dual_transcription_enabled && alternate.is_none() {
        notices.push(
            "Only one speech model succeeded. The second transcription check was unavailable.",
        );
    }
    if !notices.is_empty() {
        app.emit("verenu:model-notice", notices.join(" ")).ok();
    }
    let api_used = match &alternate {
        Some(candidate) => format!(
            "primary={}/{};secondary={}/{}",
            provider_id, model, candidate.provider, candidate.model
        ),
        None => format!("{provider_id}/{model}/transcription"),
    };
    Some((primary_text, api_used, alternate))
}

const DUAL_TRANSCRIPTION_TIMEOUT_SECS: u64 = 30;

type CandidateOutcome = (usize, String, String, anyhow::Result<String>);

async fn run_dual_transcription_candidates(
    app: &AppHandle,
    audio: &CapturedAudio,
    cfg: &store::PipelineConfig,
    gen: u64,
) -> anyhow::Result<(String, String, String, Option<(String, String, String)>)> {
    let chain = runtime_model_chain(Some(app), cfg, "transcription");
    let mut next_index = 0usize;
    let mut in_flight = tokio::task::JoinSet::<CandidateOutcome>::new();
    let mut successes = Vec::<(usize, String, String, String)>::new();
    let mut last_err: Option<anyhow::Error> = None;

    let mut offline = crate::system::connectivity::recently_confirmed_offline();
    loop {
        while next_index < chain.len() && successes.len() + in_flight.len() < 2 {
            let (provider, _) = &chain[next_index];
            if !candidate_available_offline(provider, offline) {
                next_index += 1;
                continue;
            }
            if provider == store::LOCAL && !in_flight.is_empty() {
                break;
            }
            spawn_transcription_candidate(
                &mut in_flight,
                app,
                audio,
                cfg,
                next_index,
                chain[next_index].clone(),
                next_index > 0 && provider != store::LOCAL,
                gen,
            );
            next_index += 1;
            if provider == store::LOCAL {
                break;
            }
        }
        let Some(joined) = in_flight.join_next().await else {
            break;
        };
        match joined {
            Ok((index, provider, model, Ok(text))) if !text.trim().is_empty() => {
                log::debug!(
                    "pipeline: dual transcription candidate success index={} provider={} model={} chars={}",
                    index,
                    provider,
                    model,
                    text.chars().count()
                );
                successes.push((index, text, provider, model));
                if successes.len() >= 2 {
                    in_flight.abort_all();
                    break;
                }
            }
            Ok((index, provider, model, Ok(_))) => {
                log::warn!(
                    "pipeline: dual transcription candidate empty index={} provider={} model={}",
                    index,
                    provider,
                    model
                );
            }
            Ok((index, provider, model, Err(error))) => {
                log::warn!(
                    "pipeline: dual transcription candidate failed index={} provider={} model={} error={}",
                    index,
                    provider,
                    model,
                    trim_err(&error.to_string())
                );
                if provider != store::LOCAL
                    && (crate::api::is_connectivity_error(&error)
                        || error.to_string().contains("timeout"))
                    && !offline
                {
                    offline = confirm_offline(app).await;
                }
                last_err = Some(error);
            }
            Err(error) => {
                log::warn!("pipeline: dual transcription task failed error={error}");
            }
        }
    }

    successes.sort_by_key(|(index, _, _, _)| *index);
    let mut successes = successes.into_iter();
    let Some((_, primary_text, primary_provider, primary_model)) = successes.next() else {
        if let Some(error) = last_err {
            return Err(error);
        }
        anyhow::bail!("Nothing transcribed - please try speaking more clearly");
    };
    let alternate = successes
        .next()
        .map(|(_, text, provider, model)| (text, provider, model));
    Ok((primary_text, primary_provider, primary_model, alternate))
}

#[allow(clippy::too_many_arguments)]
fn spawn_transcription_candidate(
    in_flight: &mut tokio::task::JoinSet<CandidateOutcome>,
    app: &AppHandle,
    audio: &CapturedAudio,
    cfg: &store::PipelineConfig,
    index: usize,
    candidate: (String, String),
    bounded: bool,
    gen: u64,
) {
    let app = app.clone();
    let audio = audio.clone();
    let key = cfg.key_for_request(&candidate.0);
    let language = cfg.transcription_language.clone();
    let customs = cfg.custom_providers.clone();
    let vocabulary = cfg.speech_vocabulary.clone();
    in_flight.spawn(async move {
        let (provider, model) = candidate;
        let request = transcribe_any(
            &app,
            &audio,
            &provider,
            &customs,
            if key.is_empty() { None } else { Some(key.as_str()) },
            &language,
            &model,
            gen,
            &vocabulary,
        );
        let result = if bounded {
            match tokio::time::timeout(
                std::time::Duration::from_secs(DUAL_TRANSCRIPTION_TIMEOUT_SECS),
                request,
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!(
                    "secondary transcription timed out after {} seconds",
                    DUAL_TRANSCRIPTION_TIMEOUT_SECS
                )),
            }
        } else {
            request.await
        };
        (index, provider, model, result)
    });
}

fn prepare_transcript_text(
    raw: &str,
    strip_provider_artifacts: bool,
    preserve_corroborated_artifacts: bool,
) -> String {
    let normalized = normalize_transcription_math_artifacts(raw);
    let normalized = if preserve_corroborated_artifacts {
        normalized
    } else {
        strip_trailing_hallucination(&strip_hallucinated_suffix(&normalized))
    };
    let normalized = crate::system::text::collapse_degenerate_word_runs(&normalized);
    if strip_provider_artifacts && !preserve_corroborated_artifacts {
        crate::pipeline::gates::strip_provider_artifacts(&normalized)
    } else {
        normalized
    }
}

async fn run_primary_transcription_chain(
    app: &AppHandle,
    audio: &CapturedAudio,
    cfg: &store::PipelineConfig,
    gen: u64,
    telemetry: &PipelineTelemetry,
) -> anyhow::Result<(String, String, String)> {
    let mut last_err: Option<anyhow::Error> = None;
    let mut offline = crate::system::connectivity::recently_confirmed_offline();
    for (provider_index, (provider_id, model)) in
        runtime_model_chain(Some(app), cfg, "transcription")
            .into_iter()
            .enumerate()
    {
        if !candidate_available_offline(&provider_id, offline) {
            continue;
        }
        if provider_index > 0 {
            telemetry.fallback("transcription");
        }
        let key = cfg.key_for_request(&provider_id);
        match transcribe_any(
            app,
            audio,
            &provider_id,
            &cfg.custom_providers,
            if key.is_empty() {
                None
            } else {
                Some(key.as_str())
            },
            &cfg.transcription_language,
            &model,
            gen,
            &cfg.speech_vocabulary,
        )
        .await
        {
            Ok(raw) if !raw.is_empty() => {
                log::debug!(
                    "pipeline: transcription provider success gen={} provider={} model={} chars={}",
                    gen,
                    provider_id,
                    model,
                    raw.chars().count()
                );
                return Ok((raw, provider_id, model));
            }
            Ok(_) => {}
            Err(e) => {
                let retryable = crate::api::is_retryable_provider_error(&e);
                if provider_id != store::LOCAL
                    && (crate::api::is_connectivity_error(&e) || e.to_string().contains("timeout"))
                    && !offline
                {
                    offline = confirm_offline(app).await;
                }
                log::warn!(
                    "pipeline: transcription provider failed gen={} provider={} model={} retryable={} error={}",
                    gen,
                    provider_id,
                    model,
                    retryable,
                    trim_err(&e.to_string())
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!("Nothing transcribed - please try speaking more clearly")
    }))
}
