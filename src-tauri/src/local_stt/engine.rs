use super::model::LocalSttEngineType;
use super::model::LocalSttModelManifest;
use std::path::Path;
use transcribe_rs::onnx::canary::CanaryModel;
use transcribe_rs::onnx::cohere::CohereModel;
use transcribe_rs::onnx::gigaam::GigaAMModel;
use transcribe_rs::onnx::moonshine::{MoonshineModel, MoonshineVariant, StreamingModel};
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::sense_voice::SenseVoiceModel;
use transcribe_rs::onnx::Quantization;
#[cfg(not(target_os = "android"))]
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperInferenceParams, WhisperLoadParams};
use transcribe_rs::{SpeechModel, TranscribeOptions};

/// Number of inference threads for streaming Moonshine models. Matches the
/// crate's own example default (examples/moonshine_streaming.rs) — there is
/// no per-platform tuning need here, ONNX Runtime CPU inference scales fine
/// at this thread count for a model this small.
const MOONSHINE_STREAMING_THREADS: usize = 4;

pub enum LoadedLocalSttEngine {
    Parakeet(ParakeetModel),
    Moonshine(MoonshineModel),
    MoonshineStreaming(StreamingModel),
    SenseVoice(SenseVoiceModel),
    GigaAm(GigaAMModel),
    Canary(CanaryModel),
    Cohere(CohereModel),
    #[cfg(not(target_os = "android"))]
    Whisper(WhisperEngine),
    #[cfg(target_os = "macos")]
    AppleSpeech,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    FluidAudio(super::fluid::FluidEngine),
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    #[test]
    fn local_audio_validation_preserves_recorder_duration_limit() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut samples = vec![0.0; crate::media::audio::MAX_RECORDING_SAMPLES];
        assert!(validate_audio(&samples, 16_000, &cancel).is_ok());
        samples.push(0.0);
        assert!(validate_audio(&samples, 16_000, &cancel).is_err());
        samples.pop();
        samples[0] = f32::NAN;
        assert!(validate_audio(&samples, 16_000, &cancel).is_err());
        assert!(validate_audio(&[0.0], 44_100, &cancel).is_err());
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert!(validate_audio(&[0.0], 16_000, &cancel).is_err());
    }
    #[test]
    #[ignore = "requires explicitly downloaded GGML model and public synthetic 16 kHz WAV"]
    fn whisper_native_logs_do_not_expose_public_fixture() {
        // Run in a separate process to capture native stdout/stderr as well as
        // Rust test output. The inference test supplies both speech and hints.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "local_stt::engine::tests::whisper_real_synthetic_speech_and_silence",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "public inference probe failed");
        assert!(output.stdout.len() + output.stderr.len() < 65_536);
        for stream in [&output.stdout, &output.stderr] {
            let captured = String::from_utf8_lossy(stream);
            for sentinel in [
                "orange",
                "notebook",
                "kitchen",
                "Verenu",
                "prompt tokens",
                "whisper_full:",
            ] {
                assert!(
                    !captured.contains(sentinel),
                    "native speech content leaked to process logs"
                );
            }
        }
    }
    #[test]
    #[ignore = "requires explicitly downloaded GGML model and public synthetic 16 kHz WAV"]
    fn whisper_real_synthetic_speech_and_silence() {
        let model = std::env::var("VERENU_WHISPER_TEST_MODEL").expect("downloaded model path");
        let fixture = std::env::var("VERENU_WHISPER_TEST_AUDIO").expect("public synthetic WAV");
        let mut reader = hound::WavReader::open(fixture).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        let samples = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect::<Vec<_>>();
        let manifest = super::super::model::manifest_by_id("whisper-small").unwrap();
        let mut engine = load_engine(&manifest, Path::new(&model)).unwrap();
        let vocabulary = super::super::vocabulary::Vocabulary::from_terms(["Verenu"]);
        let cancellation = std::sync::atomic::AtomicBool::new(false);
        let result = engine
            .transcribe(&samples, 16_000, "en", &vocabulary, &cancellation)
            .unwrap();
        assert!(
            result.to_lowercase().contains("notebook") && result.to_lowercase().contains("kitchen"),
            "public synthetic speech must be recognized"
        );
        assert!(engine
            .transcribe(&vec![0.0; 32_000], 16_000, "en", &vocabulary, &cancellation)
            .unwrap()
            .is_empty());
        assert!(engine
            .transcribe(&vec![1e-6; 32_000], 16_000, "", &vocabulary, &cancellation)
            .unwrap()
            .is_empty());
        assert!(engine
            .transcribe(&[f32::NAN], 16_000, "en", &vocabulary, &cancellation)
            .is_err());
        cancellation.store(true, std::sync::atomic::Ordering::Release);
        assert!(engine
            .transcribe(&samples, 16_000, "en", &vocabulary, &cancellation)
            .is_err());
    }
}

fn validate_audio(
    samples: &[f32],
    sample_rate: u32,
    cancellation: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        sample_rate == 16_000,
        "local transcription requires 16 kHz mono PCM"
    );
    anyhow::ensure!(
        samples.len() <= crate::media::audio::MAX_RECORDING_SAMPLES
            && samples.iter().all(|s| s.is_finite()),
        "invalid local speech audio"
    );
    anyhow::ensure!(
        !cancellation.load(std::sync::atomic::Ordering::Acquire),
        "local transcription cancelled"
    );
    Ok(())
}

