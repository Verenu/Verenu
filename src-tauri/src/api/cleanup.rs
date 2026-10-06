use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::gemini_types::{GeminiGenerateReq, GeminiReqContent, GeminiReqPart};
use super::prompts::{cleanup_max_output_tokens, gemini_generation_config};
use super::{CleanupAdapter, ProviderId, Target, Wire};

// Cleanup should be fast enough to run inline with dictation delivery. Keep
// this shorter than the shared client timeout so a stalled provider can fall
// through to the configured cleanup fallback instead of leaving the pill in
// processing for two minutes.
const CLEANUP_REQUEST_TIMEOUT_SECS: u64 = 45;

#[allow(clippy::too_many_arguments)]
pub async fn cleanup(
    text: &str,
    provider: impl Into<Target>,
    api_key: &str,
    model: &str,
    profile: &str,
    intensity: &str,
    snippet_instructions: &str,
    app_context: Option<&str>,
    custom_template: Option<&str>,
    gen: u64,
) -> Result<String> {
    cleanup_with_alternate(
        text,
        provider,
        api_key,
        model,
        profile,
        intensity,
        snippet_instructions,
        app_context,
        custom_template,
        None,
        gen,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn cleanup_with_alternate(
    text: &str,
    provider: impl Into<Target>,
    api_key: &str,
    model: &str,
    profile: &str,
    intensity: &str,
    snippet_instructions: &str,
    app_context: Option<&str>,
    custom_template: Option<&str>,
    alternate_transcript: Option<&str>,
    gen: u64,
) -> Result<String> {
    cleanup_with_alternate_and_evidence(
        text,
        provider,
        api_key,
        model,
        profile,
        intensity,
        snippet_instructions,
        "",
        app_context,
        custom_template,
        alternate_transcript,
        gen,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn cleanup_with_alternate_and_evidence(
    text: &str,
    provider: impl Into<Target>,
    api_key: &str,
    model: &str,
    profile: &str,
    intensity: &str,
    user_overrides: &str,
    evidence: &str,
    app_context: Option<&str>,
    custom_template: Option<&str>,
    alternate_transcript: Option<&str>,
    gen: u64,
) -> Result<String> {
    let target: Target = provider.into();
    if let Target::Builtin(provider) = &target {
        if provider.cleanup_adapter() == CleanupAdapter::Unsupported {
            anyhow::bail!(
                "{} provides no cleanup endpoint; choose a cleanup provider",
                provider.label()
            )
        }
    }
    if intensity == "none" && alternate_transcript.is_none() {
        // The pipeline normally bypasses this function for Off. Keep the API
        // boundary safe too: only dual-transcript reconciliation is a valid
        // model operation at this intensity.
        return Ok(text.to_owned());
    }
    // Prompt selection and the reasoning policy key off the provider family.
    // A custom endpoint never borrows a built-in's behavior by sharing its
    // display name, so it is always "custom" here.
    let provider_id = match &target {
        Target::Builtin(provider) => provider.as_str(),
        Target::Custom(_) => "custom",
    };
    #[cfg(any(test, debug_assertions))]
    if let Some(result) =
        crate::testing::resolve_provider_fixture("cleanup", target.id_str(), model)
    {
        return result;
    }

    let prompt = super::prompts::get_cleanup_prompt_with_alternate_and_evidence(
        provider_id,
        model,
        profile,
        intensity,
        user_overrides,
        evidence,
        app_context,
        text,
        custom_template,
        alternate_transcript,
    );
    let max_output_tokens = if intensity == "none" {
        alternate_transcript
            .map(|alternate| super::prompts::fusion_max_output_tokens(text, alternate))
            .unwrap_or_else(|| cleanup_max_output_tokens(intensity, text))
    } else {
        cleanup_max_output_tokens(intensity, text)
    };
    log::debug!(
        "cleanup: start gen={} provider={} model={} profile={} intensity={} input_chars={} prompt_chars={} max_output_tokens={} snippet_rule_lines={} app_context={} custom_template={}",
        gen,
        target.id_str(),
        model,
        profile,
        intensity,
        text.chars().count(),
        prompt.chars().count(),
        max_output_tokens,
        user_overrides.lines().filter(|l| !l.trim().is_empty()).count(),
        app_context.is_some(),
        custom_template.is_some()
    );
    if crate::system::logger::is_verbose() && !user_overrides.is_empty() {
        log::debug!(
            "cleanup: snippet_rules_meta lines={} chars={}",
            user_overrides
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            user_overrides.chars().count()
        );
    }
    let request = async {
        let provider = match &target {
            Target::Builtin(provider) => *provider,
            Target::Custom(custom) => {
                return custom_cleanup(
                    text,
                    api_key,
                    custom,
                    model,
                    &prompt,
                    max_output_tokens,
                    alternate_transcript,
                    gen,
                )
                .await;
            }
        };
        match provider.cleanup_adapter() {
            CleanupAdapter::OpenAiChat { url } => {
                openai_compat(
                    text,
                    api_key,
                    url,
                    &Wire::default(),
                    provider.label(),
                    provider.label(),
                    model,
                    &prompt,
                    max_output_tokens,
                    alternate_transcript,
                    None,
                    gen,
                )
                .await
            }
            CleanupAdapter::Gemini => {
                google_cleanup(
                    text,
                    api_key,
                    &prompt,
                    model,
                    max_output_tokens,
                    alternate_transcript,
                    gen,
                )
                .await
            }
            CleanupAdapter::Unsupported => anyhow::bail!(
                "{} provides no cleanup endpoint; choose a cleanup provider",
                provider.label()
            ),
        }
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(CLEANUP_REQUEST_TIMEOUT_SECS),
        request,
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            log::warn!(
                "cleanup: request timeout gen={} provider={} model={} timeout_secs={}",
                gen,
                target.label(),
                model,
                CLEANUP_REQUEST_TIMEOUT_SECS
            );
            Err(anyhow::anyhow!(
                "Cleanup API timeout provider={} model={} timeout_secs={}",
                target.label(),
                model,
                CLEANUP_REQUEST_TIMEOUT_SECS
            ))
        }
    }
}

#[derive(Serialize)]
struct ChatReq {
    model: String,
    messages: Vec<Msg>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'static str>,
}

#[derive(Serialize)]
struct Msg {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: MsgResp,
}

#[derive(Deserialize)]
struct MsgResp {
    content: String,
}

#[allow(clippy::too_many_arguments)]
async fn openai_compat(
    text: &str,
    api_key: &str,
    url: &str,
    wire: &Wire,
    provider_label: &str,
    policy_key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    alternate_transcript: Option<&str>,
    overrides: Option<&serde_json::Map<String, serde_json::Value>>,
    gen: u64,
) -> Result<String> {
    if policy_key != "custom" {
        ensure_openai_compat_reasoning_policy(policy_key, model)?;
    }
    let request_body = build_openai_compat_request_with_alternate(
        text,
        model,
        prompt,
        max_tokens,
        alternate_transcript,
        policy_key,
    );
    let request = wire.apply(wire.client().post(url), api_key);
    let request = if let Some(overrides) = overrides {
        let mut body = serde_json::to_value(&request_body)?;
        merge_overrides(&mut body, Some(overrides));
        request.json(&body)
    } else {
        request.json(&request_body)
    };

    log::debug!(
        "cleanup: openai_compat request gen={} provider={} model={} url={} input_chars={} prompt_chars={}",
        gen,
        provider_label,
        model,
        url,
        text.chars().count(),
        prompt.chars().count()
    );
    let request_started = std::time::Instant::now();
    let resp = request.send().await?;
    let status = resp.status();
    let request_id = super::response_request_id(&resp);
    log::debug!(
        "cleanup: openai_compat response gen={} provider={} status={} request_id={} latency_ms={}",
        gen,
        provider_label,
        status,
        request_id,
        request_started.elapsed().as_millis()
    );

    let resp = wire.check_status(resp, provider_label, model)?;
    let resp = checked_cleanup_response(resp, provider_label, model, gen).await?;

    let data: ChatResp = wire.json(resp).await?;
    let output = data
        .choices
        .first()
        .map(|c| c.message.content.trim().to_owned())
        .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;
    log::debug!(
        "cleanup: openai_compat parsed gen={} chars={}",
        gen,
        output.chars().count()
    );
    Ok(output)
}

/// Adds user-supplied body fields. Protected keys were rejected when the
/// provider was saved; `safe_overrides` strips them again before this runs.
fn merge_overrides(
    body: &mut serde_json::Value,
    overrides: Option<&serde_json::Map<String, serde_json::Value>>,
) {
    let (Some(overrides), Some(object)) = (overrides, body.as_object_mut()) else {
        return;
    };
    for (key, value) in overrides {
        object.insert(key.clone(), value.clone());
    }
}

#[allow(clippy::too_many_arguments)]
async fn custom_cleanup(
    text: &str,
    api_key: &str,
    custom: &super::custom::CustomProvider,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    alternate_transcript: Option<&str>,
    gen: u64,
) -> Result<String> {
    use super::custom::CustomProtocol;
    if !custom.supports_cleanup {
        anyhow::bail!("Cleanup is turned off for {}", custom.name);
    }
    let wire = custom.wire();
    let url = custom.cleanup_url();
    let overrides = custom.safe_overrides();
    match custom.protocol {
        CustomProtocol::Anthropic => {
            anthropic_cleanup(
                text,
                api_key,
                &url,
                &wire,
                &custom.name,
                model,
                prompt,
                max_tokens,
                alternate_transcript,
                overrides.as_ref(),
                gen,
            )
            .await
        }
        CustomProtocol::Openai | CustomProtocol::Xai => {
            openai_compat(
                text,
                api_key,
                &url,
                &wire,
                &custom.name,
                "custom",
                model,
                prompt,
                max_tokens,
                alternate_transcript,
                overrides.as_ref(),
                gen,
            )
            .await
        }
    }
}

fn build_anthropic_request(
    text: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    alternate_transcript: Option<&str>,
) -> serde_json::Value {
    // Mark standing instructions, never the changing vocabulary tail or
    // transcript. Providers silently skip prefixes below a model's minimum
    // cache size; keep prompts compact instead of padding solely for caching.
    let (standing, evidence, cacheable) = split_cached_evidence(prompt);
    if !cacheable {
        // If a custom prompt contains a malformed evidence boundary, fail
        // closed: keep the request intact but do not cache possibly dynamic
        // content as standing instructions.
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{
                "role": "user",
                "content": format_transcript_input(text, alternate_transcript),
            }],
        });
        if !prompt.trim().is_empty() {
            body["system"] = serde_json::json!([{"type": "text", "text": prompt}]);
        }
        return body;
    }
    // Two boundaries preserve the shared contract across preset changes,
    // then reuse the complete configured prefix on repeated dictations.
    let mut system = Vec::new();
    match standing.find("<cleanup_settings>") {
        Some(settings_start) => {
            // Split on the structural tag instead of the exact newline
            // spelling so templates saved with CRLF retain the shared cache
            // boundary across cleanup preset changes.
            let shared = standing[..settings_start].trim_end();
            if !shared.trim().is_empty() {
                system.push(serde_json::json!({"type": "text", "text": shared, "cache_control": {"type": "ephemeral"}}));
            }
            let settings = format!("\n\n{}", &standing[settings_start..]);
            if !settings.trim().is_empty() {
                system.push(serde_json::json!({"type": "text", "text": settings, "cache_control": {"type": "ephemeral"}}));
            }
        }
        None if !standing.trim().is_empty() => {
            system.push(serde_json::json!({"type": "text", "text": standing, "cache_control": {"type": "ephemeral"}}));
        }
        None => {}
    }
    if let Some(evidence) = evidence {
        if !evidence.trim().is_empty() {
            system.push(serde_json::json!({"type": "text", "text": evidence}));
        }
    }
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{
            "role": "user",
            "content": format_transcript_input(text, alternate_transcript),
        }],
    });
    if !system.is_empty() {
        body["system"] = serde_json::Value::Array(system);
    }
    body
}

