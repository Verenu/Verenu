//! Privacy-preserving product analytics for the desktop runtime.
//!
//! This is the only desktop module allowed to talk to PostHog. Callers pass
//! semantic, bounded values; they never pass transcript text, exceptions, or
//! window/application data.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use uuid::Uuid;

pub const SCHEMA_VERSION: u8 = 4;
pub const IDENTITY_SCHEMA_VERSION: u8 = 2;

static PANIC_ANALYTICS: OnceLock<Analytics> = OnceLock::new();
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

#[derive(Clone)]
pub struct Analytics {
    enabled: Arc<AtomicBool>,
    install_id: Arc<Mutex<Option<String>>>,
    first_seen_version: Arc<Mutex<Option<String>>>,
    session_id: Arc<Mutex<String>>,
    install_id_path: Arc<std::path::PathBuf>,
    first_seen_version_path: Arc<std::path::PathBuf>,
    milestones_path: Arc<std::path::PathBuf>,
    milestone_state: Arc<Mutex<MilestoneState>>,
    sent_failures: Arc<Mutex<HashSet<String>>>,
    sent_once: Arc<Mutex<HashSet<String>>>,
    recovery_flags: Arc<Mutex<HashSet<String>>>,
    dictation_runs: Arc<Mutex<HashMap<String, DictationRunState>>>,
}

#[derive(Default)]
struct MilestoneState {
    successful_dictations: u32,
    reached: HashSet<String>,
}

#[derive(Clone, Copy)]
struct DictationRunState {
    started_at: Instant,
    recording_duration_ms: u64,
    word_count: i64,
    context_result: &'static str,
    transcription_provider: &'static str,
    transcription_model: &'static str,
    cleanup_provider: &'static str,
    cleanup_model: &'static str,
    retry_count: u8,
    transcription_fallback: bool,
    cleanup_fallback: bool,
    clipboard_fallback: bool,
    last_failure: &'static str,
}

impl Default for DictationRunState {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            recording_duration_ms: 0,
            word_count: 0,
            context_result: "unknown",
            transcription_provider: "unknown",
            transcription_model: "unknown",
            cleanup_provider: "unknown",
            cleanup_model: "unknown",
            retry_count: 0,
            transcription_fallback: false,
            cleanup_fallback: false,
            clipboard_fallback: false,
            last_failure: "unknown",
        }
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Stage {
    Permission,
    Capture,
    Vad,
    Preprocessing,
    Transcription,
    DualTranscription,
    Cleanup,
    Formatting,
    Insertion,
    Clipboard,
    LocalModel,
    Sync,
    Unknown,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Capture => "capture",
            Self::Vad => "vad",
            Self::Preprocessing => "preprocessing",
            Self::Transcription => "transcription",
            Self::DualTranscription => "dual_transcription",
            Self::Cleanup => "cleanup",
            Self::Formatting => "formatting",
            Self::Insertion => "insertion",
            Self::Clipboard => "clipboard",
            Self::LocalModel => "local_model",
            Self::Sync => "sync",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum FailureCategory {
    PermissionMissing,
    PermissionDenied,
    Network,
    Timeout,
    ProviderUnavailable,
    EmptyResponse,
    AudioEmpty,
    AudioTooShort,
    AudioTooQuiet,
    VadRejected,
    ModelUnavailable,
    LocalModelFailure,
    InsertionUnavailable,
    InsertionFailed,
    Cancelled,
    Internal,
    Unknown,
}

/// Fixed domains keep error tracking useful without ever accepting a free-form
/// exception message, provider response, file name, endpoint, or user content.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum ErrorDomain {
    Frontend,
    Backend,
    Capture,
    Transcription,
    Cleanup,
    Insertion,
    Sync,
    Updater,
    Setup,
    Permission,
    LocalModel,
    Database,
}

impl ErrorDomain {
    fn as_str(self) -> &'static str {
        match self {
            Self::Frontend => "frontend",
            Self::Backend => "backend",
            Self::Capture => "capture",
            Self::Transcription => "transcription",
            Self::Cleanup => "cleanup",
            Self::Insertion => "insertion",
            Self::Sync => "sync",
            Self::Updater => "updater",
            Self::Setup => "setup",
            Self::Permission => "permission",
            Self::LocalModel => "local_model",
            Self::Database => "database",
        }
    }
}

#[derive(Clone, Copy)]
pub enum ErrorSeverity {
    Warning,
    Error,
    Fatal,
}

impl ErrorSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }
}

/// A structured, privacy-safe error report. No `Error`, message, path, URL,
/// or stack string can enter this type.
pub struct ErrorReport {
    pub domain: ErrorDomain,
    pub code: &'static str,
    pub stage: Option<Stage>,
    pub severity: ErrorSeverity,
    pub handled: bool,
    pub recovered: bool,
    pub recovery_method: Option<&'static str>,
    pub run_id: Option<String>,
    pub callsite: &'static str,
}

impl FailureCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::PermissionMissing => "permission_missing",
            Self::PermissionDenied => "permission_denied",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::EmptyResponse => "empty_response",
            Self::AudioEmpty => "audio_empty",
            Self::AudioTooShort => "audio_too_short",
            Self::AudioTooQuiet => "audio_too_quiet",
            Self::VadRejected => "vad_rejected",
            Self::ModelUnavailable => "model_unavailable",
            Self::LocalModelFailure => "local_model_failure",
            Self::InsertionUnavailable => "insertion_unavailable",
            Self::InsertionFailed => "insertion_failed",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
            Self::Unknown => "unknown",
        }
    }
}

