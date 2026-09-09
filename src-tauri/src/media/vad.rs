//! Local voice-activity detection, used to decide whether a recording
//! actually contains speech instead of relying on raw RMS loudness alone.
//!
//! Runs Silero VAD (via `transcribe_rs::vad::SileroVad`, already bundled
//! through the `onnx`/`vad-silero` crate features shared with local
//! transcription) over the recording's 16kHz samples in fixed 30ms frames.
//! This is CPU-only, local, and fast — Silero's own benchmarks put a single
//! frame at well under 1ms — so a full recording's worth of frames is cheap
//! enough to run synchronously inside a blocking task without adding
//! perceptible latency, especially since the caller runs it concurrently
//! with the network transcription call rather than gating on it first.

use crate::data::store;

/// Aggregate result of running VAD across an entire recording.
///
/// `contains_speech` drives `pipeline::passes_speech_gate` without carrying
/// any dictated content across the VAD boundary.
#[derive(Debug, Clone, Copy)]
pub struct SpeechDetectionResult {
    pub contains_speech: bool,
}

/// Silero's fixed frame size for its v4 ONNX graph: 30ms at 16kHz.
const FRAME_SAMPLES: usize = 480;
const FRAME_MS: u64 = 30;

/// Per-frame speech/non-speech cutoff — transcribe-rs's own documented
/// recommended default for this model.
const SPEECH_PROBABILITY_THRESHOLD: f32 = 0.3;

// Acceptance thresholds at the app's default mic gain. Scaled down for
// higher gain via `gain_leniency_scale` below — starting points, not final
// tuned values (per the design this was built against).
const MIN_SPEECH_MS_BASE: u64 = 300;
const MIN_SPEECH_RATIO_BASE: f32 = 0.12;
const MIN_LONGEST_RUN_MS_BASE: u64 = 250;

fn speech_evidence_passes(
    speech_ms: u64,
    total_ms: u64,
    longest_run_ms: u64,
    min_speech_ms: u64,
    min_speech_ratio: f32,
    min_longest_run_ms: u64,
) -> bool {
    if total_ms == 0 {
        return false;
    }

    let speech_ratio = speech_ms as f32 / total_ms as f32;
    speech_ms >= min_speech_ms
        && (speech_ratio >= min_speech_ratio || longest_run_ms >= min_longest_run_ms)
}

/// The Silero v4 ONNX model, bundled directly into the binary. At ~1.8MB
/// this is small enough that shipping it as a Tauri bundle resource (with
/// its own resource-path resolution at runtime) isn't worth the extra
/// moving part — `include_bytes!` keeps dev and packaged builds identical.
static MODEL_BYTES: &[u8] = include_bytes!("../../assets/silero_vad_v4.onnx");

/// `SileroVad::new` only accepts a file path (it calls onnxruntime's
/// `commit_from_file`), so the embedded bytes are staged to a stable path
/// once per process and reused — writing 1.8MB to disk on every dictation
/// would defeat the point of keeping this cheap.
fn staged_model_path() -> anyhow::Result<std::path::PathBuf> {
    static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    static STAGE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    if let Some(path) = PATH.get() {
        return Ok(path.clone());
    }

    let _stage_guard = STAGE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("Silero VAD model staging lock was poisoned"))?;

    if let Some(path) = PATH.get() {
        return Ok(path.clone());
    }

    let stage_dir = crate::app_data_dir().join("runtime");
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| anyhow::anyhow!("failed to create Silero VAD runtime directory: {e}"))?;
    let path = stage_dir.join("verenu_silero_vad_v4.onnx");
    if !staged_model_matches(&path) {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let temp_path = stage_dir.join(format!(
            "verenu_silero_vad_v4-{}-{unique_suffix}.onnx.tmp",
            std::process::id()
        ));
        if let Err(error) = std::fs::write(&temp_path, MODEL_BYTES) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(anyhow::anyhow!("failed to stage Silero VAD model: {error}"));
        }
        if !staged_model_matches(&temp_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(anyhow::anyhow!(
                "staged Silero VAD model failed integrity verification"
            ));
        }
        if staged_model_matches(&path) {
            let _ = std::fs::remove_file(&temp_path);
        } else {
            if path.exists() {
                if let Err(error) = std::fs::remove_file(&path) {
                    let _ = std::fs::remove_file(&temp_path);
                    return Err(anyhow::anyhow!(
                        "failed to replace staged Silero VAD model: {error}"
                    ));
                }
            }
            if let Err(error) = std::fs::rename(&temp_path, &path) {
                let _ = std::fs::remove_file(&temp_path);
                return Err(anyhow::anyhow!(
                    "failed to publish staged Silero VAD model: {error}"
                ));
            }
        }
        if !staged_model_matches(&path) {
            return Err(anyhow::anyhow!(
                "published Silero VAD model failed integrity verification"
            ));
        }
    }
    let _ = PATH.set(path.clone());
    Ok(path)
}

