use base64::{display::Base64Display, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Serialize, Serializer};

/// Encode directly into the JSON serializer instead of allocating a second
/// recording-sized base64 String beside the serialized request body.
pub struct Base64Audio(pub Bytes);

impl std::fmt::Debug for Base64Audio {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Base64Audio")
            .field("bytes", &self.0.len())
            .finish()
    }
}

impl Serialize for Base64Audio {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&Base64Display::new(&self.0, &STANDARD))
    }
}

/// Reserve the encoded audio plus a bound for escaped metadata. Incremental
/// base64 writes would otherwise grow the JSON buffer geometrically.
pub fn json_request<T: Serialize>(
    request: reqwest::RequestBuilder,
    body: &T,
    audio_bytes: usize,
    text_bytes: usize,
) -> anyhow::Result<reqwest::RequestBuilder> {
    let capacity = base64::encoded_len(audio_bytes, true)
        .and_then(|length| length.checked_add(text_bytes.checked_mul(6)?))
        .and_then(|length| length.checked_add(1024))
        .ok_or_else(|| anyhow::anyhow!("audio request size overflow"))?;
    let mut json = Vec::with_capacity(capacity);
    serde_json::to_writer(&mut json, body)?;
    Ok(request
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn json_encoding_matches_standard_base64_across_padding_and_chunks() {
        for length in [0, 1, 2, 3, 4, 511, 512, 513, 1024, 16_000 * 2] {
            let bytes = Bytes::from((0..length).map(|i| i as u8).collect::<Vec<_>>());
            let expected = serde_json::to_vec(&STANDARD.encode(&bytes)).unwrap();
            let actual = serde_json::to_vec(&Base64Audio(bytes)).unwrap();
            assert_eq!(actual, expected, "audio bytes={length}");
        }
    }

    #[test]
    fn request_preserves_json_headers_auth_and_escaped_metadata() {
        #[derive(Serialize)]
        struct Request<'a> {
            data: Base64Audio,
            prompt: &'a str,
        }
        let wav = Bytes::from_static(b"RIFF");
        let prompt = "Quote \"\\\n\t\u{0001} é";
        let body = Request {
            data: Base64Audio(wav.clone()),
            prompt,
        };
        let request = json_request(
            reqwest::Client::new()
                .post("https://example.invalid/transcribe")
                .bearer_auth("test-key"),
            &body,
            wav.len(),
            prompt.len(),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            request.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION],
            "Bearer test-key"
        );
        let json: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(json["data"], "UklGRg==");
        assert_eq!(json["prompt"], prompt);
    }
}
