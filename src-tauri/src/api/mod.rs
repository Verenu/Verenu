pub mod auto_learn;
pub mod base64_audio;
pub mod cleanup;
pub mod client;
pub mod gemini_types;
pub mod openrouter;
pub mod prompts;
pub mod service_status;
pub mod transcription;
pub mod updater;

#[cfg(test)]
mod live_regression_tests;

const AUTH_401_PREFIX: &str = "AUTH_401";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderId {
    Groq,
    OpenAI,
    Google,
    AssemblyAi,
    OpenRouter,
    Xai,
    Local,
}

/// How a provider turns a transcript into cleaned text. Identity (which key,
/// which label) stays on `ProviderId`; the wire format is a separate axis so a
/// provider can mix adapters (xAI chats like OpenAI but transcribes its own way).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupAdapter {
    OpenAiChat { url: &'static str },
    Gemini,
    Unsupported,
}

/// How a provider turns audio into text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptionAdapter {
    /// OpenAI-style multipart upload with a bearer key.
    OpenAiMultipart {
        url: &'static str,
    },
    /// JSON body with base64 `input_audio`.
    OpenRouterJson {
        url: &'static str,
    },
    /// xAI's multipart `/v1/stt`, where `file` must be the last field.
    XaiStt {
        url: &'static str,
    },
    Gemini,
    AssemblyAi,
    Unsupported,
}

impl ProviderId {
    pub fn from_str(value: &str) -> Self {
        match value {
            "openai" => Self::OpenAI,
            "google" => Self::Google,
            "assemblyai" => Self::AssemblyAi,
            "openrouter" => Self::OpenRouter,
            "xai" => Self::Xai,
            "local" => Self::Local,
            _ => Self::Groq,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Groq => "groq",
            Self::OpenAI => "openai",
            Self::Google => "google",
            Self::AssemblyAi => "assemblyai",
            Self::OpenRouter => "openrouter",
            Self::Xai => "xai",
            Self::Local => "local",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Groq => "Groq",
            Self::OpenAI => "OpenAI",
            Self::Google => "Google",
            Self::AssemblyAi => "AssemblyAI",
            Self::OpenRouter => "OpenRouter",
            Self::Xai => "xAI",
            Self::Local => "Local",
        }
    }

    pub fn cleanup_adapter(self) -> CleanupAdapter {
        match self {
            Self::Groq => CleanupAdapter::OpenAiChat {
                url: "https://api.groq.com/openai/v1/chat/completions",
            },
            Self::OpenAI => CleanupAdapter::OpenAiChat {
                url: "https://api.openai.com/v1/chat/completions",
            },
            Self::OpenRouter => CleanupAdapter::OpenAiChat {
                url: "https://openrouter.ai/api/v1/chat/completions",
            },
            Self::Xai => CleanupAdapter::OpenAiChat {
                url: "https://api.x.ai/v1/chat/completions",
            },
            Self::Google => CleanupAdapter::Gemini,
            Self::AssemblyAi | Self::Local => CleanupAdapter::Unsupported,
        }
    }