impl Analytics {
    pub fn new(enabled: bool, data_dir: std::path::PathBuf) -> Self {
        let install_id_path = data_dir.join("analytics_install_id");
        let first_seen_version_path = data_dir.join("analytics_first_seen_version");
        let milestones_path = data_dir.join("analytics_usage_milestones");
        let install_id = if enabled {
            read_or_create_install_id(&install_id_path)
        } else {
            let _ = std::fs::remove_file(&install_id_path);
            None
        };
        let first_seen_version = if enabled && install_id.is_some() {
            read_or_create_first_seen_version(&first_seen_version_path)
        } else {
            let _ = std::fs::remove_file(&first_seen_version_path);
            let _ = std::fs::remove_file(&milestones_path);
            None
        };
        let transport_configured =
            option_env!("VERENU_POSTHOG_PROJECT_TOKEN").is_some_and(|token| !token.is_empty());
        log::info!(
            "posthog: initialized enabled={} transport_configured={} identity_mode=pseudonymous_install",
            enabled,
            transport_configured
        );
        Self {
            enabled: Arc::new(AtomicBool::new(enabled)),
            install_id: Arc::new(Mutex::new(install_id)),
            first_seen_version: Arc::new(Mutex::new(first_seen_version)),
            session_id: Arc::new(Mutex::new(Uuid::new_v4().to_string())),
            install_id_path: Arc::new(install_id_path),
            first_seen_version_path: Arc::new(first_seen_version_path),
            milestone_state: Arc::new(Mutex::new(if enabled {
                read_milestone_state(&milestones_path)
            } else {
                MilestoneState::default()
            })),
            milestones_path: Arc::new(milestones_path),
            sent_failures: Arc::new(Mutex::new(HashSet::new())),
            sent_once: Arc::new(Mutex::new(HashSet::new())),
            recovery_flags: Arc::new(Mutex::new(HashSet::new())),
            dictation_runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Captures a synthetic, content-free `$exception` event that PostHog
    /// Error Tracking can group. Raw errors are deliberately not accepted.
    pub fn capture_sanitized_exception(&self, report: ErrorReport) {
        self.capture_exception("$exception", exception_properties(report));
    }

    pub fn capture_frontend_exception(&self, code: &'static str, handled: bool) {
        self.capture_sanitized_exception(ErrorReport {
            domain: ErrorDomain::Frontend,
            code,
            stage: None,
            severity: if handled {
                ErrorSeverity::Error
            } else {
                ErrorSeverity::Fatal
            },
            handled,
            recovered: false,
            recovery_method: None,
            run_id: None,
            callsite: if handled {
                "frontend_boundary"
            } else {
                "frontend_window"
            },
        });
    }

    /// Panic payloads commonly contain arbitrary values, so the hook reports
    /// a fixed fingerprint and never reads the panic message or location.
    pub fn install_panic_hook(&self) {
        let _ = PANIC_ANALYTICS.set(self.clone());
        if PANIC_HOOK_INSTALLED.set(()).is_err() {
            return;
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(analytics) = PANIC_ANALYTICS.get() {
                analytics.capture_sanitized_exception(ErrorReport {
                    domain: ErrorDomain::Backend,
                    code: "backend_panic",
                    stage: None,
                    severity: ErrorSeverity::Fatal,
                    handled: false,
                    recovered: false,
                    recovery_method: None,
                    run_id: None,
                    callsite: "panic_hook",
                });
            }
            previous(info);
        }));
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Opt-out is immediate. Deleting the installation identity means a later
    /// opt-in cannot join activity from before the opt-out.
    pub fn set_enabled(&self, enabled: bool) {
        let already_enabled = self.enabled.load(Ordering::Acquire) == enabled;
        let has_install_id = self
            .install_id
            .lock()
            .ok()
            .and_then(|install_id| install_id.clone())
            .is_some();
        if already_enabled && (!enabled || has_install_id) {
            return;
        }
        self.enabled.store(enabled, Ordering::Release);
        if enabled {
            let id = Uuid::new_v4().to_string();
            persist_install_id(&self.install_id_path, &id);
            let first_seen_version = env!("CARGO_PKG_VERSION").to_owned();
            persist_first_seen_version(&self.first_seen_version_path, &first_seen_version);
            if let Ok(mut install_id) = self.install_id.lock() {
                *install_id = Some(id);
            }
            if let Ok(mut version) = self.first_seen_version.lock() {
                *version = Some(first_seen_version);
            }
        } else {
            let _ = std::fs::remove_file(&*self.install_id_path);
            let _ = std::fs::remove_file(&*self.first_seen_version_path);
            let _ = std::fs::remove_file(&*self.milestones_path);
            if let Ok(mut install_id) = self.install_id.lock() {
                *install_id = None;
            }
            if let Ok(mut version) = self.first_seen_version.lock() {
                *version = None;
            }
            if let Ok(mut milestones) = self.milestone_state.lock() {
                *milestones = MilestoneState::default();
            }
        }
        if let Ok(mut session_id) = self.session_id.lock() {
            *session_id = Uuid::new_v4().to_string();
        }
        if let Ok(mut failures) = self.sent_failures.lock() {
            failures.clear();
        }
        if let Ok(mut once) = self.sent_once.lock() {
            once.clear();
        }
        if let Ok(mut recovery_flags) = self.recovery_flags.lock() {
            recovery_flags.clear();
        }
        if let Ok(mut runs) = self.dictation_runs.lock() {
            runs.clear();
        }
    }

    pub fn new_run_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    pub fn app_launched(
        &self,
        first_launch: bool,
        settings: &crate::data::store::SettingsHandle,
        context_group_count: i64,
    ) {
        self.capture_once(
            "app_launched",
            "app_launched",
            json!({ "first_launch": first_launch }),
        );
        self.settings_snapshot(settings, context_group_count);
    }

    pub fn setup_event(&self, event: &'static str, step: Option<&'static str>) {
        let properties = step
            .map(|step| json!({ "setup_step": step }))
            .unwrap_or_else(|| json!({}));
        if matches!(event, "setup_started" | "setup_completed") {
            self.capture_once(event, event, properties);
        } else {
            // Step views/completions are intentionally repeatable: revisiting
            // a setup step is useful product behavior, not duplicate startup
            // noise. The step and event name remain strictly allowlisted.
            self.capture(event, properties);
        }
    }

    pub fn setup_step_duration(&self, step: &'static str, duration_ms: u64) {
        self.capture(
            "setup_step_duration",
            json!({
                "setup_step": step,
                "duration_bucket": duration_bucket(duration_ms),
            }),
        );
    }

    pub fn settings_snapshot(
        &self,
        settings: &crate::data::store::SettingsHandle,
        context_group_count: i64,
    ) {
        let bool_value = |key: &str| {
            settings
                .get(key)
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        };
        let category = |key: &str| {
            settings
                .get(key)
                .and_then(|value| value.as_str().map(normalize_category))
                .unwrap_or("unknown")
        };
        let context_group_count = context_group_count.clamp(0, 200);
        let feature_breadth = settings
            .snapshot()
            .map(|snapshot| {
                crate::data::store::analytics_feature_breadth(&snapshot, context_group_count)
            })
            .unwrap_or(0);
        self.capture_once("settings_snapshot", "settings_snapshot", json!({
            "cleanup_enabled": bool_value(crate::data::store::CLEANUP_ENABLED),
            "dual_transcription_enabled": bool_value(crate::data::store::DUAL_TRANSCRIPTION_ENABLED),
            "noise_reduction": bool_value(crate::data::store::NOISE_REDUCTION),
            "auto_learn_enabled": bool_value(crate::data::store::AUTO_LEARN_ENABLED),
            "contextual_formatting": bool_value(crate::data::store::CONTEXTUAL_FORMATTING),
            "pause_media": bool_value(crate::data::store::PAUSE_MEDIA_DURING_DICTATION),
            "transcription_provider": category(crate::data::store::TRANSCRIPTION_PROVIDER),
            "cleanup_provider": category(crate::data::store::CLEANUP_PROVIDER),
            "cleanup_intensity": category(crate::data::store::CLEANUP_INTENSITY),
            "history_retention": category(crate::data::store::HISTORY_RETENTION),
            "local_model_memory_policy": category(crate::data::store::LOCAL_MODEL_MEMORY_POLICY),
            "mic_mute_button_dictation": bool_value(crate::data::store::MIC_MUTE_BUTTON_DICTATION),
            "sync_enabled": bool_value(crate::data::store::SYNC_ENABLED),
            "context_group_count": context_group_count,
            "feature_breadth": feature_breadth,
        }));
    }

    pub fn setting_changed(&self, key: &str, value: &Value) {
        let (property, safe_value) = match key {
            crate::data::store::CLEANUP_ENABLED => ("cleanup_enabled", json!(value.as_bool())),
            crate::data::store::DUAL_TRANSCRIPTION_ENABLED => {
                ("dual_transcription_enabled", json!(value.as_bool()))
            }
            crate::data::store::NOISE_REDUCTION => ("noise_reduction", json!(value.as_bool())),
            crate::data::store::AUTO_LEARN_ENABLED => {
                ("auto_learn_enabled", json!(value.as_bool()))
            }
            crate::data::store::CONTEXTUAL_FORMATTING => {
                ("contextual_formatting", json!(value.as_bool()))
            }
            crate::data::store::MIC_MUTE_BUTTON_DICTATION => {
                ("mic_mute_button_dictation", json!(value.as_bool()))
            }
            crate::data::store::SYNC_ENABLED => ("sync_enabled", json!(value.as_bool())),
            crate::data::store::PAUSE_MEDIA_DURING_DICTATION => {
                ("pause_media", json!(value.as_bool()))
            }
            crate::data::store::TRANSCRIPTION_PROVIDER => (
                "transcription_provider",
                json!(value.as_str().map(normalize_category).unwrap_or("unknown")),
            ),
            crate::data::store::CLEANUP_PROVIDER => (
                "cleanup_provider",
                json!(value.as_str().map(normalize_category).unwrap_or("unknown")),
            ),
            crate::data::store::CLEANUP_INTENSITY => (
                "cleanup_intensity",
                json!(value.as_str().map(normalize_category).unwrap_or("unknown")),
            ),
            crate::data::store::HISTORY_RETENTION => (
                "history_retention",
                json!(value.as_str().map(normalize_category).unwrap_or("unknown")),
            ),
            crate::data::store::LOCAL_MODEL_MEMORY_POLICY => (
                "local_model_memory_policy",
                json!(value.as_str().map(normalize_category).unwrap_or("unknown")),
            ),
            _ => return,
        };
        self.capture(
            "setting_changed",
            json!({ "setting": property, "value": safe_value }),
        );
    }

    fn capture_once(&self, event: &'static str, key: &str, properties: Value) {
        let fresh = self
            .sent_once
            .lock()
            .map(|mut seen| seen.insert(key.to_owned()))
            .unwrap_or(true);
        if fresh {
            self.capture(event, properties);
        }
    }

    pub fn dictation_started(&self, run_id: &str, handsfree: bool, noise_reduction: bool) {
        if let Ok(mut runs) = self.dictation_runs.lock() {
            runs.insert(
                run_id.to_owned(),
                DictationRunState {
                    started_at: Instant::now(),
                    ..DictationRunState::default()
                },
            );
        }
        self.capture(
            "dictation_started",
            json!({ "run_id": run_id, "handsfree": handsfree, "noise_reduction": noise_reduction }),
        );
    }
    pub fn recording_finished(&self, run_id: &str, duration_ms: u64) {
        self.update_run(run_id, |run| {
            run.recording_duration_ms = duration_ms.min(600_000)
        });
        self.capture(
            "recording_finished",
            json!({ "run_id": run_id, "recording_duration_bucket": duration_bucket(duration_ms) }),
        );
    }
    pub fn dictation_cancelled(&self, run_id: &str, resumable: bool) {
        self.capture(
            "dictation_cancelled",
            json!({ "run_id": run_id, "resumable": resumable }),
        );
    }
    pub fn feature_used(&self, run_id: &str, feature: &'static str) {
        let feature = normalize_feature(feature);
        self.capture(
            "feature_used",
            json!({ "run_id": run_id, "feature": feature }),
        );
    }
    pub fn retry_attempted(&self, run_id: &str, attempt: u8, reason: &'static str) {
        self.update_run(run_id, |run| {
            run.retry_count = run.retry_count.max(attempt.min(3))
        });
        if let Ok(mut flags) = self.recovery_flags.lock() {
            flags.insert(format!("{run_id}:retry"));
        }
        self.capture(
            "retry_attempted",
            json!({
                "run_id": run_id,
                "attempt_bucket": attempt_bucket(attempt),
                "retry_reason": normalize_failure_reason(reason),
            }),
        );
    }
    pub fn fallback_used(&self, run_id: &str, fallback: &'static str) {
        let fallback = normalize_fallback(fallback);
        self.update_run(run_id, |run| match fallback {
            "transcription" => run.transcription_fallback = true,
            "cleanup" => run.cleanup_fallback = true,
            "clipboard" => run.clipboard_fallback = true,
            _ => {}
        });
        if let Ok(mut flags) = self.recovery_flags.lock() {
            flags.insert(format!("{run_id}:{fallback}"));
        }
        self.capture(
            "fallback_used",
            json!({
                "run_id": run_id,
                "fallback": fallback,
            }),
        );
    }
    pub fn permission_event(&self, permission_type: &'static str, status: &'static str) {
        self.capture(
            "permission_event",
            json!({
                "permission_type": normalize_permission(permission_type),
                "permission_status": normalize_permission_status(status),
            }),
        );
    }
    pub fn input_health(&self, outcome: &'static str) {
        self.capture(
            "input_health",
            json!({ "input_outcome": normalize_input_outcome(outcome) }),
        );
    }
    pub fn sync_event(&self, event: &'static str, status: &'static str) {
        self.capture(
            event,
            json!({ "sync_status": normalize_sync_status(status) }),
        );
    }
    pub fn updater_event(
        &self,
        event: &'static str,
        from_version: Option<&str>,
        to_version: Option<&str>,
    ) {
        let mut properties = json!({});
        if let Value::Object(map) = &mut properties {
            if let Some(from) = from_version.filter(|v| is_official_version(v)) {
                map.insert("from_version".into(), json!(from));
            }
            if let Some(to) = to_version.filter(|v| is_official_version(v)) {
                map.insert("to_version".into(), json!(to));
            }
        }
        self.capture(event, properties);
    }
    pub fn pipeline_stage_started(&self, run_id: &str, stage: Stage) {
        self.capture(
            "pipeline_stage_started",
            json!({ "run_id": run_id, "pipeline_stage": stage.as_str() }),
        );
    }
    pub fn pipeline_stage_completed(&self, run_id: &str, stage: Stage, duration_ms: u128) {
        self.pipeline_stage_completed_with_model(run_id, stage, duration_ms, None);
    }
    pub fn pipeline_stage_completed_with_model(
        &self,
        run_id: &str,
        stage: Stage,
        duration_ms: u128,
        provider_model: Option<&str>,
    ) {
        let provider = provider_model.and_then(parse_safe_provider_model);
        if let Some((provider, model)) = provider {
            self.update_run(run_id, |run| match stage {
                Stage::Transcription | Stage::DualTranscription => {
                    run.transcription_provider = provider;
                    run.transcription_model = model;
                }
                Stage::Cleanup => {
                    run.cleanup_provider = provider;
                    run.cleanup_model = model;
                }
                _ => {}
            });
        }
        let mut properties = json!({
            "run_id": run_id,
            "pipeline_stage": stage.as_str(),
            "duration_bucket": duration_bucket(duration_ms as u64),
            "duration_ms": (duration_ms as u64).min(300_000),
        });
        add_model_properties(&mut properties, provider_model);
        self.capture("pipeline_stage_completed", properties);
    }
    pub fn pipeline_failed(&self, run_id: &str, stage: Stage, category: FailureCategory) {
        self.pipeline_failed_with_model(run_id, stage, category, None);
    }
    pub fn pipeline_failed_with_model(
        &self,
        run_id: &str,
        stage: Stage,
        category: FailureCategory,
        provider_model: Option<&str>,
    ) {
        self.update_run(run_id, |run| {
            run.last_failure = category.as_str();
            if let Some((provider, model)) = provider_model.and_then(parse_safe_provider_model) {
                match stage {
                    Stage::Transcription | Stage::DualTranscription => {
                        run.transcription_provider = provider;
                        run.transcription_model = model;
                    }
                    Stage::Cleanup => {
                        run.cleanup_provider = provider;
                        run.cleanup_model = model;
                    }
                    _ => {}
                }
            }
        });
        let key = format!("{run_id}:{}:{}", stage.as_str(), category.as_str());
        let fresh = self
            .sent_failures
            .lock()
            .map(|mut seen| seen.insert(key))
            .unwrap_or(true);
        if fresh {
            let mut properties = json!({
                "run_id": run_id,
                "stage": stage.as_str(),
                "category": category.as_str(),
            });
            add_model_properties(&mut properties, provider_model);
            self.capture("pipeline_failed", properties);
        }
    }
    pub fn insertion_attempted(&self, run_id: &str) {
        self.capture("insertion_attempted", json!({ "run_id": run_id }));
    }
    pub fn delivery_outcome(&self, run_id: &str, outcome: &'static str) {
        // One final outcome is the denominator for the delivery-quality
        // dashboard.  Retries and late error paths must not create a second
        // terminal outcome for the same opaque run ID.
        let resolved = self.resolved_outcome(run_id, outcome);
        let fresh = self
            .sent_once
            .lock()
            .map(|mut seen| seen.insert(format!("dictation_outcome:{run_id}")))
            .unwrap_or(true);
        let run = self
            .dictation_runs
            .lock()
            .ok()
            .and_then(|mut runs| runs.remove(run_id))
            .unwrap_or_default();
        if fresh {
            let status = outcome_status(resolved);
            let reason = outcome_reason(resolved, run.last_failure);
            self.capture(
                "dictation_outcome",
                json!({
                    "run_id": run_id,
                    "outcome": resolved,
                    "status": status,
                    "reason": reason,
                    "total_duration_ms": run.started_at.elapsed().as_millis().min(900_000) as u64,
                    "recording_duration_ms": run.recording_duration_ms,
                    "recording_duration_bucket": duration_bucket(run.recording_duration_ms),
                    "word_count": run.word_count.clamp(0, 10_000),
                    "transcription_provider": run.transcription_provider,
                    "transcription_model": run.transcription_model,
                    "cleanup_provider": run.cleanup_provider,
                    "cleanup_model": run.cleanup_model,
                    "context_result": run.context_result,
                    "retry_attempt_bucket": attempt_bucket(run.retry_count),
                    "transcription_fallback_used": run.transcription_fallback,
                    "cleanup_fallback_used": run.cleanup_fallback,
                    "clipboard_fallback_used": run.clipboard_fallback,
                    "recovered": resolved.starts_with("success_after_")
                }),
            );
        }
        if let Ok(mut flags) = self.recovery_flags.lock() {
            let prefix = format!("{run_id}:");
            flags.retain(|flag| !flag.starts_with(&prefix));
        }
    }

    fn resolved_outcome(&self, run_id: &str, requested: &str) -> &'static str {
        let requested = normalize_outcome(requested);
        if !matches!(requested, "success_clean" | "unknown") {
            return requested;
        }
        let has = |flag: &str| {
            self.recovery_flags
                .lock()
                .map(|flags| flags.contains(&format!("{run_id}:{flag}")))
                .unwrap_or(false)
        };
        if has("clipboard") {
            "success_after_clipboard_fallback"
        } else if has("cleanup") {
            "success_after_cleanup_fallback"
        } else if has("transcription") {
            "success_after_transcription_fallback"
        } else if has("retry") {
            "success_after_retry"
        } else {
            requested
        }
    }
    fn update_run(&self, run_id: &str, update: impl FnOnce(&mut DictationRunState)) {
        if let Ok(mut runs) = self.dictation_runs.lock() {
            let run = runs.entry(run_id.to_owned()).or_default();
            update(run);
        }
    }
    pub fn context_outcome(&self, run_id: &str, matched: bool, source: &'static str) {
        self.update_run(run_id, |run| {
            run.context_result = if source == "manual" {
                "manual"
            } else if matched {
                "matched"
            } else {
                "no_match"
            };
        });
        self.capture(
            "context_match",
            json!({
                "run_id": run_id,
                "match_result": if matched { "matched" } else { "no_match" },
                "match_source": normalize_match_source(source),
            }),
        );
    }
    pub fn dictation_inserted(&self, run_id: &str, method: &'static str, word_count: i64) {
        self.update_run(run_id, |run| run.word_count = word_count.clamp(0, 10_000));
        self.capture(
            "dictation_inserted",
            json!({
                "run_id": run_id,
                "delivery_method": method,
                "word_count": word_count.clamp(0, 10_000),
            }),
        );
        self.record_success_milestones();
    }