fn staged_model_matches(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.len() != MODEL_BYTES.len() as u64 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    bytes.as_slice() == MODEL_BYTES
}

/// Streaming speech detector for the active recording. It shares the same
/// Silero model, frame size, and evidence thresholds as the final
/// recording-wide VAD, but evaluates them incrementally so the pill can
/// acknowledge speech without waiting for the recording to stop. Requiring
/// the aggregate gate here is intentional: a monitor bump or a hard breath
/// can look speech-like for one frame, but should not earn a checkmark.
pub struct LiveSpeechDetector {
    #[cfg(not(target_os = "android"))]
    vad: Option<transcribe_rs::vad::SileroVad>,
    frame: Vec<f32>,
    fallback_rms: f32,
    total_ms: u64,
    speech_ms: u64,
    current_run_ms: u64,
    longest_run_ms: u64,
    min_speech_ms: u64,
    min_speech_ratio: f32,
    min_longest_run_ms: u64,
}

impl LiveSpeechDetector {
    pub fn new(active_gain: f32) -> Self {
        #[cfg(not(target_os = "android"))]
        let vad = match staged_model_path() {
            Ok(path) => {
                match transcribe_rs::vad::SileroVad::new(&path, SPEECH_PROBABILITY_THRESHOLD) {
                    Ok(vad) => Some(vad),
                    Err(error) => {
                        log::warn!("live VAD unavailable, using the RMS fallback: {error}");
                        None
                    }
                }
            }
            Err(error) => {
                log::warn!("live VAD model unavailable, using the RMS fallback: {error}");
                None
            }
        };

        let scale = gain_leniency_scale(active_gain);

        Self {
            #[cfg(not(target_os = "android"))]
            vad,
            frame: Vec::with_capacity(FRAME_SAMPLES * 2),
            fallback_rms: live_fallback_rms(active_gain),
            total_ms: 0,
            speech_ms: 0,
            current_run_ms: 0,
            longest_run_ms: 0,
            min_speech_ms: (MIN_SPEECH_MS_BASE as f32 * scale) as u64,
            min_speech_ratio: MIN_SPEECH_RATIO_BASE * scale,
            min_longest_run_ms: (MIN_LONGEST_RUN_MS_BASE as f32 * scale) as u64,
        }
    }

    /// Feeds processed 16 kHz samples and returns true once the same evidence
    /// gate used after recording has passed. The caller can keep the result
    /// latched for the rest of the recording.
    pub fn push(&mut self, samples_16k: &[f32]) -> bool {
        self.frame.extend_from_slice(samples_16k);
        while self.frame.len() >= FRAME_SAMPLES {
            let frame = &self.frame[..FRAME_SAMPLES];
            let fallback_detected = crate::media::audio::rms_f32(frame) >= self.fallback_rms;

            #[cfg(not(target_os = "android"))]
            let detected = {
                let probability = self.vad.as_mut().map(|vad| vad.speech_probability(frame));
                match probability {
                    Some(Ok(probability)) => probability >= SPEECH_PROBABILITY_THRESHOLD,
                    Some(Err(error)) => {
                        log::warn!(
                            "live VAD inference failed, switching to the RMS fallback: {error}"
                        );
                        self.vad = None;
                        fallback_detected
                    }
                    None => fallback_detected,
                }
            };

            #[cfg(target_os = "android")]
            let detected = fallback_detected;

            self.frame.drain(..FRAME_SAMPLES);
            self.total_ms += FRAME_MS;
            if detected {
                self.speech_ms += FRAME_MS;
                self.current_run_ms += FRAME_MS;
                self.longest_run_ms = self.longest_run_ms.max(self.current_run_ms);
            } else {
                self.current_run_ms = 0;
            }

            if speech_evidence_passes(
                self.speech_ms,
                self.total_ms,
                self.longest_run_ms,
                self.min_speech_ms,
                self.min_speech_ratio,
                self.min_longest_run_ms,
            ) {
                return true;
            }
        }
        false
    }
}

/// Mirrors the recording gate's gain normalization for the live fallback.
/// The processed samples already include the configured gain, so the fallback
/// threshold must move with that gain to avoid penalizing a quiet microphone.
fn live_fallback_rms(active_gain: f32) -> f32 {
    const BASE_RMS: f32 = 0.005;
    let gain = active_gain.clamp(store::MIN_MIC_GAIN, store::MAX_MIC_GAIN);
    if gain <= store::DEFAULT_MIC_GAIN {
        BASE_RMS * gain / store::DEFAULT_MIC_GAIN
    } else {
        BASE_RMS * store::DEFAULT_MIC_GAIN / gain
    }
}