fn split_cached_evidence(prompt: &str) -> (&str, Option<String>, bool) {
    const OPEN: &str = "<evidence>";
    const CLOSE: &str = "</evidence>";
    let Some(start) = prompt.rfind(OPEN) else {
        return (prompt, None, !prompt.contains(CLOSE));
    };
    let content_start = start + OPEN.len();
    let Some(close_relative) = prompt[content_start..].find(CLOSE) else {
        return (prompt, None, false);
    };
    let close_end = content_start + close_relative + CLOSE.len();
    if !prompt[close_end..].trim().is_empty() {
        return (prompt, None, false);
    }

    let standing = prompt[..start].trim_end();
    let evidence = format!("\n\n{}", &prompt[start..close_end]);
    (standing, Some(evidence), true)
}

/// Joins the text blocks of an Anthropic messages response.
fn parse_anthropic_text(body: &serde_json::Value) -> Option<String> {
    let text: String = body
        .get("content")?
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[allow(clippy::too_many_arguments)]
async fn anthropic_cleanup(
    text: &str,
    api_key: &str,
    url: &str,
    wire: &Wire,
    provider_label: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    alternate_transcript: Option<&str>,
    overrides: Option<&serde_json::Map<String, serde_json::Value>>,
    gen: u64,
) -> Result<String> {
    let mut body = build_anthropic_request(text, model, prompt, max_tokens, alternate_transcript);
    // Other vendors can expose the messages protocol without implementing
    // Anthropic's cache extensions. Preserve their plain system-string shape.
    if !reqwest::Url::parse(url)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some("api.anthropic.com"))
    {
        if prompt.trim().is_empty() {
            if let Some(object) = body.as_object_mut() {
                object.remove("system");
            }
        } else {
            body["system"] = serde_json::Value::String(prompt.to_owned());
        }
    }
    merge_overrides(&mut body, overrides);
    log::debug!(
        "cleanup: anthropic request gen={} provider={} model={} input_chars={}",
        gen,
        provider_label,
        model,
        text.chars().count()
    );
    let resp = wire
        .apply(wire.client().post(url), api_key)
        .json(&body)
        .send()
        .await?;
    let resp = wire.check_status(resp, provider_label, model)?;
    let resp = checked_cleanup_response(resp, provider_label, model, gen).await?;
    let data: serde_json::Value = wire.json(resp).await?;
    parse_anthropic_text(&data)
        .ok_or_else(|| anyhow::anyhow!("No text content in {provider_label} response"))
}