    fn record_success_milestones(&self) {
        if !self.enabled() {
            return;
        }
        let Ok(mut state) = self.milestone_state.lock() else {
            return;
        };
        state.successful_dictations = state.successful_dictations.saturating_add(1).min(100_000);
        let milestones = [
            (5, "dictations_5"),
            (10, "dictations_10"),
            (25, "dictations_25"),
        ];
        let reached: Vec<&'static str> = milestones
            .into_iter()
            .filter(|(threshold, name)| {
                state.successful_dictations >= *threshold && !state.reached.contains(*name)
            })
            .map(|(_, name)| name)
            .collect();
        for milestone in &reached {
            state.reached.insert((*milestone).to_owned());
        }
        persist_milestone_state(&self.milestones_path, &state);
        drop(state);
        for milestone in reached {
            self.capture("usage_milestone_reached", json!({ "milestone": milestone }));
        }
    }

    /// The only trusted `$exception` producer. It is fed solely by
    /// `capture_sanitized_exception`, which has no raw error/message input.
    fn capture_exception(&self, event: &'static str, properties: Value) {
        self.dispatch_exception(event, properties);
    }

    fn capture(&self, event: &'static str, properties: Value) {
        self.dispatch(event, safe_properties(event, properties));
    }

    fn dispatch(&self, event: &'static str, properties: Value) {
        if !self.enabled() {
            return;
        }
        let token = option_env!("VERENU_POSTHOG_PROJECT_TOKEN")
            .unwrap_or("")
            .to_owned();
        let host = option_env!("VERENU_POSTHOG_HOST")
            .unwrap_or("https://us.i.posthog.com")
            .trim_end_matches('/')
            .to_owned();
        if token.is_empty() {
            return;
        }
        let distinct_id = self
            .install_id
            .lock()
            .ok()
            .and_then(|id| id.clone())
            .unwrap_or_default();
        if distinct_id.is_empty() {
            return;
        }
        let session_id = self
            .session_id
            .lock()
            .map(|id| id.clone())
            .unwrap_or_default();
        let first_seen_version = self
            .first_seen_version
            .lock()
            .ok()
            .and_then(|version| version.clone());
        let safe = match properties {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        let payload = outbound_payload(
            &token,
            event,
            &distinct_id,
            add_common_properties(safe, &session_id, first_seen_version.as_deref()),
        );
        log::debug!(
            "analytics: event queued name={} schema={}",
            event,
            SCHEMA_VERSION
        );
        let enabled = self.enabled.clone();
        tauri::async_runtime::spawn(async move {
            if !enabled.load(Ordering::Acquire) {
                return;
            }
            let result = reqwest::Client::new()
                .post(format!("{host}/capture/"))
                .json(&payload)
                .send()
                .await;
            if let Err(error) = result {
                log::debug!(
                    "analytics: delivery unavailable ({})",
                    error.status().map(|s| s.as_u16()).unwrap_or(0)
                );
            }
        });
    }

    /// Error Tracking's documented manual endpoint expects the installation
    /// ID in `properties`, unlike the legacy `/capture/` event envelope.
    /// Keeping this separate makes the unusual wire contract reviewable.
    fn dispatch_exception(&self, event: &'static str, properties: Value) {
        if !self.enabled() {
            return;
        }
        let token = option_env!("VERENU_POSTHOG_PROJECT_TOKEN")
            .unwrap_or("")
            .to_owned();
        let host = option_env!("VERENU_POSTHOG_HOST")
            .unwrap_or("https://us.i.posthog.com")
            .trim_end_matches('/')
            .to_owned();
        if token.is_empty() {
            return;
        }
        let distinct_id = self
            .install_id
            .lock()
            .ok()
            .and_then(|id| id.clone())
            .unwrap_or_default();
        if distinct_id.is_empty() {
            return;
        }
        let session_id = self
            .session_id
            .lock()
            .map(|id| id.clone())
            .unwrap_or_default();
        let first_seen_version = self
            .first_seen_version
            .lock()
            .ok()
            .and_then(|version| version.clone());
        let safe = match properties {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        let payload = error_tracking_payload(
            &token,
            event,
            &distinct_id,
            add_common_properties(safe, &session_id, first_seen_version.as_deref()),
        );
        log::debug!(
            "analytics: error tracking event queued name={} schema={}",
            event,
            SCHEMA_VERSION
        );
        let enabled = self.enabled.clone();
        tauri::async_runtime::spawn(async move {
            if !enabled.load(Ordering::Acquire) {
                return;
            }
            let result = reqwest::Client::new()
                .post(format!("{host}/i/v0/e/"))
                .json(&payload)
                .send()
                .await;
            if let Err(error) = result {
                log::debug!(
                    "analytics: error tracking delivery unavailable ({})",
                    error.status().map(|s| s.as_u16()).unwrap_or(0)
                );
            }
        });
    }
}

fn outbound_payload(token: &str, event: &str, distinct_id: &str, properties: Value) -> Value {
    json!({
        "api_key": token,
        "event": event,
        "distinct_id": distinct_id,
        "properties": properties,
    })
}

fn error_tracking_payload(token: &str, event: &str, distinct_id: &str, properties: Value) -> Value {
    let mut properties = properties.as_object().cloned().unwrap_or_default();
    properties.insert("distinct_id".into(), json!(distinct_id));
    json!({ "token": token, "event": event, "properties": properties })
}

fn add_common_properties(
    mut safe: serde_json::Map<String, Value>,
    session_id: &str,
    first_seen_version: Option<&str>,
) -> Value {
    safe.insert("analytics_schema_version".into(), json!(SCHEMA_VERSION));
    safe.insert(
        "identity_schema_version".into(),
        json!(IDENTITY_SCHEMA_VERSION),
    );
    safe.insert("platform".into(), json!(std::env::consts::OS));
    safe.insert("arch".into(), json!(std::env::consts::ARCH));
    safe.insert("app_version".into(), json!(env!("CARGO_PKG_VERSION")));
    safe.insert(
        "build_channel".into(),
        json!(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }),
    );
    safe.insert("analytics_session_id".into(), json!(session_id));
    if let Some(version) = first_seen_version.filter(|version| is_official_version(version)) {
        safe.insert("first_seen_version".into(), json!(version));
    }
    // PostHog's ingestion endpoint otherwise derives GeoIP data from the
    // connection address. These are processing controls, not analytics
    // dimensions: keep no IP-derived location and no person profile.
    safe.insert("$ip".into(), json!("0.0.0.0"));
    safe.insert("$geoip_disable".into(), json!(true));
    safe.insert("$process_person_profile".into(), json!(false));
    Value::Object(safe)
}

/// Builds the exact custom property object placed in the outbound `$exception`
/// payload. Its input is typed so caller-provided error text cannot enter it.
fn exception_properties(report: ErrorReport) -> Value {
    let domain = report.domain.as_str();
    let code = normalize_error_code(report.code);
    let callsite = normalize_error_callsite(report.callsite);
    let fingerprint = format!("verenu:{domain}:{code}:{callsite}");
    let stage = report.stage.map(Stage::as_str).unwrap_or("unknown");
    json!({
        "$exception_fingerprint": fingerprint,
        "$issue_name": format!("{domain}_{code}"),
        "$issue_description": format!("Verenu {domain} error: {code}"),
        "$exception_level": report.severity.as_str(),
        "$exception_list": [{
            "type": format!("Verenu.{domain}.{code}"),
            "value": format!("{domain}_{code}"),
            // This represents a real application error path even though its
            // content is normalized before transport. PostHog uses this flag
            // when deciding whether to materialize an Error Tracking issue.
            "mechanism": { "handled": report.handled, "synthetic": false },
            "stacktrace": {
                "type": "raw",
                "frames": [{
                    "platform": "custom",
                    "lang": "rust",
                    "filename": "verenu",
                    "function": callsite,
                    "lineno": 0,
                    "in_app": true
                }]
            }
        }],
        "error_domain": domain,
        "error_code": code,
        "error_stage": stage,
        "error_severity": report.severity.as_str(),
        "handled": report.handled,
        "recovered": report.recovered,
        "recovery_method": report.recovery_method.map(normalize_recovery_method).unwrap_or("none"),
        "run_id": safe_run_id(report.run_id),
        "error_callsite": callsite,
    })
}

fn safe_properties(event: &str, properties: Value) -> Value {
    let allowed: &[&str] = match event {
        "app_launched" => &["first_launch"],
        "setup_started" | "setup_completed" => &[],
        "setup_step_viewed" | "setup_step_completed" => &["setup_step"],
        "setup_step_duration" => &["setup_step", "duration_bucket"],
        "settings_snapshot" => &[
            "cleanup_enabled",
            "dual_transcription_enabled",
            "noise_reduction",
            "auto_learn_enabled",
            "contextual_formatting",
            "pause_media",
            "transcription_provider",
            "cleanup_provider",
            "cleanup_intensity",
            "history_retention",
            "local_model_memory_policy",
            "context_group_count",
            "feature_breadth",
            "mic_mute_button_dictation",
            "sync_enabled",
        ],
        "setting_changed" => &["setting", "value"],
        "dictation_started" => &["run_id", "handsfree", "noise_reduction"],
        "recording_finished" => &["run_id", "recording_duration_bucket"],
        "dictation_cancelled" => &["run_id", "resumable"],
        "feature_used" => &["run_id", "feature"],
        "retry_attempted" => &["run_id", "attempt_bucket", "retry_reason"],
        "fallback_used" => &["run_id", "fallback"],
        "permission_event" => &["permission_type", "permission_status"],
        "input_health" => &["input_outcome"],
        "sync_enabled" | "sync_flow" | "sync_started" | "sync_completed" | "sync_failed" => {
            &["sync_status"]
        }
        "update_available"
        | "update_download_started"
        | "update_download_completed"
        | "update_download_failed"
        | "install_requested"
        | "first_launch_after_update"
        | "migration_started"
        | "migration_completed"
        | "migration_failed" => &["from_version", "to_version"],
        "pipeline_stage_started" => &["run_id", "pipeline_stage"],
        "pipeline_stage_completed" => &[
            "run_id",
            "pipeline_stage",
            "duration_bucket",
            "duration_ms",
            "provider",
            "model",
        ],
        "pipeline_failed" => &["run_id", "stage", "category", "provider", "model"],
        "context_match" => &["run_id", "match_result", "match_source"],
        "insertion_attempted" => &["run_id"],
        "dictation_outcome" => &[
            "run_id",
            "outcome",
            "status",
            "reason",
            "total_duration_ms",
            "recording_duration_ms",
            "recording_duration_bucket",
            "word_count",
            "transcription_provider",
            "transcription_model",
            "cleanup_provider",
            "cleanup_model",
            "context_result",
            "retry_attempt_bucket",
            "transcription_fallback_used",
            "cleanup_fallback_used",
            "clipboard_fallback_used",
            "recovered",
        ],
        "dictation_inserted" => &["run_id", "delivery_method", "word_count"],
        "usage_milestone_reached" => &["milestone"],
        _ => &[],
    };
    let Value::Object(mut map) = properties else {
        return Value::Object(serde_json::Map::new());
    };
    map.retain(|key, _| allowed.contains(&key.as_str()));
    // A dictation correlation ID must be the random UUID made at the start of
    // a run. Dropping any other value prevents a future caller from smuggling
    // text or an application identifier through a superficially safe field.
    map.retain(|key, value| {
        key != "run_id"
            || value
                .as_str()
                .is_some_and(|run_id| Uuid::parse_str(run_id).is_ok())
    });
    // Keep this boundary defensive even when a future caller bypasses one of
    // the typed convenience methods above.  Every string-valued telemetry
    // field is reduced to an explicit low-cardinality vocabulary here.
    match event {
        "setup_step_viewed" | "setup_step_completed" => {
            if let Some(value) = map.get_mut("setup_step") {
                *value = json!(normalize_setup_step(value.as_str().unwrap_or("")));
            }
        }
        "setup_step_duration" => {
            if let Some(value) = map.get_mut("setup_step") {
                *value = json!(normalize_setup_step(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("duration_bucket") {
                *value = json!(normalize_duration_bucket(value.as_str().unwrap_or("")));
            }
        }
        "setting_changed" => {
            if let Some(value) = map.get_mut("setting") {
                *value = json!(normalize_setting(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("value") {
                *value = safe_setting_value(value);
            }
        }
        "feature_used" => {
            if let Some(value) = map.get_mut("feature") {
                *value = json!(normalize_feature(value.as_str().unwrap_or("")));
            }
        }
        "retry_attempted" => {
            if let Some(value) = map.get_mut("attempt_bucket") {
                *value = json!(normalize_attempt_bucket(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("retry_reason") {
                *value = json!(normalize_failure_reason(value.as_str().unwrap_or("")));
            }
        }
        "fallback_used" => {
            if let Some(value) = map.get_mut("fallback") {
                *value = json!(normalize_fallback(value.as_str().unwrap_or("")));
            }
        }
        "permission_event" => {
            if let Some(value) = map.get_mut("permission_type") {
                *value = json!(normalize_permission(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("permission_status") {
                *value = json!(normalize_permission_status(value.as_str().unwrap_or("")));
            }
        }
        "input_health" => {
            if let Some(value) = map.get_mut("input_outcome") {
                *value = json!(normalize_input_outcome(value.as_str().unwrap_or("")));
            }
        }
        "sync_enabled" | "sync_flow" | "sync_started" | "sync_completed" | "sync_failed" => {
            if let Some(value) = map.get_mut("sync_status") {
                *value = json!(normalize_sync_status(value.as_str().unwrap_or("")));
            }
        }
        "pipeline_stage_started" | "pipeline_stage_completed" => {
            if let Some(value) = map.get_mut("pipeline_stage") {
                *value = json!(normalize_stage(value.as_str().unwrap_or("")));
            }
            if event == "pipeline_stage_completed" {
                if let Some(value) = map.get_mut("provider") {
                    *value = json!(normalize_provider(value.as_str().unwrap_or("")));
                }
                if let Some(value) = map.get_mut("model") {
                    *value = json!(normalize_model_family(value.as_str().unwrap_or("")));
                }
            }
        }
        "pipeline_failed" => {
            if let Some(value) = map.get_mut("stage") {
                *value = json!(normalize_stage(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("category") {
                *value = json!(normalize_failure_category(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("provider") {
                *value = json!(normalize_provider(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("model") {
                *value = json!(normalize_model_family(value.as_str().unwrap_or("")));
            }
        }
        "context_match" => {
            if let Some(value) = map.get_mut("match_result") {
                *value = json!(normalize_match_result(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("match_source") {
                *value = json!(normalize_match_source(value.as_str().unwrap_or("")));
            }
        }
        "dictation_outcome" => {
            if let Some(value) = map.get_mut("outcome") {
                *value = json!(normalize_outcome(value.as_str().unwrap_or("")));
            }
            for key in ["status", "reason", "context_result"] {
                if let Some(value) = map.get_mut(key) {
                    *value = json!(match key {
                        "status" => normalize_outcome_status(value.as_str().unwrap_or("")),
                        "reason" => normalize_outcome_reason(value.as_str().unwrap_or("")),
                        _ => normalize_context_result(value.as_str().unwrap_or("")),
                    });
                }
            }
            for key in ["transcription_provider", "cleanup_provider"] {
                if let Some(value) = map.get_mut(key) {
                    *value = json!(normalize_provider(value.as_str().unwrap_or("")));
                }
            }
            for key in ["transcription_model", "cleanup_model"] {
                if let Some(value) = map.get_mut(key) {
                    *value = json!(normalize_model_family(value.as_str().unwrap_or("")));
                }
            }
            if let Some(value) = map.get_mut("recording_duration_bucket") {
                *value = json!(normalize_duration_bucket(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("retry_attempt_bucket") {
                *value = json!(normalize_attempt_bucket(value.as_str().unwrap_or("")));
            }
            for key in ["total_duration_ms", "recording_duration_ms"] {
                if let Some(value) = map.get_mut(key) {
                    *value = json!(value.as_u64().unwrap_or(0).min(900_000));
                }
            }
            if let Some(value) = map.get_mut("word_count") {
                *value = json!(value.as_i64().unwrap_or(0).clamp(0, 10_000));
            }
        }
        "dictation_inserted" => {
            if let Some(value) = map.get_mut("delivery_method") {
                *value = json!(normalize_delivery_method(value.as_str().unwrap_or("")));
            }
            if let Some(value) = map.get_mut("word_count") {
                *value = json!(value.as_i64().unwrap_or(0).clamp(0, 10_000));
            }
        }
        "usage_milestone_reached" => {
            if let Some(value) = map.get_mut("milestone") {
                *value = json!(normalize_milestone(value.as_str().unwrap_or("")));
            }
        }
        _ => {}
    }
    Value::Object(map)
}

fn read_or_create_install_id(path: &std::path::Path) -> Option<String> {
    if let Ok(contents) = std::fs::read_to_string(path) {
        let id = contents.trim();
        if Uuid::parse_str(id).is_ok() {
            return Some(id.to_owned());
        }
    }
    let id = Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(id.as_bytes()).ok()?;
            Some(id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::read_to_string(path).ok().and_then(|contents| {
                let id = contents.trim();
                Uuid::parse_str(id).ok().map(|_| id.to_owned())
            })
        }
        Err(_) => None,
    }
}

fn persist_install_id(path: &std::path::Path, id: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, id);
}

fn read_or_create_first_seen_version(path: &std::path::Path) -> Option<String> {
    if let Ok(contents) = std::fs::read_to_string(path) {
        let version = contents.trim();
        if is_official_version(version) {
            return Some(version.to_owned());
        }
    }
    let version = env!("CARGO_PKG_VERSION").to_owned();
    persist_first_seen_version(path, &version);
    Some(version)
}

fn persist_first_seen_version(path: &std::path::Path, version: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, version);
}

fn read_milestone_state(path: &std::path::Path) -> MilestoneState {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return MilestoneState::default();
    };
    let mut lines = contents.lines();
    let successful_dictations = lines
        .next()
        .and_then(|line| line.parse::<u32>().ok())
        .unwrap_or(0)
        .min(100_000);
    let reached = lines
        .filter(|line| matches!(*line, "dictations_5" | "dictations_10" | "dictations_25"))
        .map(str::to_owned)
        .collect();
    MilestoneState {
        successful_dictations,
        reached,
    }
}

fn persist_milestone_state(path: &std::path::Path, state: &MilestoneState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut contents = state.successful_dictations.to_string();
    for milestone in ["dictations_5", "dictations_10", "dictations_25"] {
        if state.reached.contains(milestone) {
            contents.push('\n');
            contents.push_str(milestone);
        }
    }
    let _ = std::fs::write(path, contents);
}

fn add_model_properties(properties: &mut Value, provider_model: Option<&str>) {
    let Some((provider, model)) = provider_model.and_then(parse_safe_provider_model) else {
        return;
    };
    if let Value::Object(map) = properties {
        map.insert("provider".to_owned(), json!(provider));
        map.insert("model".to_owned(), json!(model));
    }
}

fn parse_safe_provider_model(value: &str) -> Option<(&'static str, &'static str)> {
    // Pipeline strings may contain a primary/secondary description. Only the
    // first configured provider/model is used, and only its fixed families
    // leave the process.
    let first = value.split(';').next()?.trim();
    let first = first
        .split_once('=')
        .map_or(first, |(_, value)| value.trim());
    let (provider, model) = first.split_once('/')?;
    let provider = normalize_provider(provider);
    (provider != "unknown").then_some((provider, normalize_model_family(model)))
}

fn normalize_provider(value: &str) -> &'static str {
    match value {
        "groq" => "groq",
        "openai" => "openai",
        "google" => "google",
        "assemblyai" => "assemblyai",
        "local" => "local",
        _ => "unknown",
    }
}

fn normalize_model_family(value: &str) -> &'static str {
    let value = value.to_ascii_lowercase();
    if value.starts_with("whisper-") || value.contains("whisper") {
        match value.as_str() {
            value if value.contains("groq") => "groq_whisper",
            _ => "transcription_whisper",
        }
    } else if value.contains("gemini") {
        "google_gemini"
    } else if value.contains("qwen") {
        "groq_qwen"
    } else if value.contains("gpt") || value.contains("chat") {
        "chat_model"
    } else if value.contains("universal") {
        "assemblyai_universal"
    } else if value.contains("parakeet") || value.contains("local") {
        "local_catalog"
    } else {
        "unknown"
    }
}

fn outcome_status(outcome: &str) -> &'static str {
    match outcome {
        "success_clean"
        | "success_after_retry"
        | "success_after_transcription_fallback"
        | "success_after_cleanup_fallback"
        | "success_after_clipboard_fallback" => "success",
        "rejected_expected" => "rejected",
        "cancelled_user" => "cancelled",
        "failure_terminal" => "failure",
        _ => "unknown",
    }
}

fn outcome_reason(outcome: &str, last_failure: &str) -> &'static str {
    match outcome {
        "success_clean" => "clean",
        "success_after_retry" => "retry_recovered",
        "success_after_transcription_fallback" => "transcription_fallback_recovered",
        "success_after_cleanup_fallback" => "cleanup_fallback_recovered",
        "success_after_clipboard_fallback" => "clipboard_fallback_recovered",
        "rejected_expected" => normalize_failure_category(last_failure),
        "cancelled_user" => "user_cancelled",
        "failure_terminal" => normalize_failure_category(last_failure),
        _ => "unknown",
    }
}

fn normalize_outcome_status(value: &str) -> &'static str {
    match value {
        "success" => "success",
        "rejected" => "rejected",
        "cancelled" => "cancelled",
        "failure" => "failure",
        _ => "unknown",
    }
}

fn normalize_outcome_reason(value: &str) -> &'static str {
    match value {
        "clean" => "clean",
        "retry_recovered" => "retry_recovered",
        "transcription_fallback_recovered" => "transcription_fallback_recovered",
        "cleanup_fallback_recovered" => "cleanup_fallback_recovered",
        "clipboard_fallback_recovered" => "clipboard_fallback_recovered",
        "user_cancelled" => "user_cancelled",
        _ => normalize_failure_category(value),
    }
}

fn normalize_context_result(value: &str) -> &'static str {
    match value {
        "matched" => "matched",
        "no_match" => "no_match",
        "manual" => "manual",
        _ => "unknown",
    }
}

fn normalize_category(value: &str) -> &'static str {
    match value {
        "groq" | "openai" | "google" | "assemblyai" => "cloud",
        "local" => "local",
        "none" => "none",
        "light" => "light",
        "medium" => "medium",
        "high" => "high",
        "7 days" | "30 days" | "90 days" | "Forever" => match value {
            "7 days" => "7_days",
            "30 days" => "30_days",
            "90 days" => "90_days",
            _ => "forever",
        },
        "keep_loaded" => "keep_loaded",
        "unload_after_5m" => "unload_after_5m",
        "unload_after_15m" => "unload_after_15m",
        "unload_immediately" => "unload_immediately",
        _ => "unknown",
    }
}

fn duration_bucket(ms: u64) -> &'static str {
    match ms {
        0..=999 => "<1s",
        1000..=4999 => "1-5s",
        5000..=14999 => "5-15s",
        15000..=59999 => "15-60s",
        _ => "60s+",
    }
}

fn normalize_feature(value: &str) -> &'static str {
    match value {
        "dual_transcription" => "dual_transcription",
        "cleanup" => "cleanup",
        "transcription_fallback" => "transcription_fallback",
        "cleanup_fallback" => "cleanup_fallback",
        "local_transcription" => "local_transcription",
        "local_cleanup" => "local_cleanup",
        "noise_reduction" => "noise_reduction",
        "contextual_formatting" => "contextual_formatting",
        "auto_learn" => "auto_learn",
        "media_pause" => "media_pause",
        "hands_free" => "hands_free",
        "context_match" => "context_match",
        "retry" => "retry",
        "clipboard_fallback" => "clipboard_fallback",
        _ => "unknown",
    }
}

fn normalize_fallback(value: &str) -> &'static str {
    match value {
        "transcription" => "transcription",
        "cleanup" => "cleanup",
        "clipboard" => "clipboard",
        _ => "unknown",
    }
}
fn normalize_failure_reason(value: &str) -> &'static str {
    match value {
        "provider_failure" => "provider_failure",
        "timeout" => "timeout",
        "network" => "network",
        "empty_response" => "empty_response",
        "user_requested" => "user_requested",
        _ => "unknown",
    }
}
fn normalize_permission(value: &str) -> &'static str {
    match value {
        "microphone" => "microphone",
        "accessibility" => "accessibility",
        "notifications" => "notifications",
        "battery" => "battery",
        _ => "unknown",
    }
}
fn normalize_permission_status(value: &str) -> &'static str {
    match value {
        "missing" => "missing",
        "request_shown" => "request_shown",
        "granted" => "granted",
        "denied" => "denied",
        "settings_opened" => "settings_opened",
        "recovered" => "recovered",
        "still_missing" => "still_missing",
        "abandoned" => "abandoned",
        _ => "unknown",
    }
}
fn normalize_input_outcome(value: &str) -> &'static str {
    match value {
        "microphone_available" => "microphone_available",
        "no_input_device" => "no_input_device",
        "microphone_permission_missing" => "microphone_permission_missing",
        "capture_initialized" => "capture_initialized",
        "capture_initialization_failed" => "capture_initialization_failed",
        "zero_audio_detected" => "zero_audio_detected",
        "too_quiet" => "too_quiet",
        "too_short" => "too_short",
        "vad_passed" => "vad_passed",
        "vad_rejected" => "vad_rejected",
        "vad_internal_failure" => "vad_internal_failure",
        "capture_stream_failed" => "capture_stream_failed",
        _ => "unknown",
    }
}

fn normalize_stage(value: &str) -> &'static str {
    match value {
        "permission" => "permission",
        "capture" => "capture",
        "vad" => "vad",
        "preprocessing" => "preprocessing",
        "transcription" => "transcription",
        "dual_transcription" => "dual_transcription",
        "cleanup" => "cleanup",
        "formatting" => "formatting",
        "insertion" => "insertion",
        "clipboard" => "clipboard",
        "local_model" => "local_model",
        "sync" => "sync",
        _ => "unknown",
    }
}

fn normalize_failure_category(value: &str) -> &'static str {
    match value {
        "permission_missing" => "permission_missing",
        "permission_denied" => "permission_denied",
        "network" => "network",
        "timeout" => "timeout",
        "provider_unavailable" => "provider_unavailable",
        "empty_response" => "empty_response",
        "audio_empty" => "audio_empty",
        "audio_too_short" => "audio_too_short",
        "audio_too_quiet" => "audio_too_quiet",
        "vad_rejected" => "vad_rejected",
        "model_unavailable" => "model_unavailable",
        "local_model_failure" => "local_model_failure",
        "insertion_unavailable" => "insertion_unavailable",
        "insertion_failed" => "insertion_failed",
        "cancelled" => "cancelled",
        "internal" => "internal",
        _ => "unknown",
    }
}

fn normalize_outcome(value: &str) -> &'static str {
    match value {
        "success_clean" => "success_clean",
        "success_after_retry" => "success_after_retry",
        "success_after_transcription_fallback" => "success_after_transcription_fallback",
        "success_after_cleanup_fallback" => "success_after_cleanup_fallback",
        "success_after_clipboard_fallback" => "success_after_clipboard_fallback",
        "rejected_expected" => "rejected_expected",
        "cancelled_user" => "cancelled_user",
        "failure_terminal" => "failure_terminal",
        _ => "unknown",
    }
}

fn normalize_delivery_method(value: &str) -> &'static str {
    match value {
        "direct_insertion" => "direct_insertion",
        "clipboard_fallback" => "clipboard_fallback",
        "event_only" => "event_only",
        _ => "unknown",
    }
}

fn normalize_match_result(value: &str) -> &'static str {
    match value {
        "matched" => "matched",
        "no_match" => "no_match",
        _ => "unknown",
    }
}

fn normalize_match_source(value: &str) -> &'static str {
    match value {
        "automatic" => "automatic",
        "manual" => "manual",
        _ => "unknown",
    }
}

fn normalize_milestone(value: &str) -> &'static str {
    match value {
        "dictations_5" => "dictations_5",
        "dictations_10" => "dictations_10",
        "dictations_25" => "dictations_25",
        _ => "unknown",
    }
}

fn normalize_attempt_bucket(value: &str) -> &'static str {
    match value {
        "1" => "1",
        "2" => "2",
        "3" => "3",
        "4+" => "4+",
        _ => "unknown",
    }
}

fn normalize_setup_step(value: &str) -> &'static str {
    match value {
        "intro" => "intro",
        "analytics" => "analytics",
        "provider" => "provider",
        "api_key" => "api_key",
        "permissions" => "permissions",
        "models" => "models",
        "writing_style" => "writing_style",
        "language" => "language",
        "audio_environment" => "audio_environment",
        "audio" => "audio",
        "try_it" => "try_it",
        "complete" => "complete",
        "done" => "done",
        _ => "unknown",
    }
}