    pub fn transcription_adapter(self) -> TranscriptionAdapter {
        match self {
            Self::Groq => TranscriptionAdapter::OpenAiMultipart {
                url: "https://api.groq.com/openai/v1/audio/transcriptions",
            },
            Self::OpenAI => TranscriptionAdapter::OpenAiMultipart {
                url: "https://api.openai.com/v1/audio/transcriptions",
            },
            Self::OpenRouter => TranscriptionAdapter::OpenRouterJson {
                url: "https://openrouter.ai/api/v1/audio/transcriptions",
            },
            Self::Xai => TranscriptionAdapter::XaiStt {
                url: "https://api.x.ai/v1/stt",
            },
            Self::Google => TranscriptionAdapter::Gemini,
            Self::AssemblyAi => TranscriptionAdapter::AssemblyAi,
            Self::Local => TranscriptionAdapter::Unsupported,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthErrorCategory {
    InvalidOrRevokedKey,
    ScopeOrAccountRestriction,
    UnknownUnauthorized,
}

impl AuthErrorCategory {
    fn as_wire_value(self) -> &'static str {
        match self {
            Self::InvalidOrRevokedKey => "invalid_or_revoked_key",
            Self::ScopeOrAccountRestriction => "scope_or_account_restriction",
            Self::UnknownUnauthorized => "unknown_unauthorized",
        }
    }

    fn from_wire_value(v: &str) -> Option<Self> {
        match v {
            "invalid_or_revoked_key" => Some(Self::InvalidOrRevokedKey),
            "scope_or_account_restriction" => Some(Self::ScopeOrAccountRestriction),
            "unknown_unauthorized" => Some(Self::UnknownUnauthorized),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedAuth401Error {
    pub provider: String,
    pub category: AuthErrorCategory,
    pub model: Option<String>,
    pub request_id: Option<String>,
}

pub fn quota_bail(provider: &str) -> anyhow::Error {
    anyhow::anyhow!("QUOTA_EXCEEDED: {provider} quota reached")
}

pub fn is_quota_error(e: &anyhow::Error) -> bool {
    e.to_string().starts_with("QUOTA_EXCEEDED:")
}

pub fn validate_model_for_url(model: &str) -> anyhow::Result<()> {
    if model.is_empty() {
        anyhow::bail!("Invalid model identifier (empty)");
    }
    if model.contains("..") {
        anyhow::bail!("Invalid model identifier (path traversal): {model}");
    }
    if model
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | '/'))
    {
        Ok(())
    } else {
        anyhow::bail!("Invalid model identifier for API URL: {model}")
    }
}

pub fn sanitize_error_body_preview(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 180 {
        format!("{}...", compact.chars().take(177).collect::<String>())
    } else {
        compact
    }
}

enum ProviderHttpError {
    Quota(anyhow::Error),
    Auth {
        error: anyhow::Error,
        status: reqwest::StatusCode,
        request_id: String,
        preview: String,
    },
    NonSuccess {
        source: reqwest::Error,
        status: reqwest::StatusCode,
        request_id: String,
        preview: String,
    },
}

fn response_request_id(resp: &reqwest::Response) -> String {
    resp.headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

async fn ensure_provider_success(
    resp: reqwest::Response,
    quota_label: &str,
    auth: Option<(&str, &str)>,
) -> Result<reqwest::Response, ProviderHttpError> {
    let status = resp.status();
    let request_id = response_request_id(&resp);

    if status.as_u16() == 429 {
        return Err(ProviderHttpError::Quota(quota_bail(quota_label)));
    }

    if matches!(status.as_u16(), 401 | 403) {
        if let Some((provider, model)) = auth {
            let body = resp.text().await.unwrap_or_default();
            let preview = sanitize_error_body_preview(&body);
            let category = classify_unauthorized_body(&body);
            let error = auth_status_error(provider, model, &request_id, status.as_u16(), category);
            return Err(ProviderHttpError::Auth {
                error,
                status,
                request_id,
                preview,
            });
        }
    }

    if let Err(source) = resp.error_for_status_ref() {
        let body = resp.text().await.unwrap_or_default();
        let preview = sanitize_error_body_preview(&body);
        return Err(ProviderHttpError::NonSuccess {
            source,
            status,
            request_id,
            preview,
        });
    }

    Ok(resp)
}

pub fn classify_unauthorized_body(body: &str) -> AuthErrorCategory {
    let lower = body.to_lowercase();

    if [
        "forbidden",
        "permission",
        "not allowed",
        "access denied",
        "scope",
        "organization",
        "role",
        "project",
        "team owner",
        "developer role",
        "insufficient",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return AuthErrorCategory::ScopeOrAccountRestriction;
    }

    if [
        "invalid api key",
        "incorrect api key",
        "invalid key",
        "revoked",
        "authentication failed",
        "unauthorized",
        "api key is not valid",
        "bad api key",
        "key has expired",
        "expired key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return AuthErrorCategory::InvalidOrRevokedKey;
    }

    AuthErrorCategory::UnknownUnauthorized
}

fn auth_401_user_message(provider: &str, category: AuthErrorCategory) -> String {
    match category {
        AuthErrorCategory::InvalidOrRevokedKey => {
            format!("{provider} API key looks invalid or revoked. Replace it in Settings > API Keys.")
        }
        AuthErrorCategory::ScopeOrAccountRestriction => format!(
            "{provider} rejected this key for account or model access. Check the key's permissions and your provider account's access to the selected model."
        ),
        AuthErrorCategory::UnknownUnauthorized => {
            format!("{provider} rejected authentication. Replace the key in Settings > API Keys and check your provider account's access.")
        }
    }
}

pub fn auth_401_error(
    provider: &str,
    model: &str,
    request_id: &str,
    category: AuthErrorCategory,
) -> anyhow::Error {
    auth_status_error(provider, model, request_id, 401, category)
}

fn auth_status_error(
    provider: &str,
    model: &str,
    request_id: &str,
    status: u16,
    category: AuthErrorCategory,
) -> anyhow::Error {
    let user_msg = auth_401_user_message(provider, category);
    anyhow::anyhow!(
        "{AUTH_401_PREFIX}|provider={provider}|category={}|model={model}|request_id={request_id}|status={status}: {user_msg}",
        category.as_wire_value()
    )
}

pub fn parse_auth_401_error(message: &str) -> Option<ParsedAuth401Error> {
    if !message.starts_with(AUTH_401_PREFIX) {
        return None;
    }
    let meta = message.split(": ").next().unwrap_or(message);
    let mut provider: Option<String> = None;
    let mut category: Option<AuthErrorCategory> = None;
    let mut model: Option<String> = None;
    let mut request_id: Option<String> = None;

    for part in meta.split('|').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "provider" => provider = Some(value.to_string()),
            "category" => category = AuthErrorCategory::from_wire_value(value),
            "model" if !value.is_empty() && value != "-" => model = Some(value.to_string()),
            "request_id" if !value.is_empty() && value != "-" => {
                request_id = Some(value.to_string())
            }
            _ => {}
        }
    }

    Some(ParsedAuth401Error {
        provider: provider?,
        category: category?,
        model,
        request_id,
    })
}

pub fn auth_401_display_message(parsed: &ParsedAuth401Error) -> String {
    auth_401_user_message(&parsed.provider, parsed.category)
}

pub fn is_retryable_provider_error(e: &anyhow::Error) -> bool {
    if is_quota_error(e) {
        return true;
    }

    for cause in e.chain() {
        if let Some(reqwest_err) = cause.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_timeout() || reqwest_err.is_connect() || reqwest_err.is_request() {
                return true;
            }
            if let Some(status) = reqwest_err.status() {
                return status.as_u16() == 408
                    || status.as_u16() == 429
                    || status.is_server_error();
            }
        }
    }

    let msg = e.to_string().to_lowercase();
    if let Some(status) = extract_http_status_code(&msg) {
        if status == 408 || status == 429 || (500..=599).contains(&status) {
            return true;
        }
    }
    msg.contains("timeout")
        || msg.contains("timed out")
        || msg.contains("connection")
        || msg.contains("temporarily unavailable")
        || msg.contains("overloaded")
        || msg.contains("rate limit")
        || msg.contains(" 502")
        || msg.contains(" 503")
        || msg.contains(" 504")
}

/// Converts an error into a safe, actionable user-facing message. Provider
/// metadata and response bodies must never be shown directly in the UI.
pub fn user_facing_error(e: &anyhow::Error) -> String {
    // The outer context can hide the actual network or HTTP failure.
    if parse_auth_401_error(&e.to_string()).is_some() || is_quota_error(e) {
        return user_facing_message(&e.to_string());
    }
    for cause in e.chain() {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = error.status() {
                return provider_status_message(status.as_u16());
            }
            if error.is_timeout() {
                return "The provider took too long to respond. Check your connection, then try again or choose another provider.".to_string();
            }
            if error.is_connect() {
                return "Verenu could not connect to the provider. Check your internet connection and any VPN or firewall, then try again.".to_string();
            }
            if error.is_decode() {
                return "The provider returned a response Verenu could not read. Try again or choose another model in Settings > Models.".to_string();
            }
        }
    }
    user_facing_message(&e.to_string())
}