/// Shared status handling for chat-style cleanup responses.
async fn checked_cleanup_response(
    resp: reqwest::Response,
    provider_label: &str,
    model: &str,
    gen: u64,
) -> Result<reqwest::Response> {
    match super::ensure_provider_success(resp, provider_label, Some((provider_label, model))).await
    {
        Ok(resp) => Ok(resp),
        Err(super::ProviderHttpError::Quota(e)) => Err(e),
        Err(super::ProviderHttpError::Auth {
            error,
            status,
            request_id,
            preview,
        }) => {
            log::warn!(
                "cleanup: openai_compat unauthorized gen={} provider={} model={} status={} request_id={} body_preview=\"{}\"",
                gen,
                provider_label,
                model,
                status,
                request_id,
                preview
            );
            Err(error)
        }
        Err(super::ProviderHttpError::NonSuccess {
            source,
            status,
            request_id,
            preview,
        }) => {
            log::warn!(
                "cleanup: openai_compat non_success gen={} provider={} model={} status={} request_id={} body_preview=\"{}\"",
                gen,
                provider_label,
                model,
                status,
                request_id,
                preview
            );
            Err(anyhow::Error::new(source).context(format!(
                "Cleanup API error provider={} model={} status={} request_id={} body_preview={}",
                provider_label, model, status, request_id, preview
            )))
        }
    }
}

