use super::*;
use std::sync::Arc;
use std::time::Instant;

#[tauri::command]
pub fn get_model_performance() -> Vec<crate::model_performance::ModelPerformance> {
    crate::model_performance::snapshot()
}

#[derive(serde::Serialize)]
pub struct LocalBenchmark {
    initial_ms: u64,
    warm_ms: u64,
    cleanup_ms: Option<u64>,
    initially_loaded: bool,
}

struct BenchmarkReservation(SharedState);
impl Drop for BenchmarkReservation {
    fn drop(&mut self) {
        pipeline::cancel_starting_reservation(&self.0);
    }
}

/// Uses public bundled speech, never the microphone, history, or a provider API.
#[tauri::command]
pub async fn benchmark_local_models(
    app: AppHandle,
    speech_model: String,
    cleanup_model: Option<String>,
) -> Result<LocalBenchmark, String> {
    if crate::local_stt::model::manifest_by_id(&speech_model).is_none()
        || cleanup_model
            .as_ref()
            .is_some_and(|id| crate::local_llm::model::manifest_by_id(id).is_none())
    {
        return Err("Choose an installed local model.".into());
    }
    let state = app.state::<SharedState>().inner().clone();
    pipeline::reserve_starting(&state)?;
    let _reservation = BenchmarkReservation(state);
    let speech = app
        .state::<crate::local_stt::LocalTranscriptionManager>()
        .inner()
        .clone();
    let initially_loaded = speech.state().is_loaded
        && speech.state().current_model_id.as_deref() == Some(&speech_model);
    let mut wav = hound::WavReader::new(std::io::Cursor::new(include_bytes!(
        "../../../resources/model-speed-sample.wav"
    )))
    .map_err(|_| "Bundled speech sample is unavailable")?;
    let rate = wav.spec().sample_rate;
    let samples = Arc::new(
        wav.samples::<i16>()
            .map(|sample| sample.map(|value| f32::from(value) / 32768.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Bundled speech sample is invalid")?,
    );
    let duration_ms = samples.len() as f64 * 1000.0 / f64::from(rate);
    let mut timings = Vec::new();
    for run in 0..4 {
        let started = Instant::now();
        let text = crate::local_stt::transcribe::transcribe(
            speech.clone(),
            app.clone(),
            speech_model.clone(),
            Arc::clone(&samples),
            rate,
            "en".into(),
            Default::default(),
        )
        .await
        .map_err(|_| "Local speech test could not finish")?;
        if text.trim().is_empty() {
            return Err("Local speech test returned no text.".into());
        }
        let elapsed = started.elapsed().as_millis() as u64;
        timings.push(elapsed);
        if run > 0 {
            crate::model_performance::record(
                "transcription",
                "local",
                &speech_model,
                elapsed as f64 * 10_000.0 / duration_ms,
                true,
            );
        }
    }
    let mut cleanup_ms = None;
    if let Some(model) = cleanup_model {
        let cleanup = app.state::<crate::local_llm::LocalLlmManager>();
        let raw = "please send the meeting notes tomorrow morning";
        let prompt = "Return only the following sentence with sentence capitalization and punctuation: please send the meeting notes tomorrow morning";
        let mut elapsed_total = 0;
        for run in 0..4 {
            let started = Instant::now();
            let text = cleanup
                .cleanup_with_prompt(&app, &model, raw, prompt, 64)
                .await
                .map_err(|_| "Local cleanup test could not finish")?;
            if text.trim().is_empty() {
                return Err("Local cleanup test returned no text.".into());
            }
            let elapsed = started.elapsed().as_millis() as u64;
            if run > 0 {
                elapsed_total += elapsed;
                crate::model_performance::record(
                    "cleanup",
                    "local",
                    &model,
                    elapsed as f64 * 100.0 / raw.chars().count() as f64,
                    true,
                );
            }
        }
        cleanup_ms = Some(elapsed_total / 3);
    }
    Ok(LocalBenchmark {
        initial_ms: timings[0],
        warm_ms: timings[1..].iter().sum::<u64>() / 3,
        cleanup_ms,
        initially_loaded,
    })
}
