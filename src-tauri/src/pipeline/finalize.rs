use super::*;
use crate::core::context::ResolvedContextIdentity;

pub(super) struct PipelineCompletionContext<'a> {
    pub(super) telemetry: Option<&'a PipelineTelemetry>,
    pub(super) raw: &'a str,
    pub(super) final_text_before_dict: &'a str,
    pub(super) clipboard_plan: Option<&'a super::clipboard_phrase::ClipboardPhrasePlan>,
    pub(super) clipboard_warning: Option<&'static str>,
    pub(super) dict_entries: &'a [db::DictionaryEntry],
    pub(super) snippet_expansions: i64,
    pub(super) duration_ms: u64,
    pub(super) api_used: &'a str,
    pub(super) target_hwnd: usize,
    pub(super) target: crate::core::window_geometry::WindowTarget,
    pub(super) cfg: &'a store::PipelineConfig,
    pub(super) profile: &'a str,
    pub(super) process_name: String,
    pub(super) cleanup_cache_key: String,
    pub(super) cleanup_cache_result_hash: String,
    pub(super) captured_at: std::time::Instant,
    pub(super) event_only: bool,
    pub(super) caps_lock_on: bool,
    pub(super) context: ResolvedContextIdentity,
}

fn dictionary_protects_initial_case(text: &str, entries: &[db::DictionaryEntry]) -> bool {
    let Some(start) = text
        .char_indices()
        .find_map(|(idx, ch)| ch.is_alphanumeric().then_some(idx))
    else {
        return false;
    };
    let leading = text.trim_start();
    let alphanumeric_leading = &text[start..];
    entries.iter().any(|entry| {
        !entry.term.is_empty()
            && entry.term.chars().any(char::is_uppercase)
            && [leading, alphanumeric_leading]
                .into_iter()
                .any(|candidate| {
                    candidate.strip_prefix(&entry.term).is_some_and(|rest| {
                        rest.chars()
                            .next()
                            .is_none_or(|ch| !ch.is_alphanumeric() && !matches!(ch, '\'' | '’'))
                    })
                })
    })
}

