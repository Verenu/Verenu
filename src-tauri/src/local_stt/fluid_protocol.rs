//! Compact little-endian float PCM for the private helper pipe.
use crate::api::base64_audio::Base64Audio;
use bytes::Bytes;
use serde::Serialize;
use std::{path::Path, sync::atomic::AtomicBool};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Transcription<'a> {
    operation: &'static str,
    audio_pcm: Base64Audio,
    language: &'a str,
    vocabulary: &'a [String],
    booster_path: Option<&'a Path>,
}

pub(super) fn transcription_payload(
    samples: &[f32],
    language: &str,
    vocabulary: &[String],
    booster_path: Option<&Path>,
    cancel: &AtomicBool,
) -> anyhow::Result<Vec<u8>> {
    let mut pcm = Vec::with_capacity(samples.len() * 4);
    for chunk in samples.chunks(16_000) {
        crate::api::model_download::ensure_not_cancelled(cancel)?;
        for sample in chunk {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
    }
    let audio_bytes = pcm.len();
    let metadata_bytes = language.len()
        + vocabulary.iter().map(String::len).sum::<usize>()
        + booster_path.map_or(0, |path| path.as_os_str().len());
    let capacity = base64::encoded_len(audio_bytes, true)
        .and_then(|size| size.checked_add(metadata_bytes.checked_mul(6)?))
        .and_then(|size| size.checked_add(1024))
        .ok_or_else(|| anyhow::anyhow!("audio request size overflow"))?;
    let body = Transcription {
        operation: "transcribe",
        audio_pcm: Base64Audio(Bytes::from(pcm)),
        language,
        vocabulary,
        booster_path,
    };
    let mut payload = Vec::with_capacity(capacity);
    serde_json::to_writer(&mut payload, &body)?;
    crate::api::model_download::ensure_not_cancelled(cancel)?;
    payload.push(b'\n');
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn helper_pcm_contract_preserves_exact_samples_and_metadata() {
        let samples = [0.0f32, -0.0, 0.125, -0.75, 1.0];
        let terms = vec!["public synthetic \"term\"".to_string()];
        let payload = transcription_payload(
            &samples,
            "en",
            &terms,
            Some(Path::new("synthetic-booster")),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(payload.last(), Some(&b'\n'));
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert!(value.get("samples").is_none());
        assert_eq!(value["operation"], "transcribe");
        assert_eq!(value["language"], "en");
        assert_eq!(value["vocabulary"], serde_json::json!(terms));
        assert_eq!(value["boosterPath"], "synthetic-booster");
        let pcm = STANDARD
            .decode(value["audioPcm"].as_str().unwrap())
            .unwrap();
        let decoded: Vec<f32> = pcm
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        assert_eq!(
            decoded.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
            samples.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn helper_pcm_preparation_observes_cancellation() {
        assert!(
            transcription_payload(&[0.0], "auto", &[], None, &AtomicBool::new(true))
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
}
