//! Cleanup stage: refusal/artifact guards, local cleanup requests, the
//! cleanup-result cache, provider fallback chains, and the
//! cleanup+snippet orchestration entrypoints.

use super::stages_style::ensure_terminal_punctuation;
use super::*;

// Cleanup is an enhancement, not a reason to leave a completed dictation
// blocked behind a provider that has accepted a request but stopped replying.
// Normal cleanup completes in about a second, so retry once quickly and then
// deliver the transcription without cleanup if both attempts stall.
const CLEANUP_FAST_ATTEMPT_TIMEOUT_SECS: u64 = 3;
const CLEANUP_FAST_ATTEMPTS: u8 = 2;
// Bump this whenever cleanup instructions change so previously generated
// output cannot mask the new prompt through the cleanup-result cache.
pub(super) const CLEANUP_PROMPT_VERSION: &str = "dictation-v10";

fn cleanup_soft_timeout_error(provider: &str, model: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "CLEANUP_SOFT_TIMEOUT provider={provider} model={model} timeout_secs={CLEANUP_FAST_ATTEMPT_TIMEOUT_SECS}"
    )
}

fn is_cleanup_soft_timeout(error: &anyhow::Error) -> bool {
    error.to_string().starts_with("CLEANUP_SOFT_TIMEOUT ")
}

/// Runtime safety net for a cleanup result that looks like the model
/// answering/refusing instead of returning cleaned dictation. Differential:
/// only acts if `cleaned` looks like a refusal AND `raw` does not (a real
/// speaker can legitimately say "I cannot...").
///
/// Returns `Some(text)` for a usable cleaned result (safe to cache), or
/// `None` if the retry also looks like a refusal/failed and the caller
/// should skip cleanup entirely and use the pre-cleanup text.
#[allow(clippy::too_many_arguments)]
async fn run_local_cleanup_request(
    app: Option<&AppHandle>,
    model: &str,
    expanded: &str,
    profile: &str,
    intensity: &str,
    extra_rules: &str,
    evidence: &str,
    app_context: Option<&str>,
    custom_template: Option<&str>,
    alternate_transcript: Option<&str>,
) -> anyhow::Result<String> {
    #[cfg(any(test, debug_assertions))]
    if let Some(result) = crate::testing::resolve_provider_fixture("cleanup", "local", model) {
        return result;
    }

    let app = app.ok_or_else(|| anyhow::anyhow!("Local cleanup runtime unavailable"))?;
    let prompt = prompts::get_cleanup_prompt_with_alternate_and_evidence(
        "local",
        model,
        profile,
        intensity,
        extra_rules,
        evidence,
        app_context,
        expanded,
        custom_template,
        alternate_transcript,
    );
    // The runtime is launched with thinking disabled, so this budget is for
    // visible cleanup output rather than hidden reasoning.
    let max_output_tokens = if intensity == "none" {
        alternate_transcript
            .map(|alternate| prompts::fusion_max_output_tokens(expanded, alternate))
            .unwrap_or_else(|| prompts::cleanup_max_output_tokens(intensity, expanded))
    } else {
        prompts::cleanup_max_output_tokens(intensity, expanded)
    };
    let manager = app
        .state::<crate::local_llm::LocalLlmManager>()
        .inner()
        .clone();
    manager
        .cleanup_with_prompt_and_alternate(
            app,
            model,
            expanded,
            &prompt,
            alternate_transcript,
            max_output_tokens,
        )
        .await
}