fn ensure_openai_compat_reasoning_policy(provider_label: &str, model: &str) -> Result<()> {
    if !openai_compat_model_supports_no_reasoning(provider_label, model) {
        anyhow::bail!(
            "{} model '{model}' cannot satisfy Verenu's dictation reasoning policy; choose Qwen 3.6/3.8, GPT-5.1, or an ordinary non-reasoning model.",
            provider_label
        )
    }
    Ok(())
}

fn is_groq_qwen_no_reasoning_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.starts_with("qwen/qwen3.6-") || model.starts_with("qwen/qwen3.8-")
}

fn is_openai_o_series(model: &str) -> bool {
    let mut chars = model.chars();
    chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn is_openai_gpt_51_no_reasoning_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("gpt-5.1")
}

fn openai_compat_model_supports_no_reasoning(provider_label: &str, model: &str) -> bool {
    let provider = provider_label.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    if model.contains("gpt-oss") {
        return false;
    }
    if provider == "groq" {
        // GPT-OSS and other Qwen 3 variants expose reasoning but not the
        // no-thinking mode used here. Known ordinary Groq models need no
        // reasoning field at all.
        return !model.starts_with("qwen/qwen3") || is_groq_qwen_no_reasoning_model(&model);
    }
    if provider == "openrouter" {
        // Model ids are `vendor/model`. Judge the model by its own name and
        // refuse families that always reason, since OpenRouter has no uniform
        // switch that turns reasoning off.
        let bare = model.rsplit('/').next().unwrap_or(&model);
        return !(model.ends_with(":thinking")
            || bare.contains("-thinking")
            || bare.contains("reasoner")
            || bare.contains("-r1")
            || is_openai_o_series(bare)
            || (bare.starts_with("gpt-5") && !is_openai_gpt_51_no_reasoning_model(bare)));
    }
    if provider == "xai" {
        // Grok 4 and the *-reasoning / mini lines always think; only the
        // explicit non-reasoning variants and older Grok 2/3 chat models fit.
        if model.contains("non-reasoning") {
            return true;
        }
        return !(model.contains("reasoning")
            || model.starts_with("grok-4")
            || model.starts_with("grok-3-mini"));
    }
    if provider == "openai" {
        // OpenAI's o-series and GPT-5 before 5.1 do not support none. GPT-5.1
        // does; all ordinary GPT-4.x chat models are non-reasoning.
        if model.starts_with('o')
            || (model.starts_with("gpt-5") && !is_openai_gpt_51_no_reasoning_model(&model))
        {
            return false;
        }
    }
    true
}

/// Chain-level version of the reasoning-policy check that also understands
/// custom providers. A deleted or cleanup-disabled custom provider is skipped.
/// Custom models are not policed: the user chose the endpoint and the model.
pub fn chain_entry_supports_cleanup(
    customs: &[super::custom::CustomProvider],
    provider: &str,
    model: &str,
) -> bool {
    match Target::resolve(provider, customs) {
        None => false,
        Some(Target::Builtin(provider)) => model_supports_cleanup_reasoning_policy(provider, model),
        Some(Target::Custom(custom)) => custom.supports_cleanup,
    }
}

/// Whether a selected cleanup backend satisfies the dictation reasoning
/// policy. Google Gemini 3.x is the deliberate exception to "off": it is
/// accepted only when the request can carry the supported minimum level.
/// Provider-chain selection uses this so unsupported models are skipped before
/// a request is made.
pub fn model_supports_cleanup_reasoning_policy(provider: ProviderId, model: &str) -> bool {
    match provider {
        ProviderId::Groq => openai_compat_model_supports_no_reasoning("Groq", model),
        ProviderId::Google => super::prompts::gemini_generation_reasoning_supported(model),
        ProviderId::OpenAI => openai_compat_model_supports_no_reasoning("OpenAI", model),
        ProviderId::OpenRouter => openai_compat_model_supports_no_reasoning("OpenRouter", model),
        ProviderId::Xai => openai_compat_model_supports_no_reasoning("xAI", model),
        ProviderId::Local => true,
        // AssemblyAI is transcription-only and has no cleanup endpoint.
        ProviderId::AssemblyAi => false,
    }
}