fn normalize_duration_bucket(value: &str) -> &'static str {
    match value {
        "<1s" => "<1s",
        "1-5s" => "1-5s",
        "5-15s" => "5-15s",
        "15-60s" => "15-60s",
        "60s+" => "60s+",
        _ => "unknown",
    }
}

fn normalize_setting(value: &str) -> &'static str {
    match value {
        "cleanup_enabled" => "cleanup_enabled",
        "dual_transcription_enabled" => "dual_transcription_enabled",
        "noise_reduction" => "noise_reduction",
        "auto_learn_enabled" => "auto_learn_enabled",
        "contextual_formatting" => "contextual_formatting",
        "pause_media" => "pause_media",
        "transcription_provider" => "transcription_provider",
        "cleanup_provider" => "cleanup_provider",
        "cleanup_intensity" => "cleanup_intensity",
        "history_retention" => "history_retention",
        "local_model_memory_policy" => "local_model_memory_policy",
        "mic_mute_button_dictation" => "mic_mute_button_dictation",
        "sync_enabled" => "sync_enabled",
        _ => "unknown",
    }
}

fn safe_setting_value(value: &Value) -> Value {
    match value {
        Value::Bool(value) => json!(value),
        Value::String(value) => json!(normalize_category(value)),
        _ => json!("unknown"),
    }
}
fn normalize_sync_status(value: &str) -> &'static str {
    match value {
        "enabled" => "enabled",
        "started" => "started",
        "completed" => "completed",
        "failed" => "failed",
        "pairing_started" => "pairing_started",
        "pairing_completed" => "pairing_completed",
        "pairing_failed" => "pairing_failed",
        "conflict" => "conflict",
        _ => "unknown",
    }
}