/// Scales how lenient the speech thresholds are with the active mic gain,
/// mirroring `pipeline::gates::recording_gate_rms`'s normalization: a user
/// who raised gain for a quiet voice or a distant mic already told the app
/// their raw signal is faint, so the bar for "this looks like speech" comes
/// down proportionally instead of penalizing them twice for the same thing.
/// Floored at 0.4 rather than scaling to zero — VAD still needs *some*
/// signal to tell speech from a fan.
fn gain_leniency_scale(active_gain: f32) -> f32 {
    let gain = active_gain.clamp(store::MIN_MIC_GAIN, store::MAX_MIC_GAIN);
    if gain <= store::DEFAULT_MIC_GAIN {
        1.0
    } else {
        (store::DEFAULT_MIC_GAIN / gain).max(0.4)
    }
}

/// Runs VAD over an entire recording and judges whether it contains speech.
///
/// Blocking (ONNX inference) — call from `spawn_blocking`, ideally started
/// concurrently with the transcription API call so it adds no wall-clock
/// latency of its own.
#[allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
pub fn analyze_speech_with_sensitivity(
    samples_16k: &[f32],
    active_gain: f32,
    sensitivity_level: u8,
) -> anyhow::Result<SpeechDetectionResult> {
    let adaptive_scale = crate::pipeline::adaptive_sensitivity_scale(sensitivity_level);
    let probability_threshold = (SPEECH_PROBABILITY_THRESHOLD * adaptive_scale).max(0.16);
    let model_path = staged_model_path()?;
    let mut vad = transcribe_rs::vad::SileroVad::new(&model_path, probability_threshold)
        .map_err(|e| anyhow::anyhow!("failed to load Silero VAD model: {e}"))?;

    let mut speech_ms: u64 = 0;
    let mut longest_run_ms: u64 = 0;
    let mut current_run_ms: u64 = 0;
    let mut frame_count: u64 = 0;

    for frame in samples_16k.chunks_exact(FRAME_SAMPLES) {
        frame_count += 1;
        let probability = vad
            .speech_probability(frame)
            .map_err(|e| anyhow::anyhow!("Silero VAD inference failed: {e}"))?;
        if probability >= probability_threshold {
            speech_ms += FRAME_MS;
            current_run_ms += FRAME_MS;
            longest_run_ms = longest_run_ms.max(current_run_ms);
        } else {
            current_run_ms = 0;
        }
    }

    let total_ms = frame_count * FRAME_MS;
    let scale = gain_leniency_scale(active_gain) * adaptive_scale;
    let min_speech_ms = (MIN_SPEECH_MS_BASE as f32 * scale) as u64;
    let min_ratio = MIN_SPEECH_RATIO_BASE * scale;
    let min_longest_run_ms = (MIN_LONGEST_RUN_MS_BASE as f32 * scale) as u64;

    let contains_speech = speech_evidence_passes(
        speech_ms,
        total_ms,
        longest_run_ms,
        min_speech_ms,
        min_ratio,
        min_longest_run_ms,
    );

    Ok(SpeechDetectionResult { contains_speech })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_leniency_scale_is_neutral_at_default_gain() {
        assert_eq!(gain_leniency_scale(store::DEFAULT_MIC_GAIN), 1.0);
    }

    #[test]
    fn gain_leniency_scale_relaxes_thresholds_for_boosted_gain() {
        let scale = gain_leniency_scale(store::MAX_MIC_GAIN);
        assert!(scale < 1.0);
        assert!(scale >= 0.4);
    }

    #[test]
    fn gain_leniency_scale_never_goes_below_floor() {
        // active_gain is clamped to MAX_MIC_GAIN before scaling, so the floor
        // is only reachable if the ratio itself would go under 0.4 within
        // the valid gain range — assert the invariant instead of a specific
        // value baked in from an out-of-range input.
        assert!(gain_leniency_scale(store::MAX_MIC_GAIN) >= 0.4);
    }

    #[test]
    fn minimum_gain_uses_unscaled_thresholds() {
        // At minimum gain, VAD must judge the raw signal on the full
        // thresholds rather than the relaxed ones meant for boosted mics.
        assert_eq!(gain_leniency_scale(1.0), 1.0);
    }

    #[test]
    fn analyze_speech_on_digital_silence_finds_no_speech() {
        let silence = vec![0.0f32; 16_000]; // 1s of exact silence
        let result = analyze_speech_with_sensitivity(&silence, store::DEFAULT_MIC_GAIN, 0)
            .expect("model should load and run on staged path");
        assert!(!result.contains_speech);
    }
}
