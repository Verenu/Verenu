use super::stages_cleanup::{
    cleanup_output_is_unusable_against_candidates, CLEANUP_PROMPT_VERSION,
};
use super::*;
pub(super) struct CleanupCachePlan {
    pub(super) key: String,
    pub(super) allow_cache: bool,
    pub(super) has_snippets: bool,
}

#[cfg(test)]
pub(super) fn cleanup_cache_plan(
    expanded: &str,
    profile: &str,
    intensity: &str,
    snippet_instructions: &str,
    alternate_transcript: Option<&str>,
    dual_context_fingerprint: Option<String>,
) -> CleanupCachePlan {
    cleanup_cache_plan_for_context(
        expanded,
        profile,
        intensity,
        snippet_instructions,
        alternate_transcript,
        dual_context_fingerprint,
        None,
    )
}

pub(super) fn cleanup_cache_plan_for_context(
    expanded: &str,
    profile: &str,
    intensity: &str,
    snippet_instructions: &str,
    alternate_transcript: Option<&str>,
    dual_context_fingerprint: Option<String>,
    context_id: Option<i64>,
) -> CleanupCachePlan {
    let has_snippets = !snippet_instructions.is_empty();
    let cache_tokens = cache_tokens(expanded);
    let allow_cache = should_use_cleanup_cache_tokens(&cache_tokens)
        && (expanded.chars().count() <= 200 || has_snippets);
    let key = if allow_cache {
        // Structured encoding preserves word boundaries, case, punctuation,
        // and field boundaries. Only the digest is persisted in SQLite.
        format!(
            "cleanup-v2:{}",
            cache_digest(&serde_json::json!([
                expanded,
                profile,
                intensity,
                snippet_instructions,
                alternate_transcript,
                dual_context_fingerprint,
                context_id
            ]))
        )
    } else {
        String::new()
    };

    CleanupCachePlan {
        key,
        allow_cache,
        has_snippets,
    }
}

pub(super) fn cleanup_context_fingerprint(
    cfg: &store::PipelineConfig,
    extra_rules: &str,
    app_context: Option<&str>,
) -> String {
    // Tone may come from a Context rather than the global default. Include
    // every preset edit so cached cleanup never survives an instruction edit.
    let edits: std::collections::BTreeMap<_, _> = cfg.style_prompt_instructions.iter().collect();
    cache_digest(&serde_json::json!([
        CLEANUP_PROMPT_VERSION,
        cfg.cleanup_provider,
        cfg.cleanup_default_model,
        cfg.cleanup_fallback_models,
        cfg.custom_providers,
        extra_rules,
        cfg.t3_skill_catalog.as_ref().map(|catalog| (
            &catalog.environment_id,
            &catalog.id,
            &catalog.revision
        )),
        app_context,
        edits,
        cfg.advanced_model_ui,
        cfg.cleanup_prompt_override
    ]))
}

fn touch_cleanup_cache_hit(db_handle: &DbHandle, cache_key: &str, entry: &db::CleanupCacheEntry) {
    let now = Utc::now();
    let new_hit_count = entry.hit_count + 1;
    let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let new_expires_at = next_cache_expiry(&entry.created_at, now);
    match db::cleanup_cache_touch_hit(
        db_handle,
        cache_key,
        &entry.created_at,
        entry.hit_count,
        new_hit_count,
        &now_str,
        &new_expires_at,
    ) {
        Ok(_) => log::debug!(
            "pipeline: cleanup cache touch hit_count={} expires_at={}",
            new_hit_count,
            new_expires_at
        ),
        Err(err) => log::warn!("pipeline: cleanup cache touch failed: {err}"),
    }
}