impl LoadedLocalSttEngine {
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        language: &str,
        vocabulary: &super::vocabulary::Vocabulary,
        cancellation: &std::sync::atomic::AtomicBool,
    ) -> anyhow::Result<String> {
        #[cfg(not(target_os = "macos"))]
        let _ = cancellation;
        #[cfg(target_os = "android")]
        let _ = vocabulary;
        validate_audio(samples, sample_rate, cancellation)?;
        let options = TranscribeOptions {
            language: if language.trim().is_empty() {
                None
            } else {
                Some(language.to_string())
            },
            ..Default::default()
        };
        let result = match self {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Self::FluidAudio(engine) => {
                return engine.transcribe(samples, language, vocabulary, cancellation)
            }
            #[cfg(target_os = "macos")]
            Self::AppleSpeech => {
                return super::apple::transcribe(samples, language, vocabulary, cancellation)
            }
            #[cfg(not(target_os = "android"))]
            Self::Whisper(model) => {
                if samples.is_empty() || samples.iter().all(|sample| sample.abs() < 1e-5) {
                    return Ok(String::new());
                }
                model.transcribe_with(
                    samples,
                    &WhisperInferenceParams {
                        language: options.language.clone(),
                        initial_prompt: vocabulary.whisper_prompt(),
                        no_speech_thold: 0.6,
                        ..Default::default()
                    },
                )
            }
            Self::Parakeet(model) => model.transcribe(samples, &options),
            Self::Moonshine(model) => model.transcribe(samples, &options),
            Self::MoonshineStreaming(model) => model.transcribe(samples, &options),
            Self::SenseVoice(model) => model.transcribe(samples, &options),
            Self::GigaAm(model) => model.transcribe(samples, &options),
            Self::Canary(model) => model.transcribe(samples, &options),
            Self::Cohere(model) => model.transcribe(samples, &options),
        }?;
        Ok(result.text.trim().to_string())
    }
}

pub fn load_engine(
    manifest: &LocalSttModelManifest,
    model_path: &Path,
) -> anyhow::Result<LoadedLocalSttEngine> {
    #[cfg(target_os = "android")]
    crate::android::local_ai::ensure_onnx_runtime()?;
    match manifest.engine_type {
        LocalSttEngineType::FluidAudio => {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                Ok(LoadedLocalSttEngine::FluidAudio(
                    super::fluid::FluidEngine::load(model_path, manifest.id)?,
                ))
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                anyhow::bail!("FluidAudio requires macOS 14 or later on Apple Silicon")
            }
        }
        LocalSttEngineType::CtcBooster => anyhow::bail!(
            "The vocabulary booster is an auxiliary download, not a transcription model"
        ),
        LocalSttEngineType::AppleSpeech => {
            #[cfg(target_os = "macos")]
            {
                Ok(LoadedLocalSttEngine::AppleSpeech)
            }
            #[cfg(not(target_os = "macos"))]
            {
                anyhow::bail!("Apple Speech is available on macOS only")
            }
        }
        LocalSttEngineType::Whisper => {
            #[cfg(not(target_os = "android"))]
            {
                // Native debug logs can contain prompt and transcript tokens.
                // With no logging backend enabled, both hooks discard them.
                whisper_rs::install_logging_hooks();
                Ok(LoadedLocalSttEngine::Whisper(
                    WhisperEngine::load_with_params(
                        model_path,
                        WhisperLoadParams {
                            use_gpu: false,
                            flash_attn: false,
                            ..Default::default()
                        },
                    )?,
                ))
            }
            #[cfg(target_os = "android")]
            {
                anyhow::bail!("Portable Whisper is available on desktop only")
            }
        }
        LocalSttEngineType::Parakeet => Ok(LoadedLocalSttEngine::Parakeet(ParakeetModel::load(
            model_path,
            &Quantization::Int8,
        )?)),
        LocalSttEngineType::Moonshine => Ok(LoadedLocalSttEngine::Moonshine(MoonshineModel::load(
            model_path,
            MoonshineVariant::Base,
            &Quantization::default(),
        )?)),
        LocalSttEngineType::MoonshineStreaming => Ok(LoadedLocalSttEngine::MoonshineStreaming(
            StreamingModel::load(model_path, MOONSHINE_STREAMING_THREADS, &Quantization::Int8)?,
        )),
        LocalSttEngineType::SenseVoice => Ok(LoadedLocalSttEngine::SenseVoice(
            SenseVoiceModel::load(model_path, &Quantization::Int8)?,
        )),
        LocalSttEngineType::GigaAm => Ok(LoadedLocalSttEngine::GigaAm(GigaAMModel::load(
            model_path,
            &Quantization::Int8,
        )?)),
        LocalSttEngineType::Canary => Ok(LoadedLocalSttEngine::Canary(CanaryModel::load(
            model_path,
            &Quantization::Int8,
        )?)),
        LocalSttEngineType::Cohere => Ok(LoadedLocalSttEngine::Cohere(CohereModel::load(
            model_path,
            &Quantization::Int8,
        )?)),
    }
}