/// Refusal text ("I am an AI..."), leaked model internals (chat-template
/// control tokens, chain-of-thought preamble), degenerate repetition,
/// fabricated content (output sharing almost no words with what was
/// actually dictated), excessive content loss (a "light"/"none" intensity
/// result missing a large chunk of what was actually dictated), unwanted
/// expansion (a "light"/"none" intensity result padded with extra words
/// built mostly from vocabulary that genuinely appears in the input, so
/// fabrication's word-overlap check doesn't catch it), and perspective flip
/// (the model answers dictation that sounds like it's addressed to someone,
/// swapping every "you" for "I" or vice versa — pronouns are too small a
/// fraction of total words to move the fabrication/length checks) are all
/// "the model didn't return usable cleaned dictation" — none of these are
/// ever safe to inject as if they were the user's speech. `reference` is the
/// text `text` is judged against for the fabrication/length/perspective
/// checks (the actual LLM input); pass the same string for both when there's
/// no meaningful baseline to compare against (e.g. judging the raw dictation
/// on its own, where these checks relative to itself are moot).
fn cleanup_output_is_unusable(intensity: &str, reference: &str, text: &str) -> bool {
    prompts::looks_like_refusal(text)
        || prompts::looks_like_model_artifact_leak(text)
        || prompts::looks_like_degenerate_repetition(text)
        || prompts::looks_like_fabricated_content(reference, text)
        || prompts::looks_like_excessive_content_loss(intensity, reference, text)
        || prompts::looks_like_unwanted_expansion(intensity, reference, text)
        || prompts::looks_like_perspective_flip(reference, text)
}