pub(super) fn cleanup_cache_hit_text(
    db_handle: &DbHandle,
    cache_key: &str,
    intensity: &str,
    snippet_instructions: &str,
    expanded: &str,
    alternate: Option<&str>,
) -> Option<String> {
    let entry = db::cleanup_cache_get_active(db_handle, cache_key)
        .ok()
        .flatten()?;
    // Validate against current candidates without persisting raw transcripts.
    if cleanup_output_is_unusable_against_candidates(
        intensity,
        expanded,
        alternate,
        &entry.clean_text,
    ) {
        log::warn!(
            "pipeline: cleanup cache entry looks unusable (model artifact leak/refusal), evicting and treating as miss key_len={}",
            cache_key.len()
        );
        let _ = db::cleanup_cache_delete_by_key(db_handle, cache_key);
        return None;
    }
    log::debug!(
        "pipeline: cleanup cache hit key_len={} hit_count={}",
        cache_key.len(),
        entry.hit_count
    );
    touch_cleanup_cache_hit(db_handle, cache_key, &entry);
    Some(snippets::apply_cleanup_instruction_overrides(
        &entry.clean_text,
        snippet_instructions,
    ))
}
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn cache_digest(value: &serde_json::Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static CLEANUP_CALLS: AtomicU64 = AtomicU64::new(0);
static CLEANUP_MS: AtomicU64 = AtomicU64::new(0);

/// Process-local counts only. No dictated text or prompt data is recorded.
#[derive(serde::Serialize)]
pub(crate) struct CleanupCacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub provider_calls: u64,
    pub provider_ms: u64,
}

pub(crate) fn metrics() -> CleanupCacheMetrics {
    CleanupCacheMetrics {
        hits: CACHE_HITS.load(Ordering::Relaxed),
        misses: CACHE_MISSES.load(Ordering::Relaxed),
        provider_calls: CLEANUP_CALLS.load(Ordering::Relaxed),
        provider_ms: CLEANUP_MS.load(Ordering::Relaxed),
    }
}

pub(super) fn record_lookup(hit: bool) {
    (if hit { &CACHE_HITS } else { &CACHE_MISSES }).fetch_add(1, Ordering::Relaxed);
}

pub(super) fn record_provider_duration(duration: std::time::Duration) {
    CLEANUP_CALLS.fetch_add(1, Ordering::Relaxed);
    CLEANUP_MS.fetch_add(
        duration.as_millis().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
}

#[cfg(test)]
pub(super) fn should_use_cleanup_cache(raw: &str) -> bool {
    let tokens = cache_tokens(raw);
    should_use_cleanup_cache_tokens(&tokens)
}

pub(super) fn cache_tokens(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub(super) fn should_use_cleanup_cache_tokens(tokens: &[String]) -> bool {
    let mut numeric_count = 0usize;
    let mut has_math_operator = false;

    for t in tokens {
        if t.chars().any(|c| c.is_ascii_digit()) || is_number_word_token(t) {
            numeric_count += 1;
            continue;
        }
        if matches!(
            t.as_str(),
            "plus" | "minus" | "times" | "multiplied" | "multiply" | "divided" | "over" | "x"
        ) {
            has_math_operator = true;
        }
    }

    !(has_math_operator && numeric_count >= 2)
}

pub(super) fn parse_sqlite_utc(s: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(DateTime::from_naive_utc_and_offset(naive, Utc))
}

pub(super) fn sqlite_utc_plus(days: i64) -> String {
    (Utc::now() + Duration::days(days))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub(super) fn next_cache_expiry(created_at: &str, now: DateTime<Utc>) -> String {
    let created = parse_sqlite_utc(created_at).unwrap_or(now);
    let next = (now + Duration::days(db::CLEANUP_CACHE_IDLE_DAYS))
        .min(created + Duration::days(db::CLEANUP_CACHE_MAX_AGE_DAYS));

    next.format("%Y-%m-%d %H:%M:%S").to_string()
}
pub(super) fn should_run_cleanup_llm(
    cleanup_enabled: bool,
    has_cleanup_key: bool,
    no_pure_expansion: bool,
    cleanup_intensity: &str,
    _profile: &str,
    needs_transcript_fusion: bool,
) -> bool {
    cleanup_enabled
        && has_cleanup_key
        && no_pure_expansion
        && (cleanup_intensity != "none" || needs_transcript_fusion)
}

pub(super) fn snippet_instructions_fingerprint(instructions: &str) -> u64 {
    // djb2 hash — deterministic across runs, no external dep
    let mut h: u64 = 5381;
    for b in instructions.bytes() {
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(b as u64);
    }
    h
}
