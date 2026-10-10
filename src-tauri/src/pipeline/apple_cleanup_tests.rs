use super::*;

#[tokio::test(flavor = "current_thread")]
async fn apple_cleanup_uses_own_identity_without_key_or_implicit_fallback() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    // A configured cloud candidate makes the chain ready on Linux. The first
    // Apple adapter is independently exercised with a deterministic fixture.
    config.cleanup_fallback_models = vec!["openai/gpt-4o-mini".into()];
    assert_eq!(config.key_for(store::APPLE_INTELLIGENCE), "");
    assert!(!config.provider_has_auth(store::APPLE_INTELLIGENCE));
    fixture(
        "transcription",
        "groq",
        "whisper-large-v3-turbo",
        Some("please send the note tomorrow"),
        None,
        None,
    );
    fixture(
        "cleanup",
        "apple-intelligence",
        "system",
        Some("Please send the note tomorrow."),
        None,
        None,
    );
    let result = run_pipeline_fixture(base_request(config)).await.unwrap();
    assert_eq!(
        result.final_text_before_dictionary,
        "Please send the note tomorrow."
    );
    assert!(result
        .api_used
        .contains("cleanup=apple-intelligence/system"));
    assert_eq!(
        fixture_hit_count("cleanup", "apple-intelligence", "system"),
        1
    );
    assert_eq!(fixture_hit_count("cleanup", "openai", "gpt-4o-mini"), 0);
    reset();
}

#[tokio::test(flavor = "current_thread")]
async fn apple_cleanup_refusal_retry_stays_on_apple_and_preserves_speech() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    config.cleanup_fallback_models = vec!["openai/gpt-4o-mini".into()];
    fixture(
        "transcription",
        "groq",
        "whisper-large-v3-turbo",
        Some("please send the note tomorrow"),
        None,
        None,
    );
    fixture(
        "cleanup",
        "apple-intelligence",
        "system",
        Some("As an AI, I cannot help with that."),
        None,
        None,
    );
    let result = run_pipeline_fixture(base_request(config)).await.unwrap();
    assert_eq!(
        result.final_text_before_dictionary,
        "please send the note tomorrow"
    );
    assert_eq!(
        fixture_hit_count("cleanup", "apple-intelligence", "system"),
        2
    );
    assert_eq!(fixture_hit_count("cleanup", "openai", "gpt-4o-mini"), 0);
    assert_eq!(result.history_entry.clean_text, result.injected_text);
    reset();
}

#[tokio::test(flavor = "current_thread")]
async fn apple_cleanup_failure_uses_only_explicit_recovery_candidate() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    config.cleanup_fallback_models = vec!["openai/gpt-4o-mini".into()];
    fixture(
        "transcription",
        "groq",
        "whisper-large-v3-turbo",
        Some("please send the note tomorrow"),
        None,
        None,
    );
    fixture(
        "cleanup",
        "apple-intelligence",
        "system",
        None,
        Some("provider"),
        Some("Apple Intelligence model is not ready"),
    );
    fixture(
        "cleanup",
        "openai",
        "gpt-4o-mini",
        Some("Please send the note tomorrow."),
        None,
        None,
    );
    let result = run_pipeline_fixture(base_request(config)).await.unwrap();
    assert_eq!(
        result.final_text_before_dictionary,
        "Please send the note tomorrow."
    );
    assert_eq!(
        fixture_hit_count("cleanup", "apple-intelligence", "system"),
        1
    );
    assert_eq!(fixture_hit_count("cleanup", "openai", "gpt-4o-mini"), 1);
    reset();
}