pub(super) async fn finalize_pipeline_completion(
    app: &AppHandle,
    state: &SharedState,
    ctx: PipelineCompletionContext<'_>,
) -> anyhow::Result<db::RecentEntry> {
    let dict_stage = std::time::Instant::now();
    let (mut final_text_substituted, mut applied_dict_ids, mut applied_term_ids, mut vocabulary_replacements) =
        dictionary::apply_substitutions_details(ctx.final_text_before_dict, ctx.dict_entries);
    let mut snippet_expansions = ctx.snippet_expansions;
    let mut dict_changed = !applied_dict_ids.is_empty();
    let apply_caps_lock_upper = ctx.cfg.caps_lock_uppercase_enabled && ctx.caps_lock_on;
    log::debug!(
        "pipeline: caps lock uppercase setting_enabled={} detected_caps_lock_on={} applied={}",
        ctx.cfg.caps_lock_uppercase_enabled,
        ctx.caps_lock_on,
        apply_caps_lock_upper
    );
    if apply_caps_lock_upper {
        final_text_substituted = final_text_substituted.to_uppercase();
    }
    if let Some(catalog) = &ctx.cfg.t3_skill_catalog {
        final_text_substituted = catalog.normalize_mentions(&final_text_substituted);
    }
    let (final_text_substituted, delivered_text, private_text, clipboard_warning) =
        if let Some(plan) = ctx.clipboard_plan {
            let marker_survived = plan
                .markers
                .iter()
                .all(|marker| final_text_substituted.matches(marker).count() == 1);
            let marker_text = if marker_survived {
                final_text_substituted
            } else {
                log::warn!("pipeline: cleanup did not preserve clipboard marker");
                let (mut fallback, fallback_dict_ids, fallback_term_ids, fallback_replacements) =
                    dictionary::apply_substitutions_details(&plan.pre_cleanup, ctx.dict_entries);
                applied_dict_ids = fallback_dict_ids;
                applied_term_ids = fallback_term_ids;
                vocabulary_replacements = fallback_replacements;
                snippet_expansions = 0;
                dict_changed = !applied_dict_ids.is_empty();
                if apply_caps_lock_upper {
                    fallback = fallback.to_uppercase();
                }
                fallback
            };
            let delivered =
                super::clipboard_phrase::restore(&marker_text, plan).unwrap_or_else(|| {
                    log::error!("pipeline: generated clipboard markers were not restorable");
                    plan.markers
                        .iter()
                        .fold(plan.pre_cleanup.clone(), |text, marker| {
                            text.replace(marker, &plan.clipboard_text)
                        })
                });
            let private = super::clipboard_phrase::private_text(&marker_text, plan);
            (
                marker_text,
                delivered,
                private,
                if !marker_survived {
                    Some("Clipboard inserted without cleanup")
                } else {
                    ctx.clipboard_warning
                },
            )
        } else {
            let text = final_text_substituted;
            (text.clone(), text.clone(), text, ctx.clipboard_warning)
        };
    log::debug!(
        "pipeline: dictionary apply changed={} before_chars={} after_chars={} stage_ms={}",
        dict_changed,
        ctx.final_text_before_dict.chars().count(),
        final_text_substituted.chars().count(),
        dict_stage.elapsed().as_millis()
    );
    if dict_changed && crate::system::logger::is_verbose() {
        log::debug!(
            "pipeline: dictionary before_full=\"{}\"",
            ctx.final_text_before_dict
        );
        log::debug!(
            "pipeline: dictionary after_full=\"{}\"",
            final_text_substituted
        );
    }

    let db_handle = app.state::<DbHandle>();
    let db_for_insert = db_handle.inner().clone();
    let raw_for_insert = ctx.raw.to_string();
    let app_name_for_insert = (!ctx.process_name.is_empty() && ctx.process_name != "unknown")
        .then(|| ctx.process_name.clone());
    let clean_for_insert = private_text.clone();
    let api_used_for_insert = ctx.api_used.to_string();
    let duration_for_insert = ctx.duration_ms as i64;
    let dictionary_fixes_applied = applied_dict_ids.len() as i64;
    let context_for_insert = Some(ctx.context.id);
    let website_for_insert = ctx.context.website_domain.clone();
    let entry = match tokio::task::spawn_blocking(move || -> anyhow::Result<db::RecentEntry> {
        let entry = db::insert_transcription_with_metrics(
            &db_for_insert,
            &raw_for_insert,
            &clean_for_insert,
            0,
            duration_for_insert,
            &api_used_for_insert,
            app_name_for_insert.as_deref(),
            context_for_insert,
            website_for_insert.as_deref(),
            Some((vocabulary_replacements, snippet_expansions)),
        )?;
        if dictionary_fixes_applied > 0 {
            if let Err(e) =
                db::increment_lifetime_dictionary_fixes(&db_for_insert, dictionary_fixes_applied)
            {
                log::warn!("pipeline: failed to record dictionary fixes: {e}");
            }
            if db::record_dictionary_usage(&db_for_insert, &applied_term_ids).is_err() {
                log::warn!("pipeline: failed to record library usage");
            }
        }
        Ok(entry)
    })
    .await
    {
        Ok(Ok(entry)) => {
            super::failover::discard_durable();
            if let Ok(mut st) = lock_state(state) {
                st.failover_session_id = None;
                st.failover_reuse_id = false;
                st.failover_started_at_unix = 0;
            }
            entry
        }
        Ok(Err(e)) => {
            show_error_pill(
                app,
                &format!("Failed to save transcription: {}", trim_err(&e.to_string())),
            )
            .await;
            return Err(e);
        }
        Err(e) => {
            show_error_pill(app, "Failed to save transcription: background task crashed").await;
            return Err(anyhow::anyhow!("insert_transcription task panicked: {e}"));
        }
    };

    let calls = db::build_api_calls(
        entry.id,
        &entry.created_at,
        ctx.api_used,
        ctx.duration_ms as i64,
        ctx.raw,
        &private_text,
    );
    if !calls.is_empty() {
        let db_for_calls = db_handle.inner().clone();
        match tokio::task::spawn_blocking(move || db::insert_api_calls(&db_for_calls, &calls)).await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                log::warn!(
                    "pipeline: failed to record api usage rows: {}",
                    trim_err(&e.to_string())
                )
            }
            Err(e) => log::warn!("pipeline: api usage recording task panicked: {e}"),
        }
    }

    if let Ok(mut st) = lock_state(state) {
        if st.retry_capture.as_ref().map(|v| v.captured_at) == Some(ctx.captured_at) {
            st.retry_capture = None;
        }
    }

    let inject_stage = std::time::Instant::now();
    if let Some(telemetry) = ctx.telemetry {
        telemetry.insertion_attempted();
    }
    let protected_initial_case =
        dictionary_protects_initial_case(&delivered_text, ctx.dict_entries)
            || ctx
                .cfg
                .t3_skill_catalog
                .as_ref()
                .is_some_and(|catalog| catalog.starts_with_skill_mention(&delivered_text));

    let self_inject = self_injection_requires_clipboard(app, ctx.target_hwnd);
    log::debug!(
        "pipeline: injection decision target_hwnd={} event_only={} self_inject={}",
        ctx.target_hwnd,
        ctx.event_only,
        self_inject
    );

    let clipboard_is_leading = ctx.clipboard_plan.is_some_and(|plan| {
        plan.markers
            .first()
            .is_some_and(|marker| final_text_substituted.trim_start().starts_with(marker))
    });
    let injected = if ctx.event_only {
        injection::InjectionOutcome {
            text: delivered_text.clone(),
            context_state: "event_only",
            case_decision: "setup_try_event",
            probe_source: "unavailable",
            selection_state: "unknown",
        }
    } else if self_inject {
        log::info!("pipeline: self-inject detected — clipboard fallback");
        if let Err(e) = injection::copy_to_clipboard(&delivered_text).await {
            log::warn!("pipeline: clipboard fallback write failed: {e}");
            if let Some(telemetry) = ctx.telemetry {
                telemetry.failed(FailureCategory::InsertionFailed, false);
            }
        }
        show_copied_pill(app, "Text copied — press Ctrl+V to paste");
        injection::InjectionOutcome {
            text: delivered_text.clone(),
            context_state: "self_inject",
            case_decision: "clipboard_fallback",
            probe_source: "unavailable",
            selection_state: "unknown",
        }
    } else {
        match injection::inject_text(
            &delivered_text,
            &ctx.target,
            ctx.cfg.contextual_formatting_enabled
                && !apply_caps_lock_upper
                && !clipboard_is_leading,
            ctx.cfg.contextual_formatting_enabled,
            ctx.profile,
            &ctx.cfg.transcription_language,
            protected_initial_case,
            ctx.cfg.paste_in_chunks,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                log::error!("inject: {e}");
                if let Ok(mut st) = lock_state(state) {
                    st.paste_failure = Some(PasteFailure {
                        text: delivered_text.clone(),
                        captured_at: std::time::Instant::now(),
                    });
                }
                #[cfg(desktop)]
                let partial = e
                    .downcast_ref::<injection::ChunkedPasteError>()
                    .is_some_and(|error| error.attempted_chunks > 0);
                #[cfg(not(desktop))]
                let partial = false;
                if partial {
                    show_partial_paste_pill(app);
                } else {
                    show_paste_failed_pill(app);
                }
                injection::InjectionOutcome {
                    text: delivered_text.clone(),
                    context_state: "unknown",
                    case_decision: "inject_failed",
                    probe_source: "unavailable",
                    selection_state: "unknown",
                }
            }
        }
    };
    let injected_text = injected.text;
    if let Some(telemetry) = ctx.telemetry {
        match injected.case_decision {
            "inject_failed" => telemetry.failed(FailureCategory::InsertionFailed, false),
            "android_accessibility_handoff" => {}
            "clipboard_fallback" => telemetry.delivered("clipboard_fallback", entry.words),
            _ => telemetry.delivered(
                if ctx.event_only {
                    "event_only"
                } else {
                    "direct_insertion"
                },
                entry.words,
            ),
        }
    }
    log::debug!(
        "pipeline: delivery done contextual_formatting={} context_state={} case_decision={} probe_source={} selection_state={} output_chars={} stage_ms={}",
        ctx.cfg.contextual_formatting_enabled,
        injected.context_state,
        injected.case_decision,
        injected.probe_source,
        injected.selection_state,
        injected_text.chars().count(),
        inject_stage.elapsed().as_millis()
    );
    app.emit("verenu:transcribed", &injected_text).ok();

    if ctx.event_only {
        hide_pill(app);
        return Ok(entry);
    }

    if injected.case_decision == "inject_failed"
        || injected.case_decision == "clipboard_fallback"
        || injected.case_decision == "android_accessibility_handoff"
    {
        log::debug!(
            "pipeline: preserving terminal pill state case_decision={}",
            injected.case_decision
        );
    } else if let Some(message) = clipboard_warning {
        super::show_clipboard_warning_pill(app, message);
    } else {
        hide_pill(app);
    }

    if ctx.clipboard_plan.is_none()
        && !matches!(
            injected.case_decision,
            "inject_failed" | "clipboard_fallback" | "android_accessibility_handoff"
        )
    {
        if !ctx.cleanup_cache_key.is_empty() && !ctx.cleanup_cache_result_hash.is_empty() {
            auto_learn::start_cache_rejection_monitor(
                injected_text.clone(),
                ctx.cleanup_cache_key,
                ctx.cleanup_cache_result_hash,
                ctx.target_hwnd,
                db_handle.inner().clone(),
                app.clone(),
            );
        }
        if ctx.cfg.auto_learn_enabled {
            if !applied_dict_ids.is_empty() {
                auto_learn::start_rejection_monitor(
                    injected_text.clone(),
                    applied_dict_ids,
                    ctx.target_hwnd,
                    ctx.context.clone(),
                    db_handle.inner().clone(),
                    app.clone(),
                );
            }
            auto_learn::start_monitor(
                injected_text,
                ctx.target_hwnd,
                ctx.context.clone(),
                db_handle.inner().clone(),
                app.clone(),
            );
        }
    }

    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(term: &str) -> db::DictionaryEntry {
        db::DictionaryEntry {
            id: 1,
            term: term.to_string(),
            mistake: None,
            auto_learned: false,
            correction_count: 0,
            confidence_tier: "confirmed".to_string(),
            last_seen_at: None,
            created_at: String::new(),
            corrections: Vec::new(),
            context_corrections: Vec::new(),
            inherited_from_everywhere: false,
            assigned_to_context: false,
        }
    }

    #[test]
    fn dictionary_case_protection_only_matches_an_exact_leading_term() {
        let entries = [entry("Verenu")];
        assert!(dictionary_protects_initial_case(
            "\"Verenu works\"",
            &entries
        ));
        assert!(dictionary_protects_initial_case(
            ".NET is a useful platform",
            &[entry(".NET")]
        ));
        assert!(!dictionary_protects_initial_case("Very useful", &entries));
        assert!(!dictionary_protects_initial_case("Use Verenu", &entries));
    }
}
