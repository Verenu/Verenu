#[cfg(not(target_os = "android"))]
use super::model::LocalSttEngineType;
use super::model::LocalSttModelManifest;
use std::path::Path;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::canary::CanaryModel;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::cohere::CohereModel;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::gigaam::GigaAMModel;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::moonshine::{MoonshineModel, MoonshineVariant, StreamingModel};
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::parakeet::ParakeetModel;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::sense_voice::SenseVoiceModel;
#[cfg(not(target_os = "android"))]
use transcribe_rs::onnx::Quantization;
#[cfg(not(target_os = "android"))]
use transcribe_rs::{SpeechModel, TranscribeOptions};

/// Number of inference threads for streaming Moonshine models. Matches the
/// crate's own example default (examples/moonshine_streaming.rs) — there is
/// no per-platform tuning need here, ONNX Runtime CPU inference scales fine
/// at this thread count for a model this small.
#[cfg(not(target_os = "android"))]
const MOONSHINE_STREAMING_THREADS: usize = 4;

#[cfg(not(target_os = "android"))]
pub enum LoadedLocalSttEngine {
    Parakeet(ParakeetModel),
    Moonshine(MoonshineModel),
    MoonshineStreaming(StreamingModel),
    SenseVoice(SenseVoiceModel),
    GigaAm(GigaAMModel),
    Canary(CanaryModel),
    Cohere(CohereModel),
}

#[cfg(not(target_os = "android"))]
impl LoadedLocalSttEngine {
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        language: &str,
    ) -> anyhow::Result<String> {
        if sample_rate != 16_000 {
            anyhow::bail!("local transcription requires 16 kHz mono PCM")
        }
        let options = TranscribeOptions {
            language: if language.trim().is_empty() {
                None
            } else {
                Some(language.to_string())
            },
            ..Default::default()
        };
        let result = match self {
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

#[cfg(not(target_os = "android"))]
pub fn load_engine(
    manifest: &LocalSttModelManifest,
    model_path: &Path,
) -> anyhow::Result<LoadedLocalSttEngine> {
    match manifest.engine_type {
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

// ---------------------------------------------------------------------------
// Android stub
// ---------------------------------------------------------------------------
//
// `transcribe-rs`/ONNX Runtime ships no Android ARM64 build (see
// `crate::android::local_ai_supported_on_android` and `docs/ANDROID.md`), so
// the crate is not a dependency on that target and this module exposes the
// same type/function surface as graceful errors instead. `manager.rs` already
// treats `load_engine` failure as "model unavailable", so no caller changes
// were needed; cloud transcription is the supported Android path.

/// Placeholder engine handle on Android. Never successfully constructed —
/// see [`load_engine`].
#[cfg(target_os = "android")]
#[allow(dead_code)]
pub enum LoadedLocalSttEngine {
    Unsupported,
}

#[cfg(target_os = "android")]
impl LoadedLocalSttEngine {
    pub fn transcribe(
        &mut self,
        _samples: &[f32],
        _sample_rate: u32,
        _language: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!(crate::android::LOCAL_AI_ANDROID_UNSUPPORTED_REASON)
    }
}

#[cfg(target_os = "android")]
pub fn load_engine(
    _manifest: &LocalSttModelManifest,
    _model_path: &Path,
) -> anyhow::Result<LoadedLocalSttEngine> {
    anyhow::bail!(crate::android::LOCAL_AI_ANDROID_UNSUPPORTED_REASON)
}
