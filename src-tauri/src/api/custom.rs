//! User-defined providers: any OpenAI-, Anthropic-, or xAI-compatible endpoint.
//!
//! A custom provider is plain, non-secret configuration stored in
//! `settings.json` under `custom_providers`. Its API key lives in the native
//! credential store under `api_key_custom_<uuid>`, exactly like a built-in
//! provider's key. Identity is a random UUID, never the editable name, so
//! renaming a provider keeps its key and deleting one can find and remove it.
//!
//! Everything here is validated before it is stored, because the base URL and
//! headers decide where dictated audio and text are sent.

use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const CUSTOM_PREFIX: &str = "custom:";
/// Browser sessions must never reuse a host custom key with an edited endpoint.
pub fn native_credentials_available() -> bool {
    #[cfg(feature = "dev-session")]
    {
        !crate::dev_session::enabled()
    }
    #[cfg(not(feature = "dev-session"))]
    {
        true
    }
}
pub const MAX_CUSTOM_PROVIDERS: usize = 12;
const MAX_NAME_CHARS: usize = 40;
const MAX_MODELS_PER_TASK: usize = 100;
const MAX_MODEL_CHARS: usize = 200;
const MAX_HEADERS: usize = 16;
const MAX_HEADER_VALUE_CHARS: usize = 512;
const MAX_OVERRIDES_BYTES: usize = 4096;
/// Chat-completion fields Verenu computes itself. An override can add
/// `temperature` or a vendor flag, but never change what is being asked.
const PROTECTED_BODY_KEYS: &[&str] = &[
    "model",
    "messages",
    "system",
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "tools",
    "tool_choice",
];
/// Headers a custom provider may not set: they would break the request framing
/// or let a pasted definition smuggle in a cookie or proxy credential.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "proxy-authenticate",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CustomProtocol {
    Openai,
    Anthropic,
    Xai,
}

impl CustomProtocol {
    pub fn can_transcribe(self) -> bool {
        !matches!(self, Self::Anthropic)
    }