pub(super) fn cleanup_output_is_unusable_against_candidates(
    intensity: &str,
    primary: &str,
    alternate: Option<&str>,
    text: &str,
) -> bool {
    let intrinsic_failure = prompts::looks_like_refusal(text)
        || prompts::looks_like_model_artifact_leak(text)
        || prompts::looks_like_degenerate_repetition(text);
    if intrinsic_failure {
        return true;
    }

    let primary_failure = cleanup_output_is_unusable(intensity, primary, text);
    match alternate {
        Some(alternate) => {
            // A reconciler is allowed to choose wording that only the
            // alternate candidate supports. Reject it only when it fails
            // against both untrusted candidates.
            primary_failure && cleanup_output_is_unusable(intensity, alternate, text)
        }
        None => primary_failure,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn guard_cleanup_refusal(
    cleaned: String,
    raw: &str,
    expanded: &str,
    provider_id: &str,
    customs: &[crate::api::custom::CustomProvider],
    model: &str,
    key: &str,
    profile: &str,
    intensity: &str,
    extra_rules: &str,
    evidence: &str,
    app_context: Option<&str>,
    app: Option<&AppHandle>,
    alternate_transcript: Option<&str>,
    gen: u64,
) -> Option<String> {
    if !cleanup_output_is_unusable_against_candidates(
        intensity,
        expanded,
        alternate_transcript,
        &cleaned,
    ) || prompts::looks_like_refusal(raw)
    {
        return Some(cleaned);
    }

    log::warn!(
        "pipeline: cleanup output looks like a refusal, leaked model internals, or fabricated content, retrying once with hardened prompt provider={provider_id} model={model}"
    );

    let retried = if provider_id == store::LOCAL {
        run_local_cleanup_request(
            app,
            model,
            expanded,
            profile,
            intensity,
            extra_rules,
            evidence,
            app_context,
            Some(prompts::hardened_retry_template()),
            alternate_transcript,
        )
        .await
    } else {
        let Some(cp) = crate::api::Target::resolve(provider_id, customs) else {
            return Some(cleaned);
        };
        cleanup::cleanup_with_alternate_and_evidence(
            expanded,
            cp,
            key,
            model,
            profile,
            intensity,
            extra_rules,
            evidence,
            app_context,
            Some(prompts::hardened_retry_template()),
            alternate_transcript,
            gen,
        )
        .await
    };

    match retried {
        Ok(retried)
            if !retried.is_empty()
                && (!cleanup_output_is_unusable_against_candidates(
                    intensity,
                    expanded,
                    alternate_transcript,
                    &retried,
                ) || cleanup_output_is_unusable(intensity, raw, raw)) =>
        {
            log::debug!(
                "pipeline: cleanup refusal retry succeeded provider={provider_id} model={model}"
            );
            Some(retried)
        }
        Ok(_) => {
            log::warn!(
                "pipeline: cleanup refusal retry still unusable, falling back to pre-cleanup text provider={provider_id} model={model}"
            );
            None
        }
        Err(e) => {
            log::warn!(
                "pipeline: cleanup refusal retry failed, falling back to pre-cleanup text provider={provider_id} model={model} error={}",
                trim_err(&e.to_string())
            );
            None
        }
    }
}

struct CleanupSuccess {
    cleaned: String,
    provider_id: String,
    model: String,
    key: String,
}

#[allow(clippy::too_many_arguments)]
async fn run_cleanup_provider_chain(
    expanded: &str,
    alternate_transcript: Option<&str>,
    cfg: &store::PipelineConfig,
    profile: &str,
    extra_rules: &str,
    evidence: &str,
    app_context: Option<&str>,
    app: Option<&AppHandle>,
    gen: u64,
    telemetry: Option<&PipelineTelemetry>,
) -> (Option<CleanupSuccess>, Option<anyhow::Error>, bool) {
    let mut last_cleanup_err: Option<anyhow::Error> = None;
    let mut saw_soft_timeout = false;
    let mut offline = crate::system::connectivity::recently_confirmed_offline();
    for (provider_index, (provider_id, model)) in runtime_model_chain(app, cfg, "cleanup")
        .into_iter()
        .enumerate()
    {
        if !candidate_available_offline(&provider_id, offline) {
            continue;
        }
        if provider_index > 0 {
            if let Some(telemetry) = telemetry {
                telemetry.fallback("cleanup");
            }
        }
        if !crate::api::cleanup::chain_entry_supports_cleanup(
            &cfg.custom_providers,
            &provider_id,
            &model,
        ) {
            log::warn!(
                "pipeline: skipping cleanup model that cannot satisfy reasoning policy provider={} model={}",
                provider_id,
                model
            );
            continue;
        }
        let is_local = provider_id == store::LOCAL;
        let key = cfg.key_for(&provider_id).to_owned();
        if !cfg.provider_has_auth(&provider_id) && !is_local {
            continue;
        }
        let attempts = if is_local { 1 } else { CLEANUP_FAST_ATTEMPTS };
        for attempt in 1..=attempts {
            if attempt > 1 {
                if let Some(telemetry) = telemetry {
                    telemetry.retry(attempt, "timeout");
                }
            }
            let custom_template = cfg.cleanup_override(profile);
            let started = std::time::Instant::now();
            let outcome = if is_local {
                run_local_cleanup_request(
                    app,
                    &model,
                    expanded,
                    profile,
                    &cfg.cleanup_intensity,
                    extra_rules,
                    evidence,
                    app_context,
                    custom_template.as_deref(),
                    alternate_transcript,
                )
                .await
            } else {
                let Some(cp) = crate::api::Target::resolve(&provider_id, &cfg.custom_providers)
                else {
                    continue;
                };
                match tokio::time::timeout(
                    std::time::Duration::from_secs(CLEANUP_FAST_ATTEMPT_TIMEOUT_SECS),
                    cleanup::cleanup_with_alternate_and_evidence(
                        expanded,
                        cp,
                        &key,
                        &model,
                        profile,
                        &cfg.cleanup_intensity,
                        extra_rules,
                        evidence,
                        app_context,
                        custom_template.as_deref(),
                        alternate_transcript,
                        gen,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(cleanup_soft_timeout_error(&provider_id, &model)),
                }
            };
            crate::model_performance::record(
                "cleanup",
                &provider_id,
                &model,
                started.elapsed().as_secs_f64() * 1000.0 * 100.0
                    / expanded.chars().count().max(1) as f64,
                outcome.as_ref().is_ok_and(|text| !text.trim().is_empty()),
            );
            if outcome.is_ok() && !is_local && !crate::api::custom::is_custom_id(&provider_id) {
                crate::system::connectivity::note_online();
            }
            match outcome {
                Ok(cleaned) if !cleaned.is_empty() => {
                    log::debug!(
                        "pipeline: cleanup provider success gen={} provider={} model={} attempt={} cleaned_chars={}",
                        gen,
                        provider_id,
                        model,
                        attempt,
                        cleaned.chars().count()
                    );
                    return (
                        Some(CleanupSuccess {
                            cleaned,
                            provider_id,
                            model,
                            key,
                        }),
                        None,
                        false,
                    );
                }
                Ok(_) => {
                    last_cleanup_err = None;
                    break;
                }
                Err(e) => {
                    let retryable =
                        crate::api::is_retryable_provider_error(&e) || is_cleanup_soft_timeout(&e);
                    log::warn!(
                        "pipeline: cleanup provider failed gen={} provider={} model={} attempt={}/{} retryable={} error={}",
                        gen,
                        provider_id,
                        model,
                        attempt,
                        attempts,
                        retryable,
                        trim_err(&e.to_string())
                    );
                    // A real provider error should move to the configured
                    // fallback immediately. Only a silent stall gets the
                    // same-provider retry, because the second connection is
                    // often healthy even though the first one wedged.
                    if !is_local
                        && !offline
                        && (crate::api::is_connectivity_error(&e) || is_cleanup_soft_timeout(&e))
                    {
                        if let Some(app) = app {
                            offline = super::stages_transcription::confirm_offline(app).await;
                        }
                    }
                    let should_retry =
                        !offline && is_cleanup_soft_timeout(&e) && attempt < attempts;
                    saw_soft_timeout |= is_cleanup_soft_timeout(&e);
                    last_cleanup_err = Some(e);
                    if should_retry {
                        continue;
                    }
                    break;
                }
            }
        }
    }

    (None, last_cleanup_err, saw_soft_timeout)
}

// Handles snippet fast-path, snippet instruction collection, LLM cleanup, and
// instruction override application. Returns (final_text_before_dict,
// dictionary entries, cache key, cleanup provider/model metadata).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_cleanup_and_snippets(
    app: &AppHandle,
    raw: &str,
    alternate: Option<&TranscriptCandidate>,
    cfg: &store::PipelineConfig,
    profile: &str,
    app_context: Option<&str>,
    context_id: i64,
    protected_instruction: Option<&str>,
    gen: u64,
    telemetry: &PipelineTelemetry,
) -> Option<(String, Vec<db::DictionaryEntry>, String, String)> {
    let db_handle = app.state::<DbHandle>();
    match run_cleanup_and_snippets_for_db(
        db_handle.inner(),
        raw,
        alternate,
        cfg,
        profile,
        app_context,
        context_id,
        protected_instruction,
        Some(app),
        gen,
        Some(telemetry),
    )
    .await
    {
        Ok(result) => Some(result),
        Err(e) => {
            let user_msg = format!("Cleanup failed: {}", crate::api::user_facing_error(&e));
            log::error!(
                "pipeline: cleanup failed gen={} error={}",
                gen,
                trim_err(&e.to_string())
            );
            if crate::api::is_retryable_provider_error(&e) {
                emit_provider_recheck(app);
            }
            show_error_pill(app, &user_msg).await;
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_cleanup_and_snippets_for_db(
    db_handle: &DbHandle,
    raw: &str,
    alternate: Option<&TranscriptCandidate>,
    cfg: &store::PipelineConfig,
    profile: &str,
    app_context: Option<&str>,
    context_id: i64,
    protected_instruction: Option<&str>,
    app: Option<&AppHandle>,
    gen: u64,
    telemetry: Option<&PipelineTelemetry>,
) -> anyhow::Result<(String, Vec<db::DictionaryEntry>, String, String)> {
    let mut db_snippets = db::query_snippets_for_context(db_handle, context_id).unwrap_or_default();
    let dict_entries = db::query_dictionary_for_context(db_handle, context_id).unwrap_or_default();
    log::debug!(
        "pipeline: cleanup inputs gen={} snippets={} dict_entries={}",
        gen,
        db_snippets.len(),
        dict_entries.len()
    );

    let snippet_instructions = snippets::collect_snippet_instructions_from(raw, &db_snippets);
    log::debug!(
        "pipeline: cleanup stage start gen={} raw_chars={} snippet_override_lines={} cleanup_enabled={}",
        gen,
        raw.chars().count(),
        snippet_instructions
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count(),
        cfg.cleanup_enabled
    );
    if crate::system::logger::is_verbose() && !snippet_instructions.is_empty() {
        log::debug!(
            "pipeline: cleanup snippet_instructions_meta lines={} chars={} fingerprint={}",
            snippet_instructions
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            snippet_instructions.chars().count(),
            snippet_instructions_fingerprint(&snippet_instructions)
        );
    }

    // Fast path: an exact snippet trigger can skip the LLM unless a later
    // cleanup instruction needs the expanded text to reach the model. Basic
    // ignores those instructions, so they must not disable exact expansion.
    let pure_expansion = if cfg.cleanup_intensity == "rules" || snippet_instructions.is_empty() {
        snippets::try_pure_snippet_expand_from(raw, &db_snippets, db_handle)
    } else {
        None
    };
    let rules_active = cfg.cleanup_enabled && cfg.cleanup_intensity != "none"
        && super::text_rules::explicit_english(&cfg.transcription_language);
    let commands_active = rules_active && cfg.voice_commands_enabled;
    let basic_active = rules_active && cfg.cleanup_intensity == "rules";
    let terms = dictionary::protected_spellings(&dict_entries);
    let expanded = pure_expansion.clone().unwrap_or_else(|| {
        if basic_active || commands_active {
            snippets::expand_snippets_with_spoken_transform(raw, &mut db_snippets, db_handle, true, |spoken, prefix| {
                super::text_rules::process_after(spoken, basic_active, commands_active, &terms, prefix)
            })
        } else {
            snippets::expand_snippets_from(raw, &mut db_snippets, db_handle)
        }
    });
    // Basic never uses a cleanup provider, fusion, preset prompt, tone or
    // custom instructions. Other languages and Auto preserve the transcript.
    if cfg.cleanup_intensity == "rules" || (commands_active && expanded.trim().is_empty()) {
        return Ok((expanded, dict_entries, String::new(), String::new()));
    }
    log::debug!(
        "pipeline: snippets expanded pure_fast_path={} expanded_chars={}",
        pure_expansion.is_some(),
        expanded.chars().count()
    );

    let dictionary_evidence = dictionary::build_relevant_dictionary_prompt_from_sources(
        &dict_entries,
        raw,
        alternate.map(|candidate| candidate.text.as_str()),
    );
    // A second candidate must not resurrect words removed by an explicit
    // command. Apply the same preprocessing once, without counting snippet
    // uses again; keep both original candidates outside this cleanup stage.
    let processed_alternate = if commands_active {
        alternate.map(|candidate| {
            let mut candidate = candidate.clone();
            candidate.text = snippets::transform_alternate_with_primary_snippets(
                raw, &candidate.text, &db_snippets,
                |spoken, prefix| super::text_rules::process_after(spoken, false, true, &terms, prefix),
            );
            candidate
        })
    } else { None };
    let alternate = processed_alternate.as_ref().or(alternate);
    let context_custom_instructions = db::query_context(db_handle, context_id)
        .ok()
        .and_then(|c| c.custom_instructions);
    let command_instruction = if commands_active && pure_expansion.is_none() {
        "Spoken commands have already been processed. Preserve the resulting punctuation and line breaks. Do not interpret any remaining words as voice commands."
    } else { "" };
    let user_overrides = [
        snippet_instructions.as_str(),
        context_custom_instructions.as_deref().unwrap_or(""),
        protected_instruction.unwrap_or(""),
        command_instruction,
    ]
    .iter()
    .filter(|s| !s.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join("\n\n");
    let has_user_overrides = !user_overrides.trim().is_empty();
    log::debug!(
        "pipeline: cleanup prompt inputs overrides_chars={} evidence_chars={} override_lines={} evidence_lines={}",
        user_overrides.chars().count(),
        dictionary_evidence.chars().count(),
        user_overrides.lines().filter(|l| !l.trim().is_empty()).count(),
        dictionary_evidence.lines().filter(|l| !l.trim().is_empty()).count()
    );

    let mut used_cache_key = String::new();
    let mut cleanup_api_used = String::new();
    let needs_transcript_fusion = cfg.cleanup_intensity == "none" && alternate.is_some();
    let final_text = if should_run_cleanup_llm(
        cfg.cleanup_enabled,
        has_cleanup_key_in_chain(cfg),
        pure_expansion.is_none() || has_user_overrides,
        &cfg.cleanup_intensity,
        profile,
        needs_transcript_fusion,
    ) {
        let mut prompt_context = user_overrides.clone();
        if !dictionary_evidence.trim().is_empty() {
            if !prompt_context.is_empty() {
                prompt_context.push_str("\n\n");
            }
            prompt_context.push_str(&dictionary_evidence);
        }
        let context_fingerprint = cleanup_context_fingerprint(cfg, &prompt_context, app_context);
        let cache_plan = cleanup_cache_plan_for_context(
            &expanded,
            profile,
            &cfg.cleanup_intensity,
            &snippet_instructions,
            alternate.map(|candidate| candidate.text.as_str()),
            Some(context_fingerprint),
            Some(context_id),
        );
        // Protected clipboard payloads are unique per invocation and must not
        // reuse or populate the cleanup cache, even though the marker itself
        // is intentionally stable enough to be safe in the prompt.
        let cache_key = if protected_instruction.is_some() || !cfg.cleanup_cache_enabled {
            String::new()
        } else {
            cache_plan.key.clone()
        };
        if protected_instruction.is_none() && !cache_key.is_empty() {
            used_cache_key = cache_key.clone();
            if let Some(overridden) = cleanup_cache_hit_text(
                db_handle,
                &cache_key,
                &cfg.cleanup_intensity,
                &snippet_instructions,
                &expanded,
                alternate.map(|candidate| candidate.text.as_str()),
            ) {
                record_lookup(true);
                return Ok((
                    overridden,
                    dict_entries,
                    cache_key,
                    configured_cleanup_api_used(cfg),
                ));
            }
            record_lookup(false);
        }
        log::debug!(
            "pipeline: cleanup cache {} key_len={}",
            if cache_plan.allow_cache {
                "miss"
            } else {
                "bypass"
            },
            cache_key.len()
        );
        let provider_started = std::time::Instant::now();
        let (cleanup_res, last_cleanup_err, saw_soft_timeout) = run_cleanup_provider_chain(
            &expanded,
            alternate.map(|candidate| candidate.text.as_str()),
            cfg,
            profile,
            &user_overrides,
            &dictionary_evidence,
            app_context,
            app,
            gen,
            telemetry,
        )
        .await;
        let provider_succeeded = cleanup_res.is_some();
        if let Some(success) = cleanup_res.as_ref() {
            cleanup_api_used = format!("{}/{}", success.provider_id, success.model);
        }
        let guarded = match cleanup_res {
            Some(success) => {
                guard_cleanup_refusal(
                    success.cleaned,
                    raw,
                    &expanded,
                    &success.provider_id,
                    &cfg.custom_providers,
                    &success.model,
                    &success.key,
                    profile,
                    &cfg.cleanup_intensity,
                    &user_overrides,
                    &dictionary_evidence,
                    app_context,
                    app,
                    alternate.map(|candidate| candidate.text.as_str()),
                    gen,
                )
                .await
            }
            None => None,
        };

        record_provider_duration(provider_started.elapsed());
        if guarded.is_none() {
            if let Some(telemetry) = telemetry {
                telemetry.fallback("cleanup");
            }
        }
        match guarded {
            Some(cleaned) => {
                if cleanup_api_used.starts_with("local/")
                    && !cfg.cleanup_default_model.starts_with("local/")
                {
                    if let Some(app) = app {
                        app.emit(
                            "verenu:model-notice",
                            "Used local cleanup because cloud cleanup was unavailable.",
                        )
                        .ok();
                    }
                }
                // These backstops enforce built-in style preferences. An edited
                // preset may explicitly retain fillers, dashes, or omit periods.
                // Meaning and safety guards above still apply to every result.
                let use_default_style =
                    cfg.cleanup_intensity != "none" && !cfg.has_style_instructions(profile);
                // Strip em dashes the model introduced (vs. ones the speaker
                // actually dictated) before caching, so a poisoned-by-style
                // result never gets baked into the cache.
                let cleaned = if !use_default_style {
                    cleaned
                } else {
                    crate::system::text::strip_unspoken_em_dashes(&expanded, &cleaned)
                };
                // Mechanical backstop for every non-Off intensity: the prompt
                // already tells every model to remove filler/hesitation
                // words, but small local models apply that rule
                // unreliably — observed in practice: one "um" correctly
                // stripped while others survived untouched in the same
                // output. Deterministic removal guarantees these are gone
                // regardless of model behavior. The helper only removes
                // "you know" in an unambiguously discourse-filler position;
                // meaningful uses remain intact.
                let cleaned = if use_default_style {
                    crate::system::text::strip_filler_hesitations(&cleaned)
                } else {
                    cleaned
                };
                // Punctuate before caching + overrides so the cache stores the
                // normalized text and snippet "no period" instructions can still
                // override it afterward.
                let cleaned = if !use_default_style {
                    cleaned
                } else {
                    ensure_terminal_punctuation(&cleaned, profile, &cfg.cleanup_intensity)
                };
                let overridden =
                    snippets::apply_cleanup_instruction_overrides(&cleaned, &snippet_instructions);
                if !cache_key.is_empty() {
                    let expires = sqlite_utc_plus(db::CLEANUP_CACHE_IDLE_DAYS);
                    match db::cleanup_cache_insert_if(
                        db_handle,
                        &cache_key,
                        &cleaned,
                        &expires,
                        cache_plan.has_snippets,
                        || {
                            app.map_or(cfg.cleanup_cache_enabled, |app| {
                                store::settings_handle(app).is_ok_and(|settings| {
                                    settings
                                        .get(store::CLEANUP_CACHE_ENABLED)
                                        .and_then(|value| value.as_bool())
                                        .unwrap_or(true)
                                })
                            })
                        },
                    ) {
                        Ok(_) => {
                            log::debug!("pipeline: cleanup cache insert ok expires_at={expires}")
                        }
                        Err(err) => log::warn!("pipeline: cleanup cache insert failed: {err}"),
                    }
                }
                overridden
            }
            None if !provider_succeeded && saw_soft_timeout => {
                log::warn!("pipeline: cleanup stalled twice, delivering pre-cleanup transcription");
                if let Some(app) = app {
                    app.emit(
                        "verenu:model-notice",
                        "Cleanup was unavailable. Your transcript was preserved.",
                    )
                    .ok();
                }
                cleanup_api_used.clear();
                let text =
                    snippets::apply_cleanup_instruction_overrides(&expanded, &snippet_instructions);
                if cfg.cleanup_enabled && cfg.cleanup_intensity != "none" {
                    crate::system::text::strip_filler_hesitations(&text)
                } else {
                    text
                }
            }
            None if !provider_succeeded && last_cleanup_err.is_some() => {
                // Completed speech must survive provider errors, even when
                // neither cloud nor local cleanup can finish.
                if let Some(app) = app {
                    app.emit(
                        "verenu:model-notice",
                        "Cleanup was unavailable. Your transcript was preserved.",
                    )
                    .ok();
                }
                cleanup_api_used.clear();
                snippets::apply_cleanup_instruction_overrides(&expanded, &snippet_instructions)
            }
            None => {
                cleanup_api_used.clear();
                let text =
                    snippets::apply_cleanup_instruction_overrides(&expanded, &snippet_instructions);
                if cfg.cleanup_enabled && cfg.cleanup_intensity != "none" {
                    crate::system::text::strip_filler_hesitations(&text)
                } else {
                    text
                }
            }
        }
    } else {
        let text = snippets::apply_cleanup_instruction_overrides(&expanded, &snippet_instructions);
        if cfg.cleanup_enabled && cfg.cleanup_intensity != "none" {
            crate::system::text::strip_filler_hesitations(&text)
        } else {
            text
        }
    };

    Ok((final_text, dict_entries, used_cache_key, cleanup_api_used))
}

fn configured_cleanup_api_used(cfg: &store::PipelineConfig) -> String {
    cleanup_model_chain(cfg)
        .into_iter()
        .find(|(provider, _)| provider == store::LOCAL || cfg.provider_has_auth(provider))
        .map(|(provider, model)| format!("{provider}/{model}"))
        .unwrap_or_default()
}