fn provider_status_message(status: u16) -> String {
    match status {
        401 | 403 => "The provider rejected access. Check your API key and account access in Settings > API Keys.".to_string(),
        404 => "The requested model or download was not found. Refresh the model list in Settings > Models and choose an available model.".to_string(),
        408 | 504 => "The provider took too long to respond. Check your connection, then try again or choose another provider.".to_string(),
        413 => "The provider could not accept a recording this large. Try a shorter dictation or choose another transcription provider.".to_string(),
        429 => "The provider's request limit was reached. Wait for it to reset, check your provider plan, or choose another provider.".to_string(),
        500..=599 => "The provider is temporarily unavailable. Wait a moment, then try again or choose another provider.".to_string(),
        _ => format!("The provider rejected the request (HTTP {status}). Check the selected model and language in Settings > Models, then try again."),
    }
}

/// String-based sibling for call sites that already hold an error message.
pub fn user_facing_message(msg: &str) -> String {
    if let Some(parsed) = parse_auth_401_error(msg) {
        return auth_401_display_message(&parsed);
    }
    if msg.starts_with("QUOTA_EXCEEDED:") {
        let provider = msg
            .strip_prefix("QUOTA_EXCEEDED:")
            .unwrap_or("")
            .split_whitespace()
            .next()
            .unwrap_or("The provider");
        return format!(
            "{provider} request limit reached. Wait for it to reset, check your provider plan, or choose another provider."
        );
    }
    let metadata = msg.split("body_preview=").next().unwrap_or(msg);
    if let Some(status) =
        extract_http_status_code(metadata).filter(|status| (400..600).contains(status))
    {
        return provider_status_message(status);
    }
    if msg.contains("body_preview=") || msg.contains("request_id=") {
        return "The provider did not return a usable result. Try again or choose another model in Settings > Models.".to_string();
    }
    truncate_display(msg)
}

