use super::*;

/// Custom endpoints can be LAN-hosted; loss of internet is not evidence that
/// they are unreachable. Built-in cloud providers can be skipped when offline.
pub(super) fn candidate_available_offline(provider: &str, offline: bool) -> bool {
    !offline || provider == store::LOCAL || crate::api::custom::is_custom_id(provider)
}

#[cfg(test)]
mod offline_tests {
    use super::*;
    #[test]
    fn offline_policy_preserves_local_and_custom_endpoints() {
        assert!(!candidate_available_offline("groq", true));
        assert!(candidate_available_offline("local", true));
        assert!(candidate_available_offline(
            "custom:12345678-1234-1234-1234-123456789012",
            true
        ));
        assert!(candidate_available_offline("groq", false));
    }
    #[test]
    fn manual_order_and_quality_pair_are_preserved() {
        let chain = vec![
            ("groq".into(), "primary".into()),
            ("openai".into(), "second".into()),
            ("local".into(), "offline".into()),
        ];
        let samples = vec![crate::model_performance::ModelPerformance {
            task: "transcription".into(),
            id: "groq/primary".into(),
            samples: 3,
            failures: 3,
            latency_ms: 1.0,
            updated_at_ms: u64::MAX,
        }];
        assert_eq!(
            prioritize_model_chain(chain.clone(), "transcription", "manual", &samples),
            chain
        );
        assert_eq!(
            prioritize_model_chain(chain.clone(), "transcription", "quality", &samples),
            chain
        );
        let balanced = prioritize_model_chain(chain, "transcription", "balanced", &samples);
        assert_eq!(balanced[0].0, "openai");
        assert_eq!(balanced.last().unwrap().0, "local");
    }
}

pub(super) fn transcription_model_chain(cfg: &store::PipelineConfig) -> Vec<(String, String)> {
    model_chain(
        &cfg.transcription_default_model,
        &cfg.transcription_fallback_models,
    )
}

pub(super) fn cleanup_model_chain(cfg: &store::PipelineConfig) -> Vec<(String, String)> {
    model_chain(&cfg.cleanup_default_model, &cfg.cleanup_fallback_models)
}

fn model_chain(default_model: &str, fallback_models: &[String]) -> Vec<(String, String)> {
    let mut chain = Vec::<(String, String)>::new();
    if let Some((provider, model)) = store::parse_model_id(default_model) {
        chain.push((provider, model));
    }
    for id in fallback_models {
        if let Some((provider, model)) = store::parse_model_id(id) {
            if !chain.iter().any(|(p, m)| p == &provider && m == &model) {
                chain.push((provider, model));
            }
        }
    }
    chain
}

pub(super) fn runtime_model_chain(
    app: Option<&AppHandle>,
    cfg: &store::PipelineConfig,
    task: &str,
) -> Vec<(String, String)> {
    let chain = if task == "transcription" {
        transcription_model_chain(cfg)
    } else {
        cleanup_model_chain(cfg)
    };
    let mode = app
        .and_then(|app| store::settings_snapshot(app).ok())
        .and_then(|snapshot| {
            snapshot
                .get(store::MODEL_SELECTION_MODE)
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "manual".into());
    prioritize_model_chain(chain, task, &mode, &crate::model_performance::snapshot())
}

fn prioritize_model_chain(
    chain: Vec<(String, String)>,
    task: &str,
    mode: &str,
    samples: &[crate::model_performance::ModelPerformance],
) -> Vec<(String, String)> {
    if mode == "manual" {
        return chain;
    }
    let (mut cloud, mut local): (Vec<_>, Vec<_>) = chain
        .into_iter()
        .partition(|(provider, _)| provider != store::LOCAL);
    // The Quality pair stays intact. Only recovery candidates are reordered.
    let fixed = if mode == "quality" && task == "transcription" {
        cloud.len().min(2)
    } else {
        0
    };
    crate::model_performance::prioritize(&mut cloud[fixed..], task, samples);
    crate::model_performance::prioritize(&mut local, task, samples);
    cloud.extend(local);
    cloud
}

fn transcription_chain_root(
    local_manager: Option<&crate::local_stt::LocalTranscriptionManager>,
) -> std::path::PathBuf {
    local_manager
        .and_then(|manager| manager.prepare_models_dir().ok())
        .unwrap_or_else(crate::local_stt::LocalTranscriptionManager::models_root)
}

pub(super) fn validate_transcription_chain(
    cfg: &store::PipelineConfig,
    local_manager: Option<&crate::local_stt::LocalTranscriptionManager>,
) -> Result<(), String> {
    let root = transcription_chain_root(local_manager);
    let mut selected_local_missing = false;

    let has_usable_candidate = transcription_model_chain(cfg)
        .iter()
        .any(|(provider, model)| {
            if provider == store::LOCAL {
                let is_downloaded = crate::local_stt::model::manifest_by_id(model)
                    .map(|manifest| manifest.engine_type != crate::local_stt::model::LocalSttEngineType::CtcBooster && manifest.is_downloaded(&root))
                    .unwrap_or(false);
                if !is_downloaded
                    && cfg.transcription_provider == store::LOCAL
                    && store::parse_model_id(&cfg.transcription_default_model).is_some_and(
                        |(provider, selected)| provider == store::LOCAL && selected == *model,
                    )
                {
                    selected_local_missing = true;
                }
                is_downloaded
            } else if crate::api::custom::is_custom_id(provider) {
                cfg.custom_providers
                    .iter()
                    .any(|p| p.id == *provider && p.supports_transcription)
                    && cfg.provider_has_auth(provider)
            } else {
                !cfg.key_for(provider).is_empty()
            }
        });

    if has_usable_candidate {
        Ok(())
    } else if selected_local_missing {
        Err("Download the selected local model.".to_string())
    } else {
        Err("No configured transcription backend is available".to_string())
    }
}

pub(super) fn has_cleanup_key_in_chain(cfg: &store::PipelineConfig) -> bool {
    cleanup_model_chain(cfg).iter().any(|(provider, model)| {
        if !crate::api::cleanup::chain_entry_supports_cleanup(
            &cfg.custom_providers,
            provider,
            model,
        ) {
            return false;
        }
        if provider == store::LOCAL {
            crate::local_llm::model::manifest_by_id(model)
                .map(|manifest| {
                    manifest.is_downloaded(&crate::local_llm::LocalLlmManager::models_root())
                })
                .unwrap_or(false)
        } else {
            cfg.provider_has_auth(provider)
        }
    })
}

pub(super) fn trim_err(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 120 {
        format!("{}…", s.chars().take(117).collect::<String>())
    } else {
        s.to_string()
    }
}