async fn google_cleanup(
    text: &str,
    api_key: &str,
    prompt: &str,
    model: &str,
    max_output_tokens: u32,
    alternate_transcript: Option<&str>,
    gen: u64,
) -> Result<String> {
    use super::gemini_types::GeminiResp;

    super::prompts::ensure_gemini_generation_model(model)?;

    log::debug!(
        "cleanup: google request gen={} input_chars={} prompt_chars={} max_output_tokens={}",
        gen,
        text.chars().count(),
        prompt.chars().count(),
        max_output_tokens
    );
    let req = build_google_cleanup_request_with_alternate(
        text,
        prompt,
        model,
        max_output_tokens,
        alternate_transcript,
    );

    super::validate_model_for_url(model)?;
    // Pass the key in the `x-goog-api-key` header, never in the URL query string:
    // URLs leak into error messages, proxies, and logs, and the bare `?key=` form
    // would expose the secret (Verenu's top rule is "API keys never hit logs").
    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");

    let request_started = std::time::Instant::now();
    let resp = super::client::get()
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(&req)
        .send()
        .await?;
    let status = resp.status();
    let request_id = super::response_request_id(&resp);
    log::debug!(
        "cleanup: google response gen={} status={} request_id={} latency_ms={}",
        gen,
        status,
        request_id,
        request_started.elapsed().as_millis()
    );

    let resp = match super::ensure_provider_success(resp, "Google", Some(("Google", model))).await {
        Ok(resp) => resp,
        Err(super::ProviderHttpError::Quota(e)) => return Err(e),
        Err(super::ProviderHttpError::Auth {
            error,
            status,
            request_id,
            preview,
        }) => {
            log::warn!(
                "cleanup: google unauthorized gen={} model={} status={} request_id={} body_preview=\"{}\"",
                gen,
                model,
                status,
                request_id,
                preview
            );
            return Err(error);
        }
        Err(super::ProviderHttpError::NonSuccess {
            source,
            status,
            request_id,
            preview,
        }) => {
            log::warn!(
                "cleanup: google non_success gen={} model={} status={} request_id={} body_preview=\"{}\"",
                gen,
                model,
                status,
                request_id,
                preview
            );
            return Err(anyhow::Error::new(source).context(format!(
                "Google Cleanup API error status={} request_id={} body_preview={}",
                status, request_id, preview
            )));
        }
    };

    let data: GeminiResp = resp.json().await?;
    if let Some(candidate) = data.candidates.as_ref().and_then(|c| c.first()) {
        if let Some(reason) = candidate.finish_reason.as_deref() {
            if reason != "STOP" && reason != "MAX_TOKENS" {
                anyhow::bail!("Gemini cleanup finish_reason: {reason}");
            }
            if reason == "MAX_TOKENS" {
                anyhow::bail!(
                    "Gemini cleanup output reached max_output_tokens={max_output_tokens}"
                );
            }
        }
    }
    let output = data
        .candidates
        .unwrap_or_default()
        .into_iter()
        .next()
        .and_then(|c| c.content)
        .and_then(|c| c.parts.into_iter().next())
        .and_then(|p| p.text)
        .map(|t| t.trim().to_owned())
        .ok_or_else(|| anyhow::anyhow!("No candidates or parts in Google response"))?;
    log::debug!(
        "cleanup: google parsed gen={} chars={}",
        gen,
        output.chars().count()
    );
    Ok(output)
}

#[cfg(test)]
fn build_openai_compat_request(text: &str, model: &str, prompt: &str, max_tokens: u32) -> ChatReq {
    build_openai_compat_request_with_alternate(text, model, prompt, max_tokens, None, "OpenAI")
}

fn build_openai_compat_request_with_alternate(
    text: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    alternate_transcript: Option<&str>,
    provider_label: &str,
) -> ChatReq {
    let user_content = format_transcript_input(text, alternate_transcript);
    let lower_model = model.to_ascii_lowercase();
    let is_no_thinking = (provider_label.eq_ignore_ascii_case("Groq")
        && is_groq_qwen_no_reasoning_model(&lower_model))
        || (provider_label.eq_ignore_ascii_case("OpenAI")
            && is_openai_gpt_51_no_reasoning_model(&lower_model));
    ChatReq {
        model: model.to_owned(),
        messages: vec![
            Msg {
                role: "system".into(),
                content: prompt.to_owned(),
            },
            Msg {
                role: "user".into(),
                content: user_content,
            },
        ],
        max_tokens,
        temperature: 0.0,
        reasoning_effort: is_no_thinking.then_some("none"),
        // Only the native OpenAI adapter receives this routing field.
        // Compatible third-party endpoints may reject unknown parameters.
        prompt_cache_key: provider_label
            .eq_ignore_ascii_case("OpenAI")
            .then_some("verenu-cleanup-v2"),
    }
}

#[cfg(test)]
fn build_google_cleanup_request(
    text: &str,
    prompt: &str,
    model: &str,
    max_output_tokens: u32,
) -> GeminiGenerateReq {
    build_google_cleanup_request_with_alternate(text, prompt, model, max_output_tokens, None)
}