fn truncate_display(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 600 {
        format!("{}…", s.chars().take(597).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Whether an error indicates the request never reached a reachable server.
pub fn is_connectivity_error(e: &anyhow::Error) -> bool {
    for cause in e.chain() {
        if let Some(reqwest_err) = cause.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_connect() || reqwest_err.is_request() {
                return true;
            }
        }
    }

    let msg = e.to_string().to_lowercase();
    msg.contains("error sending request")
        || msg.contains("dns error")
        || msg.contains("connection reset")
        || msg.contains("connection refused")
        || msg.contains("network is unreachable")
        || msg.contains("failed to resolve")
}

fn extract_http_status_code(msg: &str) -> Option<u16> {
    for marker in ["status=", "status:", "status ", "HTTP "] {
        if let Some(idx) = msg.find(marker) {
            let digits: String = msg[idx + marker.len()..]
                .chars()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.len() == 3 {
                if let Ok(status) = digits.parse::<u16>() {
                    return Some(status);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        auth_401_display_message, classify_unauthorized_body, parse_auth_401_error,
        sanitize_error_body_preview, AuthErrorCategory, ParsedAuth401Error,
    };

    #[test]
    fn user_errors_explain_http_failures_without_response_data() {
        for (status, guidance) in [
            (401, "API Keys"),
            (403, "account access"),
            (404, "not found"),
            (408, "too long"),
            (413, "shorter dictation"),
            (429, "request limit"),
            (500, "temporarily unavailable"),
            (503, "temporarily unavailable"),
            (504, "too long"),
        ] {
            let message = super::user_facing_message(&format!(
                "provider status={status} request_id=fixture-id body_preview=private response data"
            ));
            assert!(message.contains(guidance), "status={status}");
            assert!(!message.contains("fixture-id"));
            assert!(!message.contains("private response"));
        }
    }

    #[test]
    fn user_errors_do_not_infer_status_from_provider_body() {
        let message =
            super::user_facing_message("provider request_id=fixture-id body_preview=status=401");
        assert!(message.contains("usable result"));
        assert!(!message.contains("401"));
    }

    #[test]
    fn user_errors_preserve_complete_recovery_instructions() {
        let message = "This backup contains a newer context format that this version cannot read. Update Verenu on this device, then select the same backup and import it again.";
        assert_eq!(super::user_facing_message(message), message);
    }

    #[test]
    fn classifies_invalid_or_revoked_key_signals() {
        let c = classify_unauthorized_body(r#"{"error":{"message":"Invalid API Key"}}"#);
        assert_eq!(c, AuthErrorCategory::InvalidOrRevokedKey);
    }

    #[test]
    fn classifies_scope_and_role_signals() {
        let c = classify_unauthorized_body(
            r#"{"error":{"message":"Only team owners or users with the developer role may create or manage API keys."}}"#,
        );
        assert_eq!(c, AuthErrorCategory::ScopeOrAccountRestriction);
    }

    #[test]
    fn parses_auth_401_metadata() {
        let parsed = parse_auth_401_error(
            "AUTH_401|provider=Groq|category=invalid_or_revoked_key|model=whisper-large-v3-turbo|request_id=req_123|status=401: Groq API key looks invalid or revoked. Re-enter it in Settings.",
        )
        .expect("parse");
        assert_eq!(parsed.provider, "Groq");
        assert_eq!(parsed.category, AuthErrorCategory::InvalidOrRevokedKey);
        assert_eq!(parsed.model.as_deref(), Some("whisper-large-v3-turbo"));
        assert_eq!(parsed.request_id.as_deref(), Some("req_123"));
    }

    #[test]
    fn auth_display_message_is_specific() {
        let msg = auth_401_display_message(&ParsedAuth401Error {
            provider: "Groq".to_string(),
            category: AuthErrorCategory::InvalidOrRevokedKey,
            model: None,
            request_id: None,
        });
        assert!(msg.to_lowercase().contains("invalid or revoked"));
    }

    #[test]
    fn auth_error_can_encode_forbidden_status() {
        let err = super::auth_status_error(
            "Google",
            "gemini-3.5-flash",
            "req_403",
            403,
            AuthErrorCategory::ScopeOrAccountRestriction,
        );
        let msg = err.to_string();
        assert!(msg.starts_with("AUTH_401|provider=Google"));
        assert!(msg.contains("status=403"));
    }

    #[test]
    fn error_preview_is_single_line_and_truncated() {
        let source = "line one\nline two\tline three";
        let preview = sanitize_error_body_preview(source);
        assert!(!preview.contains('\n'));
        assert!(!preview.contains('\t'));
        assert!(preview.contains("line one line two line three"));
    }
}