fn normalize_error_code(value: &str) -> &'static str {
    match value {
        "frontend_unhandled" => "frontend_unhandled",
        "frontend_handled" => "frontend_handled",
        "backend_panic" => "backend_panic",
        "transcription_failed" => "transcription_failed",
        "cleanup_failed" => "cleanup_failed",
        "insertion_failed" => "insertion_failed",
        "sync_transport_failed" => "sync_transport_failed",
        "update_check_failed" => "update_check_failed",
        "local_model_failed" => "local_model_failed",
        "capture_failed" => "capture_failed",
        "database_operation_failed" => "database_operation_failed",
        _ => "unknown_error",
    }
}

fn normalize_error_callsite(value: &str) -> &'static str {
    match value {
        "frontend_window" => "frontend_window",
        "frontend_boundary" => "frontend_boundary",
        "panic_hook" => "panic_hook",
        "pipeline_transcription" => "pipeline_transcription",
        "pipeline_cleanup" => "pipeline_cleanup",
        "pipeline_insertion" => "pipeline_insertion",
        "sync_command" => "sync_command",
        "updater" => "updater",
        _ => "unknown_callsite",
    }
}

fn normalize_recovery_method(value: &str) -> &'static str {
    match value {
        "retry" => "retry",
        "fallback" => "fallback",
        "clipboard" => "clipboard",
        "none" => "none",
        _ => "unknown",
    }
}