fn build_google_cleanup_request_with_alternate(
    text: &str,
    prompt: &str,
    model: &str,
    max_output_tokens: u32,
    alternate_transcript: Option<&str>,
) -> GeminiGenerateReq {
    let input = format_transcript_input(text, alternate_transcript);
    GeminiGenerateReq {
        contents: vec![GeminiReqContent {
            parts: vec![GeminiReqPart {
                inline_data: None,
                text: Some(input),
            }],
        }],
        system_instruction: GeminiReqContent {
            parts: vec![GeminiReqPart {
                inline_data: None,
                text: Some(prompt.to_owned()),
            }],
        },
        generation_config: gemini_generation_config(model, max_output_tokens),
    }
}

/// Escapes dictation text for embedding inside the `<raw_dictation>` XML tag
/// of prompts. Shared with the pipeline's local-cleanup path.
pub fn escape_transcript_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        let replacement = match byte {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            _ => continue,
        };
        escaped.push_str(&text[start..index]);
        escaped.push_str(replacement);
        start = index + 1;
    }
    escaped.push_str(&text[start..]);
    escaped
}

/// Formats transcript candidates identically for every cleanup provider. The
/// tags are labels for data; the system prompt defines how candidates are
/// reconciled.
pub fn format_transcript_input(primary: &str, alternate: Option<&str>) -> String {
    match alternate {
        Some(alternate) => format!(
            "<primary_transcript>\n{}\n</primary_transcript>\n<alternate_transcript>\n{}\n</alternate_transcript>",
            escape_transcript_xml(primary),
            escape_transcript_xml(alternate),
        ),
        None => format!(
            "<raw_dictation>\n{}\n</raw_dictation>",
            escape_transcript_xml(primary)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_google_cleanup_request, build_google_cleanup_request_with_alternate,
        build_openai_compat_request, build_openai_compat_request_with_alternate,
        ensure_openai_compat_reasoning_policy, model_supports_cleanup_reasoning_policy,
    };

    #[test]
    fn xml_escaping_preserves_unicode_and_escapes_existing_entities_once() {
        for (text, expected) in [
            ("", ""),
            ("plain café 🎙", "plain café 🎙"),
            ("<é> &amp; && >", "&lt;é&gt; &amp;amp; &amp;&amp; &gt;"),
            ("<&><&>", "&lt;&amp;&gt;&lt;&amp;&gt;"),
        ] {
            assert_eq!(super::escape_transcript_xml(text), expected);
        }
    }

    #[tokio::test]
    async fn off_without_alternate_bypasses_the_cleanup_provider() {
        let result = super::cleanup(
            "um keep this raw",
            crate::api::ProviderId::Groq,
            "",
            "openai/gpt-oss-20b",
            "formal",
            "none",
            "",
            None,
            None,
            0,
        )
        .await
        .unwrap();
        assert_eq!(result, "um keep this raw");
    }

    #[test]
    fn openai_compat_request_uses_dynamic_max_tokens() {
        let body = build_openai_compat_request("hello", "gpt-4o-mini", "prompt", 128);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["max_tokens"], 128);
        assert_eq!(json["temperature"], 0.0);
        assert!(json.get("reasoning_effort").is_none());
        assert_eq!(json["messages"][0]["content"], "prompt");
    }

    #[test]
    fn gpt_oss_cleanup_is_rejected_instead_of_using_hidden_reasoning() {
        let body = build_openai_compat_request("hello", "openai/gpt-oss-20b", "prompt", 128);
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("reasoning_effort").is_none());
        assert!(json.get("include_reasoning").is_none());
        assert!(ensure_openai_compat_reasoning_policy("Groq", "openai/gpt-oss-20b").is_err());
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::Groq,
            "openai/gpt-oss-20b"
        ));
    }

    #[test]
    fn qwen_cleanup_uses_non_thinking_mode() {
        let body = build_openai_compat_request_with_alternate(
            "hello",
            "qwen/qwen3.6-27b",
            "prompt",
            128,
            None,
            "Groq",
        );
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["reasoning_effort"], "none");
        assert!(json.get("include_reasoning").is_none());
    }

    #[test]
    fn unsupported_reasoning_families_are_not_advertised_for_cleanup() {
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::Groq,
            "qwen/qwen3-32b"
        ));
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::OpenAI,
            "o3-mini"
        ));
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::OpenAI,
            "gpt-5"
        ));
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::OpenAI,
            "gpt-oss-20b"
        ));
        assert!(model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::OpenAI,
            "gpt-5.1"
        ));
        assert!(!model_supports_cleanup_reasoning_policy(
            crate::api::ProviderId::AssemblyAi,
            "universal-2"
        ));
    }

    #[test]
    fn anthropic_request_puts_the_prompt_in_system() {
        let body = super::build_anthropic_request("hello", "claude-x", "be brief", 256, None);
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"][0]["text"], "be brief");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("hello"));
    }

    #[test]
    fn anthropic_cache_boundary_excludes_changing_evidence_and_transcripts() {
        let first = super::build_anthropic_request(
            "hello",
            "claude-x",
            "Stable instructions\n\n<evidence>Verenu</evidence>",
            256,
            None,
        );
        let second = super::build_anthropic_request(
            "different speech",
            "claude-x",
            "Stable instructions\n\n<evidence>Claude</evidence>",
            256,
            Some("alternate speech"),
        );
        assert_eq!(first["system"][0], second["system"][0]);
        assert!(first["system"][0]["cache_control"].is_object());
        assert!(first["system"][1]["text"]
            .as_str()
            .unwrap()
            .starts_with("\n\n<evidence>"));
        assert!(first["system"][1].get("cache_control").is_none());
        assert_ne!(first["system"][1], second["system"][1]);
        assert_ne!(first["messages"], second["messages"]);
    }

    #[test]
    fn anthropic_evidence_boundary_accepts_whitespace_variants() {
        let prompts = [
            "Shared rules\n\n<cleanup_settings>Light</cleanup_settings>\n\n<evidence>Term</evidence>",
            "Shared rules\n\n<cleanup_settings>Light</cleanup_settings>\n<evidence>Term</evidence>",
            "Shared rules\n\n<cleanup_settings>Light</cleanup_settings>  <evidence>Term</evidence>",
            "Shared rules\r\n\r\n<cleanup_settings>Medium</cleanup_settings>\r\n\r\n<evidence>Term</evidence>",
        ];
        for prompt in prompts {
            let body = super::build_anthropic_request("hello", "claude-x", prompt, 256, None);
            assert_eq!(body["system"][0]["text"], "Shared rules");
            assert!(body["system"][1]["text"]
                .as_str()
                .unwrap()
                .ends_with("</cleanup_settings>"));
            assert!(body["system"][2]["text"]
                .as_str()
                .unwrap()
                .starts_with("\n\n<evidence>"));
            assert!(body["system"][2].get("cache_control").is_none());
        }
    }

    #[test]
    fn malformed_anthropic_evidence_is_never_marked_cacheable() {
        let body = super::build_anthropic_request(
            "hello",
            "claude-x",
            "Stable rules\n<evidence>changing",
            256,
            None,
        );
        assert_eq!(
            body["system"][0]["text"],
            "Stable rules\n<evidence>changing"
        );
        assert!(body["system"][0].get("cache_control").is_none());
    }

    #[test]
    fn anthropic_request_omits_empty_system_blocks() {
        let empty = super::build_anthropic_request("hello", "claude-x", "", 256, None);
        assert!(empty.get("system").is_none());

        let settings = super::build_anthropic_request(
            "hello",
            "claude-x",
            "\n\n<cleanup_settings>Light</cleanup_settings>",
            256,
            None,
        );
        assert_eq!(settings["system"].as_array().unwrap().len(), 1);
        assert!(!settings["system"][0]["text"].as_str().unwrap().is_empty());

        let evidence = super::build_anthropic_request(
            "hello",
            "claude-x",
            "<evidence>Term</evidence>",
            256,
            None,
        );
        assert_eq!(evidence["system"].as_array().unwrap().len(), 1);
        assert!(!evidence["system"][0]["text"].as_str().unwrap().is_empty());
        assert!(evidence["system"][0].get("cache_control").is_none());
    }

    #[test]
    fn cache_routing_key_is_stable_and_only_sent_to_openai() {
        for provider in ["OpenAI", "Groq", "OpenRouter", "xAI", "custom"] {
            let first = super::build_openai_compat_request_with_alternate(
                "hello", "model", "stable", 128, None, provider,
            );
            let second = super::build_openai_compat_request_with_alternate(
                "different",
                "model",
                "stable",
                256,
                Some("alternate"),
                provider,
            );
            let first = serde_json::to_value(first).unwrap();
            let second = serde_json::to_value(second).unwrap();
            assert_eq!(first["prompt_cache_key"], second["prompt_cache_key"]);
            if provider == "OpenAI" {
                assert_eq!(first["prompt_cache_key"], "verenu-cleanup-v2");
            } else {
                assert!(first.get("prompt_cache_key").is_none());
            }
        }
    }

    #[test]
    fn anthropic_shared_cache_boundary_survives_preset_changes() {
        let first = super::build_anthropic_request("hello", "claude-x", "Shared rules\n\n<cleanup_settings>Light</cleanup_settings>\n\n<evidence>Verenu</evidence>", 256, None);
        let second = super::build_anthropic_request("other words", "claude-x", "Shared rules\n\n<cleanup_settings>Strong</cleanup_settings>\n\n<evidence>Claude</evidence>", 256, None);
        assert_eq!(first["system"][0], second["system"][0]);
        assert!(first["system"][0]["cache_control"].is_object());
        assert!(first["system"][1]["cache_control"].is_object());
        assert!(first["system"][2].get("cache_control").is_none());
        assert_ne!(first["system"][1], second["system"][1]);
    }

    #[test]
    fn anthropic_response_joins_text_blocks_only() {
        let body = serde_json::json!({"content": [
            {"type": "thinking", "thinking": "hmm"},
            {"type": "text", "text": "Hello "},
            {"type": "text", "text": "world. "},
        ]});
        assert_eq!(
            super::parse_anthropic_text(&body).as_deref(),
            Some("Hello world.")
        );
        assert!(super::parse_anthropic_text(&serde_json::json!({"content": []})).is_none());
        assert!(super::parse_anthropic_text(&serde_json::json!({})).is_none());
    }

    #[test]
    fn overrides_add_fields_but_a_custom_name_never_borrows_a_builtin_policy() {
        let mut body = serde_json::json!({"model": "m", "temperature": 0.0});
        let mut extra = serde_json::Map::new();
        extra.insert("temperature".into(), serde_json::json!(0.7));
        extra.insert("top_p".into(), serde_json::json!(0.9));
        super::merge_overrides(&mut body, Some(&extra));
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["model"], "m");

        // A provider the user names "Groq" must not trigger Groq's
        // reasoning_effort rewrite: custom requests use the policy key "custom".
        let req = super::build_openai_compat_request_with_alternate(
            "t",
            "qwen/qwen3.8-27b",
            "p",
            10,
            None,
            "custom",
        );
        assert!(req.reasoning_effort.is_none());
    }

    #[test]
    fn openrouter_and_xai_cleanup_skip_always_reasoning_models() {
        use crate::api::ProviderId::{OpenRouter, Xai};
        let ok = |p, m| super::model_supports_cleanup_reasoning_policy(p, m);
        assert!(ok(OpenRouter, "openai/gpt-4o-mini"));
        assert!(ok(OpenRouter, "meta-llama/llama-3.3-70b-instruct"));
        assert!(!ok(OpenRouter, "openai/o3-mini"));
        assert!(!ok(OpenRouter, "openai/gpt-oss-20b"));
        assert!(!ok(OpenRouter, "deepseek/deepseek-r1"));
        assert!(!ok(OpenRouter, "qwen/qwen3-235b-a22b:thinking"));
        assert!(ok(Xai, "grok-4-fast-non-reasoning"));
        assert!(!ok(Xai, "grok-4-fast-reasoning"));
        assert!(!ok(Xai, "grok-3-mini"));
        assert!(!ok(Xai, "grok-4"));
    }

    #[test]
    fn transcription_only_and_cleanup_only_providers_are_distinct() {
        use crate::api::{CleanupAdapter, ProviderId, TranscriptionAdapter};
        assert_eq!(
            ProviderId::AssemblyAi.cleanup_adapter(),
            CleanupAdapter::Unsupported
        );
        assert_ne!(
            ProviderId::OpenRouter.transcription_adapter(),
            TranscriptionAdapter::Unsupported
        );
        assert!(matches!(
            ProviderId::Xai.cleanup_adapter(),
            CleanupAdapter::OpenAiChat { .. }
        ));
        assert!(matches!(
            ProviderId::Xai.transcription_adapter(),
            TranscriptionAdapter::XaiStt { .. }
        ));
    }

    #[test]
    fn openai_gpt_51_uses_its_no_reasoning_mode() {
        let body = build_openai_compat_request_with_alternate(
            "hello", "gpt-5.1", "prompt", 128, None, "OpenAI",
        );
        let json = serde_json::to_value(body).unwrap();
        assert_eq!(json["reasoning_effort"], "none");
    }

    #[test]
    fn openai_compat_request_escapes_raw_transcript_xml() {
        let body = build_openai_compat_request("<tag> & text", "gpt-4o-mini", "prompt", 128);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json["messages"][1]["content"],
            "<raw_dictation>\n&lt;tag&gt; &amp; text\n</raw_dictation>"
        );
    }

    #[test]
    fn google_cleanup_request_includes_gemini_config() {
        let body = build_google_cleanup_request("hello", "prompt", "gemini-2.5-flash-lite", 256);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            0
        );
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 256);
        assert_eq!(json["generationConfig"]["temperature"], 0.0);
    }

    #[test]
    fn gemini_3_5_cleanup_request_uses_minimal_thinking_level() {
        let body = build_google_cleanup_request("hello", "prompt", "gemini-3.5-flash-lite", 256);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "minimal"
        );
        assert!(json["generationConfig"]["thinkingConfig"]
            .get("thinkingBudget")
            .is_none());
        assert!(json["generationConfig"].get("temperature").is_none());
    }

    #[test]
    fn google_cleanup_request_escapes_raw_transcript_xml() {
        let body = build_google_cleanup_request("<tag> & text", "prompt", "gemini-2.5-flash", 128);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json["contents"][0]["parts"][0]["text"],
            "<raw_dictation>\n&lt;tag&gt; &amp; text\n</raw_dictation>"
        );
    }

    #[test]
    fn enhanced_cleanup_requests_keep_candidates_in_user_data() {
        let body = build_openai_compat_request_with_alternate(
            "the issue was clawed",
            "gpt-4o-mini",
            "reconcile",
            128,
            Some("the issue was called"),
            "Groq",
        );
        let json = serde_json::to_value(body).unwrap();
        assert_eq!(json["messages"][0]["content"], "reconcile");
        let user = json["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("<primary_transcript>"));
        assert!(user.contains("<alternate_transcript>"));

        let google = build_google_cleanup_request_with_alternate(
            "primary",
            "reconcile",
            "gemini-2.5-flash",
            128,
            Some("alternate"),
        );
        let google_json = serde_json::to_value(google).unwrap();
        let input = google_json["contents"][0]["parts"][0]["text"]
            .as_str()
            .unwrap();
        assert!(input.contains("<primary_transcript>"));
        assert!(input.contains("<alternate_transcript>"));
    }
}