#[cfg(not(target_os = "macos"))]
#[tokio::test(flavor = "current_thread")]
async fn unavailable_apple_only_chain_preserves_dictation_and_saved_selection() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    config.cleanup_fallback_models.clear();
    assert_eq!(
        store::parse_model_id(&config.cleanup_default_model)
            .unwrap()
            .0,
        store::APPLE_INTELLIGENCE
    );
    fixture(
        "transcription",
        "groq",
        "whisper-large-v3-turbo",
        Some("please send the note tomorrow"),
        None,
        None,
    );
    let result = run_pipeline_fixture(base_request(config)).await.unwrap();
    assert_eq!(
        result.final_text_before_dictionary,
        "please send the note tomorrow"
    );
    assert_eq!(
        fixture_hit_count("cleanup", "apple-intelligence", "system"),
        0
    );
    reset();
}

#[tokio::test(flavor = "current_thread")]
async fn apple_cleanup_cache_does_not_infer_unknown_provider() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    // A configured recovery candidate makes cleanup ready on non-Mac test
    // hosts. Only the Apple fixture may be called, including on repeated runs.
    config.cleanup_fallback_models = vec!["openai/gpt-4o-mini".into()];
    fixture(
        "transcription",
        "groq",
        "whisper-large-v3-turbo",
        Some("please send the note tomorrow"),
        None,
        None,
    );
    fixture(
        "cleanup",
        "apple-intelligence",
        "system",
        Some("Please send the note tomorrow."),
        None,
        None,
    );
    let mut request = base_request(config);
    request.db = Some(crate::data::db::open(":memory:").expect("shared test db"));
    let first = run_pipeline_fixture(request.clone()).await.unwrap();
    let second = run_pipeline_fixture(request).await.unwrap();
    assert!(!first.cleanup_cache_key.is_empty());
    assert_eq!(first.cleanup_cache_key, second.cleanup_cache_key);
    assert!(first.api_used.contains("cleanup=apple-intelligence/system"));
    assert_eq!(second.api_used, "groq/whisper-large-v3-turbo/transcription");
    assert_eq!(
        second.history_entry.clean_text,
        first.history_entry.clean_text
    );
    assert_eq!(
        fixture_hit_count("cleanup", "apple-intelligence", "system"),
        1
    );
    assert_eq!(fixture_hit_count("cleanup", "openai", "gpt-4o-mini"), 0);
    reset();
}

#[tokio::test(flavor = "current_thread")]
async fn apple_cleanup_cache_never_labels_cloud_recovery_as_on_device() {
    let _guard = harness_test_lock().lock().expect("harness lock");
    reset();
    set_enabled(true);
    let mut config = base_config();
    config.cleanup_default_model = "apple-intelligence/system".into();
    config.cleanup_fallback_models = vec!["openai/gpt-4o-mini".into()];
    fixture("transcription", "groq", "whisper-large-v3-turbo", Some("please send the note tomorrow"), None, None);
    fixture("cleanup", "apple-intelligence", "system", None, Some("provider"), Some("Synthetic model unavailable"));
    fixture("cleanup", "openai", "gpt-4o-mini", Some("Please send the note tomorrow."), None, None);
    let mut request = base_request(config);
    request.db = Some(crate::data::db::open(":memory:").expect("shared test db"));
    let first = run_pipeline_fixture(request.clone()).await.unwrap();
    let second = run_pipeline_fixture(request.clone()).await.unwrap();
    assert!(first.api_used.contains("cleanup=openai/gpt-4o-mini"));
    assert!(!first.cleanup_cache_key.is_empty());
    assert_eq!(first.cleanup_cache_key, second.cleanup_cache_key);
    assert_eq!(second.injected_text, first.injected_text);
    assert_eq!(second.api_used, "groq/whisper-large-v3-turbo/transcription");
    let saved_api: String = request.db.as_ref().unwrap().lock().unwrap().query_row(
        "SELECT api_used FROM transcriptions WHERE id = ?1", [second.history_entry.id], |row| row.get(0),
    ).unwrap();
    assert_eq!(saved_api, second.api_used);
    assert_eq!(fixture_hit_count("cleanup", "apple-intelligence", "system"), 1);
    assert_eq!(fixture_hit_count("cleanup", "openai", "gpt-4o-mini"), 1);
    reset();
}