    fn transcription_path(self) -> Option<&'static str> {
        match self {
            Self::Openai => Some("audio/transcriptions"),
            Self::Xai => Some("stt"),
            Self::Anthropic => None,
        }
    }

    fn cleanup_path(self) -> &'static str {
        match self {
            Self::Openai | Self::Xai => "chat/completions",
            Self::Anthropic => "messages",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProvider {
    /// `custom:<uuid>`. Stable for the life of the provider.
    pub id: String,
    pub name: String,
    pub protocol: CustomProtocol,
    pub base_url: String,
    #[serde(default = "default_requires_key")]
    pub requires_key: bool,
    #[serde(default)]
    pub supports_transcription: bool,
    #[serde(default)]
    pub supports_cleanup: bool,
    /// Send the key raw in this header instead of the protocol default.
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// Extra JSON merged into the cleanup request body.
    #[serde(default)]
    pub body_overrides: Option<Map<String, Value>>,
    #[serde(default)]
    pub transcription_models: Vec<String>,
    #[serde(default)]
    pub cleanup_models: Vec<String>,
}

fn default_requires_key() -> bool {
    true
}

pub fn normalize_list(value: &Value) -> Result<Vec<CustomProvider>, String> {
    let mut providers: Vec<CustomProvider> = serde_json::from_value(value.clone())
        .map_err(|_| "Invalid custom provider configuration.".to_string())?;
    if providers.len() > MAX_CUSTOM_PROVIDERS {
        return Err(format!("At most {MAX_CUSTOM_PROVIDERS} custom providers."));
    }
    let mut seen = std::collections::HashSet::new();
    for provider in &mut providers {
        provider.normalize()?;
        if !seen.insert(provider.id.clone()) {
            return Err("Custom provider ids must be unique.".into());
        }
    }
    Ok(providers)
}

pub fn is_custom_id(id: &str) -> bool {
    parse_custom_id(id).is_some()
}

/// `custom:<uuid>` with a canonical lowercase hyphenated UUID, or `None`.
/// Strictness matters: the uuid becomes part of a keychain account name.
pub fn parse_custom_id(id: &str) -> Option<uuid::Uuid> {
    let rest = id.strip_prefix(CUSTOM_PREFIX)?;
    let parsed = uuid::Uuid::parse_str(rest).ok()?;
    (parsed.hyphenated().to_string() == rest).then_some(parsed)
}

#[cfg(test)]
pub fn new_custom_id() -> String {
    format!("{CUSTOM_PREFIX}{}", uuid::Uuid::new_v4().hyphenated())
}

/// Keychain account for a custom provider's key.
pub fn credential_account(id: &str) -> Option<String> {
    parse_custom_id(id).map(|u| {
        format!(
            "{}{}",
            crate::data::store::CUSTOM_KEY_PREFIX,
            u.hyphenated()
        )
    })
}

impl CustomProvider {
    /// Trims, canonicalizes, and checks every field. Call before storing.
    pub fn normalize(&mut self) -> Result<(), String> {
        if parse_custom_id(&self.id).is_none() {
            return Err("Invalid provider id.".into());
        }
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err("Give the provider a name.".into());
        }
        if self.name.chars().count() > MAX_NAME_CHARS || self.name.chars().any(|c| c.is_control()) {
            return Err(format!("Names are limited to {MAX_NAME_CHARS} characters."));
        }

        self.base_url = normalize_base_url(&self.base_url)?;

        if !self.protocol.can_transcribe() {
            self.supports_transcription = false;
        }
        if !self.supports_transcription && !self.supports_cleanup {
            return Err("Turn on transcription, cleanup, or both.".into());
        }

        self.auth_header = match self.auth_header.take() {
            Some(raw) if !raw.trim().is_empty() => Some(checked_header_name(raw.trim())?),
            _ => None,
        };

        if self.extra_headers.len() > MAX_HEADERS {
            return Err(format!("At most {MAX_HEADERS} extra headers."));
        }
        let mut headers = BTreeMap::new();
        for (name, value) in std::mem::take(&mut self.extra_headers) {
            let name = checked_header_name(name.trim())?;
            if matches!(
                name.as_str(),
                "authorization" | "x-api-key" | "api-key" | "x-goog-api-key"
            ) {
                return Err("Put credentials in the API key field, not extra headers.".into());
            }
            if self.auth_header.as_deref() == Some(name.as_str()) {
                return Err(format!(
                    "{name} is already the auth header; the key is sent there."
                ));
            }
            let value = value.trim().to_string();
            if value.chars().count() > MAX_HEADER_VALUE_CHARS {
                return Err(format!("The {name} header value is too long."));
            }
            reqwest::header::HeaderValue::from_str(&value)
                .map_err(|_| format!("The {name} header value is not valid."))?;
            headers.insert(name, value);
        }
        self.extra_headers = headers;

        if let Some(overrides) = &self.body_overrides {
            if overrides.is_empty() {
                self.body_overrides = None;
            } else {
                for key in overrides.keys() {
                    if PROTECTED_BODY_KEYS.contains(&key.as_str()) {
                        return Err(format!(
                            "\"{key}\" is set by Verenu and can't be overridden."
                        ));
                    }
                }
                let size = serde_json::to_vec(overrides).map(|b| b.len()).unwrap_or(0);
                if size > MAX_OVERRIDES_BYTES {
                    return Err("Body overrides are limited to 4 KB.".into());
                }
            }
        }

        self.transcription_models = clean_models(&self.transcription_models)?;
        self.cleanup_models = clean_models(&self.cleanup_models)?;
        if !self.supports_transcription {
            self.transcription_models.clear();
        }
        if !self.supports_cleanup {
            self.cleanup_models.clear();
        }
        Ok(())
    }

    /// Authentication and hardening for requests to this endpoint.
    pub fn wire(&self) -> super::Wire {
        super::Wire {
            auth_header: self.auth_header.clone(),
            anthropic: self.protocol == CustomProtocol::Anthropic,
            headers: self
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            hardened: true,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path)
    }

    pub fn cleanup_url(&self) -> String {
        self.url(self.protocol.cleanup_path())
    }

    pub fn transcription_url(&self) -> Option<String> {
        self.protocol.transcription_path().map(|p| self.url(p))
    }

    pub fn models_url(&self) -> String {
        self.url("models")
    }

    /// Overrides with protected keys removed. `normalize` already rejects
    /// them; this is the second lock on the request path.
    pub fn safe_overrides(&self) -> Option<Map<String, Value>> {
        self.body_overrides.as_ref().map(|map| {
            map.iter()
                .filter(|(k, _)| !PROTECTED_BODY_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }
}

fn clean_models(models: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        if model.chars().count() > MAX_MODEL_CHARS || model.chars().any(|c| c.is_control()) {
            return Err("A model id is too long or contains control characters.".into());
        }
        if !out.iter().any(|m| m == model) {
            out.push(model.to_string());
        }
    }
    if out.len() > MAX_MODELS_PER_TASK {
        return Err(format!("At most {MAX_MODELS_PER_TASK} models per task."));
    }
    Ok(out)
}

fn checked_header_name(name: &str) -> Result<String, String> {
    if name.len() > 128 {
        return Err("Header names are limited to 128 bytes.".into());
    }
    let parsed = reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| format!("\"{name}\" is not a valid header name."))?;
    let lower = parsed.as_str().to_string();
    if FORBIDDEN_HEADERS.contains(&lower.as_str()) {
        return Err(format!("The {lower} header can't be set."));
    }
    Ok(lower)
}

/// Whether plain `http://` is acceptable: only for machines the user controls
/// (Ollama, LM Studio, vLLM on the same box or LAN).
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Validates and canonicalizes a base URL: http(s) only, no credentials, query
/// or fragment, https unless the host is local, trailing slash removed.
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() > 2048 {
        return Err("Base URLs are limited to 2048 bytes.".into());
    }
    if raw.is_empty() {
        return Err("Enter the provider's base URL.".into());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| {
        "That doesn't look like a URL. Example: https://api.example.com/v1".to_string()
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| "The URL needs a host name.".to_string())?
        .to_string();
    match url.scheme() {
        "https" => {}
        "http" if is_local_host(&host) => {}
        "http" => {
            return Err(
                "Use https. Plain http is only allowed for localhost and private-network hosts."
                    .into(),
            )
        }
        _ => return Err("Only http and https URLs are supported.".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Don't put credentials in the URL; use the API key field.".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Remove the ?query or #fragment from the URL.".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Parses the stored list. A malformed entry is dropped rather than failing
/// the whole app, since this runs on every dictation.
pub fn parse_stored(value: Option<&Value>) -> Vec<CustomProvider> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| serde_json::from_value::<CustomProvider>(item.clone()).ok())
        .filter_map(|mut p| p.normalize().ok().map(|_| p))
        .scan(std::collections::HashSet::new(), |seen, p| {
            Some(if seen.insert(p.id.clone()) {
                Some(p)
            } else {
                None
            })
        })
        .flatten()
        .take(MAX_CUSTOM_PROVIDERS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CustomProvider {
        CustomProvider {
            id: new_custom_id(),
            name: "  Together  ".into(),
            protocol: CustomProtocol::Openai,
            base_url: "https://api.together.xyz/v1/".into(),
            requires_key: true,
            supports_transcription: true,
            supports_cleanup: true,
            auth_header: None,
            extra_headers: BTreeMap::new(),
            body_overrides: None,
            transcription_models: vec![" whisper ".into(), "whisper".into(), "".into()],
            cleanup_models: vec!["llama".into()],
        }
    }

    #[test]
    fn id_must_be_canonical_uuid() {
        let id = new_custom_id();
        assert!(is_custom_id(&id));
        assert!(!is_custom_id("custom:../../etc"));
        assert!(!is_custom_id("custom:"));
        assert!(!is_custom_id("groq"));
        assert!(!is_custom_id(&id.to_uppercase()));
        assert!(credential_account(&id)
            .unwrap()
            .starts_with("api_key_custom_"));
        assert!(credential_account("custom:nope").is_none());
    }

    #[test]
    fn normalize_trims_and_dedupes() {
        let mut p = sample();
        p.normalize().unwrap();
        assert_eq!(p.name, "Together");
        assert_eq!(p.base_url, "https://api.together.xyz/v1");
        assert_eq!(p.transcription_models, vec!["whisper"]);
        assert_eq!(
            p.cleanup_url(),
            "https://api.together.xyz/v1/chat/completions"
        );
        assert_eq!(
            p.transcription_url().as_deref(),
            Some("https://api.together.xyz/v1/audio/transcriptions")
        );
    }

    #[test]
    fn base_url_rules() {
        assert!(normalize_base_url("https://api.x.ai/v1").is_ok());
        assert!(normalize_base_url("http://localhost:11434/v1").is_ok());
        assert!(normalize_base_url("http://192.168.1.20:8000/v1").is_ok());
        assert!(normalize_base_url("http://[::1]:8000/v1").is_ok());
        assert!(normalize_base_url("http://api.example.com/v1").is_err());
        assert!(normalize_base_url("ftp://example.com").is_err());
        assert!(normalize_base_url("https://user:pw@example.com/v1").is_err());
        assert!(normalize_base_url("https://example.com/v1?key=abc").is_err());
        assert!(normalize_base_url("https://example.com/v1#x").is_err());
        assert!(normalize_base_url("not a url").is_err());
        assert!(normalize_base_url("").is_err());
    }

    #[test]
    fn anthropic_cannot_transcribe() {
        let mut p = sample();
        p.protocol = CustomProtocol::Anthropic;
        p.normalize().unwrap();
        assert!(!p.supports_transcription);
        assert!(p.transcription_models.is_empty());
        assert!(p.transcription_url().is_none());
        assert!(p.cleanup_url().ends_with("/messages"));

        let mut only_stt = sample();
        only_stt.protocol = CustomProtocol::Anthropic;
        only_stt.supports_cleanup = false;
        assert!(only_stt.normalize().is_err());
    }

    #[test]
    fn needs_at_least_one_capability() {
        let mut p = sample();
        p.supports_cleanup = false;
        p.supports_transcription = false;
        assert!(p.normalize().is_err());
    }

    #[test]
    fn headers_are_checked() {
        let mut p = sample();
        p.extra_headers.insert("X-Org".into(), "acme".into());
        p.normalize().unwrap();
        assert_eq!(
            p.extra_headers.get("x-org").map(String::as_str),
            Some("acme")
        );

        let mut bad = sample();
        bad.extra_headers.insert("Host".into(), "evil".into());
        assert!(bad.normalize().is_err());

        let mut cookie = sample();
        cookie.extra_headers.insert("Cookie".into(), "a=b".into());
        assert!(cookie.normalize().is_err());

        let mut newline = sample();
        newline
            .extra_headers
            .insert("X-A".into(), "a\r\nX-B: b".into());
        assert!(newline.normalize().is_err());

        let mut clash = sample();
        clash.auth_header = Some("api-key".into());
        clash.extra_headers.insert("API-Key".into(), "x".into());
        assert!(clash.normalize().is_err());

        let mut auth = sample();
        auth.auth_header = Some("Content-Length".into());
        assert!(auth.normalize().is_err());
    }

    #[test]
    fn body_overrides_cannot_replace_core_fields() {
        let mut p = sample();
        let mut ok = Map::new();
        ok.insert("temperature".into(), Value::from(0));
        p.body_overrides = Some(ok);
        p.normalize().unwrap();
        assert!(p.safe_overrides().unwrap().contains_key("temperature"));

        for key in ["model", "messages", "system", "max_tokens", "stream"] {
            let mut bad = sample();
            let mut m = Map::new();
            m.insert(key.into(), Value::from("x"));
            bad.body_overrides = Some(m);
            assert!(bad.normalize().is_err(), "{key} should be protected");
        }

        let mut huge = sample();
        let mut m = Map::new();
        m.insert("blob".into(), Value::from("x".repeat(5000)));
        huge.body_overrides = Some(m);
        assert!(huge.normalize().is_err());
    }

    #[test]
    fn safe_overrides_strip_protected_keys_even_if_stored_directly() {
        let mut p = sample();
        let mut m = Map::new();
        m.insert("model".into(), Value::from("evil"));
        m.insert("top_p".into(), Value::from(1));
        p.body_overrides = Some(m);
        let safe = p.safe_overrides().unwrap();
        assert!(!safe.contains_key("model"));
        assert!(safe.contains_key("top_p"));
    }

    #[test]
    fn stored_list_drops_malformed_entries() {
        let good = serde_json::to_value(sample()).unwrap();
        let list = Value::Array(vec![
            good.clone(),
            serde_json::json!({"id": "custom:bad", "name": "x"}),
            serde_json::json!("junk"),
        ]);
        assert_eq!(parse_stored(Some(&list)).len(), 1);
        assert!(parse_stored(None).is_empty());
    }

    #[test]
    fn stored_entries_are_validated_and_duplicate_ids_are_dropped() {
        let p = sample();
        let mut bad = p.clone();
        bad.id = new_custom_id();
        bad.base_url = "http://untrusted.example/v1".into();
        let list = serde_json::json!([p.clone(), p.clone(), bad]);
        assert_eq!(parse_stored(Some(&list)).len(), 1);
        assert!(normalize_list(&serde_json::json!([p.clone(), p])).is_err());
    }

    #[test]
    fn credentials_cannot_be_in_configuration() {
        let mut p = sample();
        p.extra_headers
            .insert("Authorization".into(), "test-placeholder".into());
        assert!(p.normalize().is_err());
        let mut value = serde_json::to_value(sample()).unwrap();
        value["api_key"] = Value::from("test-placeholder");
        assert!(normalize_list(&serde_json::json!([value])).is_err());
    }

    fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let task = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed().as_secs() < 5 =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(e) => panic!("Test endpoint did not receive a request: {e}"),
                }
            };
            // macOS can leave an accepted stream nonblocking when its listener
            // is nonblocking. The size-cap test writes a body larger than the
            // kernel send buffer, so make the accepted socket blocking before
            // writing and bound the wait explicitly.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, task)
    }

    #[tokio::test]
    async fn sends_custom_openai_cleanup_with_named_auth_and_options() {
        let (url, request) = server(
            "200 OK",
            r#"{"choices":[{"message":{"content":"Hello."}}]}"#,
        );
        let mut p = sample();
        p.base_url = url;
        p.auth_header = Some("x-vendor-key".into());
        p.extra_headers.insert("x-project".into(), "fixture".into());
        p.body_overrides =
            Some(serde_json::from_value(serde_json::json!({"temperature":0})).unwrap());
        p.normalize().unwrap();
        let text = super::super::cleanup::cleanup(
            "hello",
            super::super::Target::Custom(Box::new(p)),
            "test-key",
            "vendor/chat",
            "casual",
            "medium",
            "",
            None,
            None,
            0,
        )
        .await
        .unwrap();
        assert_eq!(text, "Hello.");
        let request = request.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"));
        assert!(request.contains("x-vendor-key: test-key"));
        assert!(request.contains("x-project: fixture"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["model"], "vendor/chat");
        assert_eq!(body["temperature"], 0);
        assert!(body["messages"].is_array());
    }

    #[tokio::test]
    async fn sends_custom_anthropic_cleanup() {
        let (url, request) = server("200 OK", r#"{"content":[{"type":"text","text":"Hello."}]}"#);
        let mut p = sample();
        p.base_url = url;
        p.protocol = CustomProtocol::Anthropic;
        p.normalize().unwrap();
        assert_eq!(
            super::super::cleanup::cleanup(
                "hello",
                super::super::Target::Custom(Box::new(p)),
                "test-key",
                "vendor/model",
                "casual",
                "medium",
                "",
                None,
                None,
                0
            )
            .await
            .unwrap(),
            "Hello."
        );
        let request = request.join().unwrap();
        assert!(request.starts_with("POST /v1/messages"));
        assert!(request.contains("x-api-key: test-key"));
        assert!(request.contains("anthropic-version: 2023-06-01"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body["system"].is_string());
        assert!(body["max_tokens"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn custom_transcription_sends_each_protocols_multipart_shape_without_auth() {
        for protocol in [CustomProtocol::Openai, CustomProtocol::Xai] {
            let (url, request) = server("200 OK", r#"{"text":"Hello."}"#);
            let mut p = sample();
            p.base_url = url;
            p.protocol = protocol;
            p.requires_key = false;
            p.normalize().unwrap();
            assert_eq!(
                super::super::transcription::transcribe(
                    bytes::Bytes::from_static(b"synthetic audio"),
                    super::super::Target::Custom(Box::new(p)),
                    "",
                    "en",
                    "speech/model",
                    0
                )
                .await
                .unwrap(),
                "Hello."
            );
            let request = request.join().unwrap();
            assert!(request.starts_with(if protocol == CustomProtocol::Xai {
                "POST /v1/stt"
            } else {
                "POST /v1/audio/transcriptions"
            }));
            assert!(!request.to_lowercase().contains("authorization:"));
            assert!(request.contains("synthetic audio"));
            assert!(request.contains("speech/model"));
        }
    }

    #[tokio::test]
    async fn custom_errors_do_not_include_echoed_content() {
        let (url, request) = server("400 Bad Request", "echoed-private-content");
        let mut p = sample();
        p.base_url = url;
        p.normalize().unwrap();
        let error = super::super::cleanup::cleanup(
            "hello",
            super::super::Target::Custom(Box::new(p)),
            "test-key",
            "vendor/model",
            "casual",
            "medium",
            "",
            None,
            None,
            0,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("status=400"));
        assert!(!format!("{error:?}").contains("echoed-private-content"));
        request.join().unwrap();
    }

    #[tokio::test]
    async fn custom_redirect_is_rejected() {
        let (url, request) = server("302 Found\r\nLocation: http://127.0.0.1:1", "{}");
        let mut p = sample();
        p.base_url = url;
        p.normalize().unwrap();
        let error = super::super::cleanup::cleanup(
            "hello",
            super::super::Target::Custom(Box::new(p)),
            "test-key",
            "vendor/model",
            "casual",
            "medium",
            "",
            None,
            None,
            0,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("status=302"));
        request.join().unwrap();
    }

    #[tokio::test]
    async fn custom_response_size_is_capped() {
        let body = "x".repeat(super::super::MAX_CUSTOM_RESPONSE_BYTES + 1);
        let (url, request) = server("200 OK", &body);
        let wire = sample().wire();
        let response = wire.client().get(url).send().await.unwrap();
        let error = wire.json::<Value>(response).await.unwrap_err();
        assert!(error.to_string().contains("larger than 4 MB"));
        request.join().unwrap();
    }
}