fn safe_run_id(run_id: Option<String>) -> Option<String> {
    run_id.filter(|value| Uuid::parse_str(value).is_ok())
}
fn attempt_bucket(attempt: u8) -> &'static str {
    match attempt {
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "4+",
    }
}
fn is_official_version(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.clone().count() == 3
        && parts.all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn buckets_are_bounded() {
        assert_eq!(duration_bucket(4_000), "1-5s");
        assert_eq!(duration_bucket(99_999), "60s+");
    }
    #[test]
    fn taxonomy_is_bounded() {
        assert_eq!(Stage::Transcription.as_str(), "transcription");
        assert_eq!(FailureCategory::Unknown.as_str(), "unknown");
    }
    #[test]
    fn identity_is_opaque_and_changes_on_toggle() {
        let path = std::env::temp_dir().join(format!("verenu-analytics-test-{}", Uuid::new_v4()));
        let a = Analytics::new(true, path.clone());
        let first = a.install_id.lock().unwrap().clone();
        let first_seen = a.first_seen_version.lock().unwrap().clone();
        a.set_enabled(true);
        assert_eq!(first, a.install_id.lock().unwrap().clone());
        let reopened = Analytics::new(true, path.clone());
        assert_eq!(first, reopened.install_id.lock().unwrap().clone());
        assert_eq!(
            first_seen,
            reopened.first_seen_version.lock().unwrap().clone()
        );
        assert!(first
            .as_deref()
            .is_some_and(|value| Uuid::parse_str(value).is_ok()));
        assert!(first_seen.as_deref().is_some_and(is_official_version));
        a.set_enabled(false);
        assert!(!path.join("analytics_install_id").exists());
        assert!(!path.join("analytics_first_seen_version").exists());
        a.set_enabled(true);
        assert_ne!(first, a.install_id.lock().unwrap().clone());
        assert_eq!(
            a.first_seen_version.lock().unwrap().as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn identity_metadata_is_added_without_repeating_install_id() {
        let payload = add_common_properties(serde_json::Map::new(), "session", Some("0.18.1"));
        assert_eq!(payload["identity_schema_version"], IDENTITY_SCHEMA_VERSION);
        assert_eq!(payload["first_seen_version"], "0.18.1");
        assert!(payload.get("analytics_install_id").is_none());
    }
    #[test]
    fn unknown_values_collapse_to_bounded_categories() {
        assert_eq!(normalize_feature("private_feature"), "unknown");
        assert_eq!(normalize_fallback("custom-endpoint"), "unknown");
        assert_eq!(normalize_permission("camera"), "unknown");
        assert_eq!(normalize_sync_status("peer-name"), "unknown");
    }

    #[test]
    fn new_dimensions_are_bounded_and_content_free() {
        let run_id = Uuid::new_v4().to_string();
        let context = safe_properties(
            "context_match",
            json!({
                "run_id": run_id,
                "match_result": "matched",
                "match_source": "automatic",
                "context_name": "PRIVATE_CONTEXT_SENTINEL",
            }),
        );
        assert_eq!(context["match_result"], "matched");
        assert_eq!(context["match_source"], "automatic");
        assert!(context.get("context_name").is_none());

        let inserted = safe_properties(
            "dictation_inserted",
            json!({
                "run_id": Uuid::new_v4().to_string(),
                "delivery_method": "direct_insertion",
                "word_count": 99_999,
                "transcript": "PRIVATE_TRANSCRIPT_SENTINEL",
            }),
        );
        assert_eq!(inserted["word_count"], 10_000);
        assert!(inserted.get("transcript").is_none());

        let stage = safe_properties(
            "pipeline_stage_completed",
            json!({
                "run_id": Uuid::new_v4().to_string(),
                "pipeline_stage": "transcription",
                "provider": "private-provider",
                "model": "private-model",
            }),
        );
        assert_eq!(stage["provider"], "unknown");
        assert_eq!(stage["model"], "unknown");
    }

    #[test]
    fn terminal_outcome_is_rich_and_exactly_once() {
        let path =
            std::env::temp_dir().join(format!("verenu-analytics-outcome-{}", Uuid::new_v4()));
        let analytics = Analytics::new(false, path.clone());
        let run_id = analytics.new_run_id();
        analytics.dictation_started(&run_id, false, false);
        analytics.recording_finished(&run_id, 4_000);
        analytics.context_outcome(&run_id, true, "automatic");
        analytics.retry_attempted(&run_id, 2, "timeout");
        analytics.fallback_used(&run_id, "transcription");
        analytics.dictation_inserted(&run_id, "direct_insertion", 42);
        analytics.delivery_outcome(&run_id, "success_clean");
        analytics.delivery_outcome(&run_id, "failure_terminal");

        let properties = safe_properties(
            "dictation_outcome",
            json!({
                "run_id": run_id,
                "outcome": "success_after_transcription_fallback",
                "status": "success",
                "reason": "transcription_fallback_recovered",
                "total_duration_ms": 900_001_u64,
                "recording_duration_ms": 4_000_u64,
                "recording_duration_bucket": "1-5s",
                "word_count": 42,
                "transcription_provider": "groq",
                "transcription_model": "transcription_whisper",
                "cleanup_provider": "unknown",
                "cleanup_model": "unknown",
                "context_result": "matched",
                "retry_attempt_bucket": "2",
                "transcription_fallback_used": true,
                "cleanup_fallback_used": false,
                "clipboard_fallback_used": false,
                "recovered": true,
                "transcript": "PRIVATE_TRANSCRIPT_SENTINEL",
            }),
        );
        assert_eq!(properties["word_count"], 42);
        assert_eq!(properties["total_duration_ms"], 900_000);
        assert!(properties.get("transcript").is_none());
        assert_eq!(
            analytics
                .sent_once
                .lock()
                .unwrap()
                .iter()
                .filter(|key| key.as_str() == format!("dictation_outcome:{run_id}"))
                .count(),
            1
        );
        assert!(analytics.dictation_runs.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn provider_model_metadata_is_reduced_to_fixed_families() {
        assert_eq!(
            parse_safe_provider_model("primary=groq/whisper-large-v3;secondary=openai/x"),
            Some(("groq", "transcription_whisper"))
        );
        assert_eq!(
            parse_safe_provider_model("google/gemini-2.0-flash"),
            Some(("google", "google_gemini"))
        );
        assert_eq!(
            parse_safe_provider_model("https://private.example/model"),
            None
        );
    }

    #[test]
    fn usage_milestones_persist_and_are_emitted_only_at_thresholds() {
        let path =
            std::env::temp_dir().join(format!("verenu-analytics-milestones-{}", Uuid::new_v4()));
        let analytics = Analytics::new(true, path.clone());
        let run_id = Uuid::new_v4().to_string();
        for _ in 0..5 {
            analytics.dictation_inserted(&run_id, "direct_insertion", 3);
        }
        let state = read_milestone_state(&path.join("analytics_usage_milestones"));
        assert_eq!(state.successful_dictations, 5);
        assert!(state.reached.contains("dictations_5"));
        let reopened = Analytics::new(true, path.clone());
        assert_eq!(
            reopened
                .milestone_state
                .lock()
                .unwrap()
                .successful_dictations,
            5
        );
        analytics.set_enabled(false);
        assert!(!path.join("analytics_usage_milestones").exists());
        let _ = std::fs::remove_dir_all(path);
    }
    #[test]
    fn property_allowlist_drops_content_and_identifiers() {
        let value = safe_properties(
            "feature_used",
            json!({
                "run_id": "opaque",
                "feature": "cleanup",
                "transcript": "secret dictated text",
                "api_key": "fake-key",
                "window_title": "private window",
                "file_path": "C:/private/file",
            }),
        );
        assert_eq!(value["feature"], "cleanup");
        assert!(value.get("transcript").is_none());
        assert!(value.get("api_key").is_none());
        assert!(value.get("window_title").is_none());
        assert!(value.get("file_path").is_none());
    }

    #[test]
    fn final_exception_payload_rejects_sensitive_sentinels() {
        let sentinels = [
            "DICTATED_TRANSCRIPT_SENTINEL",
            "Bearer API_KEY_SENTINEL",
            r"C:\\Users\\Private\\secret.wav",
            "/Users/private/secret.wav",
            "https://private.example/path?token=URL_SENTINEL",
            "Microphone Name Sentinel",
            "com.private.application",
            "Context Name Sentinel",
            "Clipboard Sentinel",
            "192.168.55.23",
            "PAIRING-UUID-SENTINEL",
        ];
        let properties = exception_properties(ErrorReport {
            domain: ErrorDomain::Transcription,
            code: "DICTATED_TRANSCRIPT_SENTINEL",
            stage: Some(Stage::Transcription),
            severity: ErrorSeverity::Error,
            handled: true,
            recovered: false,
            recovery_method: Some("https://private.example/path?token=URL_SENTINEL"),
            run_id: Some("DICTATED_TRANSCRIPT_SENTINEL".to_owned()),
            callsite: r"C:\\Users\\Private\\secret.wav",
        });
        let payload = error_tracking_payload(
            "public-ingestion-token",
            "$exception",
            "install-id",
            add_common_properties(
                properties.as_object().unwrap().clone(),
                "session-id",
                Some("0.18.1"),
            ),
        );
        let serialized = serde_json::to_string(&payload).unwrap();
        for sentinel in sentinels {
            assert!(
                !serialized.contains(sentinel),
                "sensitive sentinel escaped exception payload: {sentinel}"
            );
        }
        assert!(serialized.contains("unknown_error"));
        assert!(serialized.contains("unknown_callsite"));
    }

    #[test]
    fn generic_exception_capture_has_no_allowlisted_properties() {
        let value = safe_properties(
            "$exception",
            json!({
                "$exception_list": "DICTATED_TRANSCRIPT_SENTINEL",
                "error": "Bearer API_KEY_SENTINEL",
            }),
        );
        assert_eq!(value, json!({}));
    }

    #[test]
    fn boundary_normalizes_unknown_event_values() {
        let outcome = safe_properties(
            "dictation_outcome",
            json!({"run_id": Uuid::new_v4().to_string(), "outcome": "private transcript"}),
        );
        assert_eq!(outcome["outcome"], "unknown");

        let setting = safe_properties(
            "setting_changed",
            json!({"setting": "private_setting", "value": "secret text"}),
        );
        assert_eq!(setting["setting"], "unknown");
        assert_eq!(setting["value"], "unknown");

        let setup = safe_properties(
            "setup_step_viewed",
            json!({"setup_step": "private form contents"}),
        );
        assert_eq!(setup["setup_step"], "unknown");
    }

    #[test]
    fn recovery_flags_classify_one_final_outcome() {
        let path =
            std::env::temp_dir().join(format!("verenu-analytics-outcome-{}", Uuid::new_v4()));
        let analytics = Analytics::new(false, path.clone());
        let run_id = Uuid::new_v4().to_string();
        analytics.fallback_used(&run_id, "transcription");
        assert_eq!(
            analytics.resolved_outcome(&run_id, "success_clean"),
            "success_after_transcription_fallback"
        );
        analytics.retry_attempted(&run_id, 2, "timeout");
        assert_eq!(
            analytics.resolved_outcome(&run_id, "success_clean"),
            "success_after_transcription_fallback"
        );
        analytics.delivery_outcome(&run_id, "success_clean");
        assert!(!analytics
            .recovery_flags
            .lock()
            .unwrap()
            .iter()
            .any(|flag| flag.starts_with(&format!("{run_id}:"))));
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn outbound_payload_carries_ip_and_person_profile_processing_controls() {
        let final_payload = error_tracking_payload(
            "token",
            "app_launched",
            "install",
            add_common_properties(serde_json::Map::new(), "session", None),
        );
        assert_eq!(final_payload["properties"]["$ip"], "0.0.0.0");
        assert_eq!(final_payload["properties"]["$geoip_disable"], true);
        assert_eq!(
            final_payload["properties"]["$process_person_profile"],
            false
        );
    }

    #[test]
    fn final_payload_cannot_carry_geoip_or_raw_ip_fields() {
        let safe = safe_properties(
            "app_launched",
            json!({
                "first_launch": true,
                "$ip": "203.0.113.42",
                "$geoip_city_name": "GeoIP city sentinel",
                "$geoip_postal_code": "SENTINEL-POSTAL",
                "$geoip_latitude": 49.0,
                "$geoip_longitude": -123.0,
                "$geoip_subdivision_1_name": "Sentinel subdivision",
                "$geoip_accuracy_radius": 5,
                "$geoip_time_zone": "America/Vancouver",
            }));
        let payload = outbound_payload(
            "token",
            "app_launched",
            "install",
            add_common_properties(safe.as_object().unwrap().clone(), "session", None),
        );
        let properties = payload["properties"].as_object().unwrap();

        // These are forbidden both as caller-provided properties and as
        // PostHog-derived metadata. The only IP control present is the
        // deliberate non-routable placeholder plus the explicit opt-out.
        for key in [
            "$geoip_city_name",
            "$geoip_postal_code",
            "$geoip_latitude",
            "$geoip_longitude",
            "$geoip_subdivision_1_name",
            "$geoip_accuracy_radius",
            "$geoip_time_zone",
        ] {
            assert!(properties.get(key).is_none(), "unexpected field: {key}");
        }
        assert_eq!(properties.get("$ip"), Some(&json!("0.0.0.0")));
        assert_eq!(properties.get("$geoip_disable"), Some(&json!(true)));
        assert!(!serde_json::to_string(&payload)
            .unwrap()
            .contains("203.0.113.42"));
    }
}
