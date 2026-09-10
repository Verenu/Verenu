//! Bounded, metadata-first diagnostics state for the native application.
//!
//! This module is intentionally independent of Tauri, the logger, and the
//! pipeline. Callers can feed it safe fields from those systems without
//! making diagnostics a second logging framework. The normal integration
//! points are:
//!
//! * call [`init`] during native startup;
//! * call [`record_log`] beside ordinary log calls when a structured record is
//!   useful;
//! * wrap a dictation with [`start_trace`], [`start_span`], and
//!   [`finish_span`];
//! * wrap `commands::run_blocking` with [`start_operation`] and
//!   [`finish_operation`];
//! * call [`set_profiler_enabled`] when the Diagnostics view becomes visible
//!   or an explicit recording starts, then feed sampled resources through
//!   [`record_resource_sample`];
//! * expose [`snapshot`] to the frontend or an export command.
//!
//! All text fields are labels or short, redacted summaries. This module does
//! not define fields for transcripts, prompts, clipboard contents, payloads,
//! API keys, or active-field text. Callers should still pass metadata-first
//! messages. The small redaction pass below is a second line of defence for
//! accidental sensitive markers.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const DEFAULT_MAX_LOGS: usize = 4_000;
const DEFAULT_MAX_FAILURES: usize = 256;
const DEFAULT_MAX_FAILURE_GROUPS: usize = 128;
const DEFAULT_MAX_TRACES: usize = 64;
const DEFAULT_MAX_SPANS_PER_TRACE: usize = 96;
const DEFAULT_MAX_OPERATIONS: usize = 256;
const DEFAULT_MAX_DURATION_SAMPLES: usize = 128;
const DEFAULT_MAX_RESOURCE_SAMPLES: usize = 240;
const DEFAULT_MAX_CHILD_PROCESSES: usize = 16;
const DEFAULT_MAX_MEASUREMENTS: usize = 16;
const DEFAULT_MAX_FLAGS: usize = 16;

const MAX_LABEL_CHARS: usize = 96;
const MAX_MESSAGE_CHARS: usize = 512;
const MAX_CAUSE_CHARS: usize = 512;
const MAX_FINGERPRINT_INPUT_CHARS: usize = 384;
const ROLLING_CALL_WINDOW_MS: u64 = 60_000;

static STORE: OnceLock<Mutex<DiagnosticsStore>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Retention limits for all in-memory diagnostics histories.
///
/// Values are clamped to at least one entry. A small configuration is useful
/// in deterministic tests and in constrained diagnostic builds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    pub max_logs: usize,
    pub max_failures: usize,
    pub max_failure_groups: usize,
    pub max_traces: usize,
    pub max_spans_per_trace: usize,
    pub max_operations: usize,
    pub max_duration_samples: usize,
    pub max_resource_samples: usize,
    pub max_child_processes: usize,
    pub max_measurements: usize,
    pub max_flags: usize,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            max_logs: DEFAULT_MAX_LOGS,
            max_failures: DEFAULT_MAX_FAILURES,
            max_failure_groups: DEFAULT_MAX_FAILURE_GROUPS,
            max_traces: DEFAULT_MAX_TRACES,
            max_spans_per_trace: DEFAULT_MAX_SPANS_PER_TRACE,
            max_operations: DEFAULT_MAX_OPERATIONS,
            max_duration_samples: DEFAULT_MAX_DURATION_SAMPLES,
            max_resource_samples: DEFAULT_MAX_RESOURCE_SAMPLES,
            max_child_processes: DEFAULT_MAX_CHILD_PROCESSES,
            max_measurements: DEFAULT_MAX_MEASUREMENTS,
            max_flags: DEFAULT_MAX_FLAGS,
        }
    }
}

impl DiagnosticsConfig {
    fn bounded(&self) -> Self {
        Self {
            max_logs: self.max_logs.max(1),
            max_failures: self.max_failures.max(1),
            max_failure_groups: self.max_failure_groups.max(1),
            max_traces: self.max_traces.max(1),
            max_spans_per_trace: self.max_spans_per_trace.max(1),
            max_operations: self.max_operations.max(1),
            max_duration_samples: self.max_duration_samples.max(1),
            max_resource_samples: self.max_resource_samples.max(1),
            max_child_processes: self.max_child_processes.max(1),
            max_measurements: self.max_measurements.max(1),
            max_flags: self.max_flags.max(1),
        }
    }
}

/// Log severity. This is separate from `log::Level` so the diagnostics model
/// stays easy to serialize and test without installing a global logger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// The terminal state of a span, trace, or operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OperationOutcome {
    Running,
    Success,
    Failure,
    Cancelled,
    Skipped,
    #[default]
    Unknown,
}

/// Cache state that is safe to show in diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheStatus {
    Hit,
    Miss,
    Bypassed,
    Unknown,
}

/// Optional structured fields shared by logs, failures, and spans.
///
/// Numeric measurements and boolean flags are copied with bounded key counts.
/// Arbitrary JSON is deliberately not accepted here, which prevents a
/// diagnostics call from retaining a provider response or user payload.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StructuredFields {
    pub operation: Option<String>,
    pub stage: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub outcome: Option<OperationOutcome>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub cache: Option<CacheStatus>,
    pub error_category: Option<String>,
    pub error_fingerprint: Option<String>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// A structured, human-readable log record. The message is short and
/// redacted; it is never intended to carry dictated text or a request body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuredLogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub subsystem: String,
    pub operation: Option<String>,
    pub stage: Option<String>,
    pub message: String,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub outcome: Option<OperationOutcome>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub cache: Option<CacheStatus>,
    pub error_category: Option<String>,
    pub error_fingerprint: Option<String>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// Input accepted by [`record_failure`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FailureEventInput {
    pub subsystem: String,
    pub operation: Option<String>,
    pub stage: Option<String>,
    pub cause: String,
    pub duration_ms: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub error_category: Option<String>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// One retained failure occurrence. Repeated occurrences also update a
/// [`FailureGroup`], but each retained event keeps its own trace and timing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailureEvent {
    pub id: String,
    pub timestamp_ms: u64,
    pub subsystem: String,
    pub operation: Option<String>,
    pub stage: Option<String>,
    pub cause: String,
    pub duration_ms: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub fingerprint: String,
    pub error_category: Option<String>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// A normalized failure family. Volatile IDs, timestamps, offsets, and paths
/// do not make a new group, while subsystem, operation, stage, and category
/// remain part of the identity so unrelated failures stay separate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailureGroup {
    pub fingerprint: String,
    pub count: u64,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub subsystem: String,
    pub operation: Option<String>,
    pub stage: Option<String>,
    pub error_category: Option<String>,
    pub representative_cause: String,
    pub last_trace_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// A bounded pipeline timeline. Active spans have no `ended_at_ms` and use
/// `running` as their outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineTrace {
    pub trace_id: String,
    pub session_id: Option<String>,
    pub root_operation: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub outcome: OperationOutcome,
    pub spans: Vec<Span>,
}

/// One pipeline stage. `parent_span_id` and `parallel_group` preserve the
/// shape of concurrent work such as dual transcription.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub span_id: String,
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub parallel_group: Option<String>,
    pub operation: String,
    pub stage: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub outcome: OperationOutcome,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub cache: Option<CacheStatus>,
    pub error_category: Option<String>,
    pub error_fingerprint: Option<String>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// Optional fields applied when a span finishes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpanFinish {
    pub outcome: OperationOutcome,
    pub duration_ms: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub retry_number: Option<u32>,
    pub fallback_number: Option<u32>,
    pub cache: Option<CacheStatus>,
    pub error_category: Option<String>,
    pub error_fingerprint: Option<String>,
    pub measurements: BTreeMap<String, f64>,
    pub flags: BTreeMap<String, bool>,
}

/// Backend command or other operation aggregate. Duration samples are kept
/// in a bounded reservoir and are used for an exact p95 over that bounded
/// recent sample set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationMetric {
    pub operation: String,
    pub calls: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub cancelled_count: u64,
    pub skipped_count: u64,
    pub total_duration_ms: u64,
    pub average_duration_ms: Option<f64>,
    pub recent_duration_ms: Option<u64>,
    pub p95_duration_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub calls_per_minute: u64,
    pub currently_running: u64,
    pub last_called_at_ms: Option<u64>,
}

/// One point in the optional resource timeline. Unknown or unsupported
/// platform values stay `None` and serialize as `null`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub observed_at_ms: u64,
    pub cpu_percent: Option<f64>,
    pub resident_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub peak_resident_bytes: Option<u64>,
    pub process_count: Option<u32>,
    pub child_processes: Vec<ChildProcessSnapshot>,
    pub read_bytes_total: Option<u64>,
    pub write_bytes_total: Option<u64>,
    pub read_bytes_per_sec: Option<f64>,
    pub write_bytes_per_sec: Option<f64>,
    pub thread_count: Option<u32>,
    pub handle_count: Option<u64>,
    pub gpu_memory_bytes: Option<u64>,
    pub gpu_total_memory_bytes: Option<u64>,
    pub uptime_ms: Option<u64>,
    pub local_stt_state: Option<String>,
    pub local_llm_state: Option<String>,
    pub collector_duration_us: Option<u64>,
}

/// Bounded child-process detail for a resource sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChildProcessSnapshot {
    pub label: String,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub resident_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub read_bytes_total: Option<u64>,
    pub write_bytes_total: Option<u64>,
    pub thread_count: Option<u32>,
}

/// A timestamped resource point retained by the profiler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceSample {
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub snapshot: ResourceSnapshot,
}

/// Cheap health counters for the diagnostics system itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsHealth {
    pub initialized: bool,
    pub profiler_enabled: bool,
    pub profiler_started_at_ms: Option<u64>,
    pub profiler_elapsed_ms: Option<u64>,
    pub retained_log_count: usize,
    pub retained_failure_count: usize,
    pub retained_trace_count: usize,
    pub retained_operation_count: usize,
    pub retained_resource_sample_count: usize,
    pub active_trace_count: usize,
    pub active_span_count: usize,
    pub total_logs_recorded: u64,
    pub total_failures_recorded: u64,
    pub total_traces_started: u64,
    pub total_traces_completed: u64,
    pub total_operations_recorded: u64,
    pub dropped_logs: u64,
    pub dropped_failures: u64,
    pub dropped_traces: u64,
    pub dropped_spans: u64,
    pub dropped_resource_samples: u64,
    pub collector_samples: u64,
    pub collector_duration_us_total: u64,
    pub collector_duration_us_average: Option<f64>,
    pub last_event_at_ms: Option<u64>,
}

/// Serializable view of all bounded diagnostics state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsSnapshot {
    pub generated_at_ms: u64,
    pub profiler_enabled: bool,
    pub profiling_recording: bool,
    pub current_resource: Option<ResourceSnapshot>,
    pub resource_samples: Vec<ResourceSample>,
    pub latest_failures: Vec<FailureEvent>,
    pub failure_groups: Vec<FailureGroup>,
    pub active_pipelines: Vec<PipelineTrace>,
    pub recent_pipelines: Vec<PipelineTrace>,
    pub logs: Vec<StructuredLogEntry>,
    pub operations: Vec<OperationMetric>,
    /// Optional runtime-manager state supplied by the command layer. Kept
    /// outside the diagnostics store so this module remains Tauri-independent.
    pub runtime: DiagnosticsRuntime,
    pub health: DiagnosticsHealth,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsRuntime {
    pub local_stt: Option<serde_json::Value>,
    pub local_llm: Option<serde_json::Value>,
    pub sync: Option<serde_json::Value>,
    pub auto_learn: Option<serde_json::Value>,
    pub cleanup_cache: Option<serde_json::Value>,
    /// Snapshot of the active recording session's atomic audio/VAD state.
    /// This deliberately contains levels and state flags only, never PCM.
    pub audio: Option<serde_json::Value>,
}

/// Opaque handle returned by [`start_trace`]. The value contains no user
/// content and can safely be passed between pipeline tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceHandle {
    pub trace_id: String,
}

/// Opaque handle returned by [`start_span`] or [`start_parallel_span`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanHandle {
    pub trace_id: String,
    pub span_id: String,
}

/// Opaque handle for a timed operation.
#[derive(Debug)]
pub struct OperationHandle {
    operation: String,
    started: Instant,
    tracked: bool,
}

struct SpanState {
    span: Span,
    started: Instant,
}

struct TraceState {
    trace_id: String,
    session_id: Option<String>,
    root_operation: String,
    started_at_ms: u64,
    started: Instant,
    ended_at_ms: Option<u64>,
    duration_ms: Option<u64>,
    outcome: OperationOutcome,
    spans: Vec<SpanState>,
}

struct OperationMetricState {
    metric: OperationMetric,
    durations: VecDeque<u64>,
    call_times: VecDeque<u64>,
}

struct DiagnosticsStore {
    config: DiagnosticsConfig,
    logs: VecDeque<StructuredLogEntry>,
    failures: VecDeque<FailureEvent>,
    failure_groups: Vec<FailureGroup>,
    traces: VecDeque<TraceState>,
    operations: Vec<OperationMetricState>,
    resource_samples: VecDeque<ResourceSample>,
    current_resource: Option<ResourceSnapshot>,
    profiler_enabled: bool,
    profiling_recording: bool,
    profiler_started_at_ms: Option<u64>,
    profiler_started: Option<Instant>,
    peak_resident_bytes: Option<u64>,
    next_resource_sequence: u64,
    total_logs_recorded: u64,
    total_failures_recorded: u64,
    total_traces_started: u64,
    total_traces_completed: u64,
    total_operations_recorded: u64,
    dropped_logs: u64,
    dropped_failures: u64,
    dropped_traces: u64,
    dropped_spans: u64,
    dropped_resource_samples: u64,
    collector_samples: u64,
    collector_duration_us_total: u64,
    last_event_at_ms: Option<u64>,
}

impl DiagnosticsStore {
    fn new(config: DiagnosticsConfig) -> Self {
        Self {
            config: config.bounded(),
            logs: VecDeque::new(),
            failures: VecDeque::new(),
            failure_groups: Vec::new(),
            traces: VecDeque::new(),
            operations: Vec::new(),
            resource_samples: VecDeque::new(),
            current_resource: None,
            profiler_enabled: false,
            profiling_recording: false,
            profiler_started_at_ms: None,
            profiler_started: None,
            peak_resident_bytes: None,
            next_resource_sequence: 0,
            total_logs_recorded: 0,
            total_failures_recorded: 0,
            total_traces_started: 0,
            total_traces_completed: 0,
            total_operations_recorded: 0,
            dropped_logs: 0,
            dropped_failures: 0,
            dropped_traces: 0,
            dropped_spans: 0,
            dropped_resource_samples: 0,
            collector_samples: 0,
            collector_duration_us_total: 0,
            last_event_at_ms: None,
        }
    }

    fn touch(&mut self, at_ms: u64) {
        self.last_event_at_ms = Some(at_ms);
    }
}

fn store() -> &'static Mutex<DiagnosticsStore> {
    STORE.get_or_init(|| Mutex::new(DiagnosticsStore::new(DiagnosticsConfig::default())))
}

fn with_store<T>(f: impl FnOnce(&mut DiagnosticsStore) -> T) -> T {
    let mut guard = store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// Initializes the global store with default bounds. Repeated calls are safe.
pub fn init() {
    let _ = store();
}

/// Initializes the store with explicit bounds. Only the first initialization
/// chooses the configuration. Later calls are harmless no-ops.
#[allow(dead_code)]
pub fn init_with_config(config: DiagnosticsConfig) {
    let _ = STORE.get_or_init(|| Mutex::new(DiagnosticsStore::new(config)));
}

/// Enables or disables the expensive sampling window. Enabling starts a new
/// resource window and clears its previous samples. Disabling stops future
/// samples but keeps the completed window in the snapshot for inspection.
pub fn set_profiler_enabled(enabled: bool) {
    with_store(|state| {
        if state.profiler_enabled == enabled {
            if !enabled && !state.profiling_recording {
                state.profiler_started = None;
            }
            return;
        }
        state.profiler_enabled = enabled;
        if enabled {
            start_resource_window(state);
        } else if !state.profiling_recording {
            state.profiler_started = None;
        }
        state.touch(now_ms());
    });
}

/// Alias suited to UI lifecycle call sites.
#[allow(dead_code)]
pub fn enable_profiler(enabled: bool) {
    set_profiler_enabled(enabled);
}

/// Returns whether the optional resource profiler is currently enabled.
#[allow(dead_code)]
pub fn profiler_enabled() -> bool {
    with_store(|state| state.profiler_enabled)
}

/// Explicit recording keeps the sampler alive even after the Diagnostics view
/// unmounts. This is intentionally separate from view visibility so ordinary
/// navigation cannot leave a process-tree sampler running forever.
pub fn set_profiling_recording(enabled: bool) {
    with_store(|state| {
        state.profiling_recording = enabled;
        if enabled {
            start_resource_window(state);
        } else if !state.profiler_enabled {
            state.profiler_started = None;
        }
        state.touch(now_ms());
    });
}

#[allow(dead_code)]
pub fn profiling_recording() -> bool {
    with_store(|state| state.profiling_recording)
}

pub fn should_sample_resources() -> bool {
    with_store(|state| state.profiler_enabled || state.profiling_recording)
}

/// Records a metadata-first structured log entry and returns the retained
/// sanitized value. The returned value is useful for event emission.
pub fn record_log(level: &str, subsystem: &str, message: &str) -> Option<StructuredLogEntry> {
    Some(record_log_with_fields(
        parse_log_level(level),
        subsystem,
        message,
        StructuredFields::default(),
    ))
}

/// Records a metadata-first structured log entry with optional fields.
pub fn record_log_with_fields(
    level: LogLevel,
    subsystem: &str,
    message: &str,
    fields: StructuredFields,
) -> StructuredLogEntry {
    let entry = StructuredLogEntry {
        timestamp_ms: now_ms(),
        level,
        subsystem: safe_label(subsystem),
        operation: safe_option_label(fields.operation),
        stage: safe_option_label(fields.stage),
        message: redact_and_limit(message, MAX_MESSAGE_CHARS),
        trace_id: safe_option_id(fields.trace_id),
        session_id: safe_option_id(fields.session_id),
        duration_ms: fields.duration_ms,
        outcome: fields.outcome,
        retry_number: fields.retry_number,
        fallback_number: fields.fallback_number,
        cache: fields.cache,
        error_category: safe_option_label(fields.error_category),
        error_fingerprint: safe_option_id(fields.error_fingerprint),
        measurements: bound_measurements(fields.measurements, DEFAULT_MAX_MEASUREMENTS),
        flags: bound_flags(fields.flags, DEFAULT_MAX_FLAGS),
    };
    record_log_entry(entry)
}

fn parse_log_level(level: &str) -> LogLevel {
    match level.to_ascii_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "warn" | "warning" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// Stores an already constructed entry after applying the same bounds and
/// redaction used by [`record_log`]. A zero timestamp is replaced with now.
pub fn record_log_entry(mut entry: StructuredLogEntry) -> StructuredLogEntry {
    entry.timestamp_ms = if entry.timestamp_ms == 0 {
        now_ms()
    } else {
        entry.timestamp_ms
    };
    entry.subsystem = safe_label(&entry.subsystem);
    entry.operation = safe_option_label(entry.operation);
    entry.stage = safe_option_label(entry.stage);
    entry.message = redact_and_limit(&entry.message, MAX_MESSAGE_CHARS);
    entry.trace_id = safe_option_id(entry.trace_id);
    entry.session_id = safe_option_id(entry.session_id);
    entry.error_category = safe_option_label(entry.error_category);
    entry.error_fingerprint = safe_option_id(entry.error_fingerprint);

    with_store(|state| {
        state.total_logs_recorded = state.total_logs_recorded.saturating_add(1);
        entry.measurements = bound_measurements(
            std::mem::take(&mut entry.measurements),
            state.config.max_measurements,
        );
        entry.flags = bound_flags(std::mem::take(&mut entry.flags), state.config.max_flags);
        if state.logs.len() >= state.config.max_logs {
            let _ = state.logs.pop_front();
            state.dropped_logs = state.dropped_logs.saturating_add(1);
        }
        state.logs.push_back(entry.clone());
        state.touch(entry.timestamp_ms);
    });
    entry
}

/// Records a failure, updates its normalized group, and returns the retained
/// sanitized event. This channel is independent of `verenu:error`.
pub fn record_failure(input: FailureEventInput) -> FailureEvent {
    let timestamp_ms = now_ms();
    let subsystem = safe_label(&input.subsystem);
    let operation = safe_option_label(input.operation);
    let stage = safe_option_label(input.stage);
    let error_category = safe_option_label(input.error_category);
    let cause = redact_and_limit(&input.cause, MAX_CAUSE_CHARS);
    let fingerprint = failure_fingerprint(
        &subsystem,
        operation.as_deref(),
        stage.as_deref(),
        error_category.as_deref(),
        &cause,
    );
    let event = FailureEvent {
        id: next_id("failure"),
        timestamp_ms,
        subsystem: subsystem.clone(),
        operation: operation.clone(),
        stage: stage.clone(),
        cause: cause.clone(),
        duration_ms: input.duration_ms,
        provider: safe_option_label(input.provider),
        model: safe_option_label(input.model),
        trace_id: safe_option_id(input.trace_id),
        session_id: safe_option_id(input.session_id),
        fingerprint: fingerprint.clone(),
        error_category: error_category.clone(),
        retry_number: input.retry_number,
        fallback_number: input.fallback_number,
        measurements: input.measurements,
        flags: input.flags,
    };

    with_store(|state| {
        let mut event = event;
        event.measurements = bound_measurements(
            std::mem::take(&mut event.measurements),
            state.config.max_measurements,
        );
        event.flags = bound_flags(std::mem::take(&mut event.flags), state.config.max_flags);
        state.total_failures_recorded = state.total_failures_recorded.saturating_add(1);
        if state.failures.len() >= state.config.max_failures {
            let _ = state.failures.pop_front();
            state.dropped_failures = state.dropped_failures.saturating_add(1);
        }
        state.failures.push_back(event.clone());

        if let Some(group) = state
            .failure_groups
            .iter_mut()
            .find(|group| group.fingerprint == fingerprint)
        {
            group.count = group.count.saturating_add(1);
            group.last_seen_ms = timestamp_ms;
            group.last_trace_id = event.trace_id.clone();
            group.provider = event.provider.clone();
            group.model = event.model.clone();
        } else {
            if state.failure_groups.len() >= state.config.max_failure_groups {
                let oldest = state
                    .failure_groups
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, group)| group.last_seen_ms)
                    .map(|(index, _)| index)
                    .unwrap_or(0);
                state.failure_groups.remove(oldest);
            }
            state.failure_groups.push(FailureGroup {
                fingerprint,
                count: 1,
                first_seen_ms: timestamp_ms,
                last_seen_ms: timestamp_ms,
                subsystem,
                operation,
                stage,
                error_category,
                representative_cause: cause,
                last_trace_id: event.trace_id.clone(),
                provider: event.provider.clone(),
                model: event.model.clone(),
            });
        }
        state.touch(timestamp_ms);
        event
    })
}

/// Short alias retained for command helpers that use `FailureInput`.
pub type FailureInput = FailureEventInput;

/// Normalizes a failure message for display in tests or a diagnostics UI.
/// The result contains no volatile IDs, timestamps, paths, or offset values.
pub fn normalize_failure_cause(cause: &str) -> String {
    let redacted = redact_and_limit(cause, MAX_FINGERPRINT_INPUT_CHARS);
    normalize_failure_text(&redacted)
}

/// Returns the stable fingerprint used by [`record_failure`]. Context fields
/// are included so identical provider messages in different subsystems do not
/// collapse into one misleading group.
pub fn failure_fingerprint(
    subsystem: &str,
    operation: Option<&str>,
    stage: Option<&str>,
    error_category: Option<&str>,
    cause: &str,
) -> String {
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalize_identity(subsystem),
        normalize_identity(operation.unwrap_or("")),
        normalize_identity(stage.unwrap_or("")),
        normalize_identity(error_category.unwrap_or("")),
        normalize_failure_cause(cause),
    );
    format!("f-{hash:016x}", hash = fnv1a64(identity.as_bytes()))
}

/// Starts a bounded dictation trace.
pub fn start_trace(root_operation: &str, session_id: Option<&str>) -> TraceHandle {
    let trace_id = next_id("trace");
    let started = Instant::now();
    let started_at_ms = now_ms();
    with_store(|state| {
        if state.traces.len() >= state.config.max_traces {
            let _ = state.traces.pop_front();
            state.dropped_traces = state.dropped_traces.saturating_add(1);
        }
        state.traces.push_back(TraceState {
            trace_id: trace_id.clone(),
            session_id: session_id.map(sanitize_id),
            root_operation: safe_label(root_operation),
            started_at_ms,
            started,
            ended_at_ms: None,
            duration_ms: None,
            outcome: OperationOutcome::Running,
            spans: Vec::new(),
        });
        state.total_traces_started = state.total_traces_started.saturating_add(1);
        state.touch(started_at_ms);
    });
    TraceHandle { trace_id }
}

/// Starts a normal child span under a trace.
pub fn start_span(
    trace_id: &str,
    operation: &str,
    stage: Option<&str>,
    parent_span_id: Option<&str>,
) -> Option<SpanHandle> {
    start_span_inner(trace_id, operation, stage, parent_span_id, None)
}

/// Starts a span and labels it as part of a parallel lane. The lane name is a
/// safe label such as `dual-transcription`, not a task payload or request ID.
pub fn start_parallel_span(
    trace_id: &str,
    operation: &str,
    stage: Option<&str>,
    parent_span_id: Option<&str>,
    parallel_group: Option<&str>,
) -> Option<SpanHandle> {
    start_span_inner(trace_id, operation, stage, parent_span_id, parallel_group)
}

fn start_span_inner(
    trace_id: &str,
    operation: &str,
    stage: Option<&str>,
    parent_span_id: Option<&str>,
    parallel_group: Option<&str>,
) -> Option<SpanHandle> {
    let span_id = next_id("span");
    let started = Instant::now();
    let started_at_ms = now_ms();
    with_store(|state| {
        let max_spans_per_trace = state.config.max_spans_per_trace;
        let trace = state
            .traces
            .iter_mut()
            .find(|trace| trace.trace_id == trace_id && trace.ended_at_ms.is_none())?;
        if trace.spans.len() >= max_spans_per_trace {
            state.dropped_spans = state.dropped_spans.saturating_add(1);
            return None;
        }
        trace.spans.push(SpanState {
            span: Span {
                span_id: span_id.clone(),
                trace_id: trace_id.to_owned(),
                parent_span_id: parent_span_id.map(sanitize_id),
                parallel_group: parallel_group.map(safe_label),
                operation: safe_label(operation),
                stage: stage.map(safe_label),
                started_at_ms,
                ended_at_ms: None,
                duration_ms: None,
                outcome: OperationOutcome::Running,
                provider: None,
                model: None,
                retry_number: None,
                fallback_number: None,
                cache: None,
                error_category: None,
                error_fingerprint: None,
                measurements: BTreeMap::new(),
                flags: BTreeMap::new(),
            },
            started,
        });
        state.touch(started_at_ms);
        Some(SpanHandle {
            trace_id: trace_id.to_owned(),
            span_id,
        })
    })
}

/// Finishes a span using its measured monotonic duration.
pub fn finish_span(handle: &SpanHandle, outcome: OperationOutcome) -> bool {
    finish_span_with(
        handle,
        SpanFinish {
            outcome,
            ..SpanFinish::default()
        },
    )
}

/// Finishes a span and applies safe stage metadata. `duration_ms` can be used
/// when the caller already has a more accurate external measurement.
pub fn finish_span_with(handle: &SpanHandle, finish: SpanFinish) -> bool {
    let ended_at_ms = now_ms();
    with_store(|state| {
        let max_measurements = state.config.max_measurements;
        let max_flags = state.config.max_flags;
        let trace = state
            .traces
            .iter_mut()
            .find(|trace| trace.trace_id == handle.trace_id)?;
        let span = trace
            .spans
            .iter_mut()
            .find(|span| span.span.span_id == handle.span_id && span.span.ended_at_ms.is_none())?;
        span.span.ended_at_ms = Some(ended_at_ms);
        span.span.duration_ms = Some(
            finish
                .duration_ms
                .unwrap_or_else(|| elapsed_ms(span.started.elapsed())),
        );
        span.span.outcome = finish.outcome;
        span.span.provider = finish.provider.map(|value| safe_label(&value));
        span.span.model = finish.model.map(|value| safe_label(&value));
        span.span.retry_number = finish.retry_number;
        span.span.fallback_number = finish.fallback_number;
        span.span.cache = finish.cache;
        span.span.error_category = finish.error_category.map(|value| safe_label(&value));
        span.span.error_fingerprint = finish.error_fingerprint.map(|value| sanitize_id(&value));
        span.span.measurements = bound_measurements(finish.measurements, max_measurements);
        span.span.flags = bound_flags(finish.flags, max_flags);
        state.touch(ended_at_ms);
        Some(true)
    })
    .unwrap_or(false)
}

/// Finishes a trace. Any still-running spans are marked cancelled so a trace
/// cannot retain live spans forever after a pipeline cancellation.
pub fn finish_trace(trace_id: &str, outcome: OperationOutcome) -> bool {
    let ended_at_ms = now_ms();
    with_store(|state| {
        let trace = state
            .traces
            .iter_mut()
            .find(|trace| trace.trace_id == trace_id)?;
        if trace.ended_at_ms.is_some() {
            return Some(false);
        }
        for span in &mut trace.spans {
            if span.span.ended_at_ms.is_none() {
                span.span.ended_at_ms = Some(ended_at_ms);
                span.span.duration_ms = Some(elapsed_ms(span.started.elapsed()));
                span.span.outcome = OperationOutcome::Cancelled;
            }
        }
        trace.ended_at_ms = Some(ended_at_ms);
        trace.duration_ms = Some(elapsed_ms(trace.started.elapsed()));
        trace.outcome = outcome;
        state.total_traces_completed = state.total_traces_completed.saturating_add(1);
        state.touch(ended_at_ms);
        Some(true)
    })
    .unwrap_or(false)
}

/// Starts timing an operation. The operation is counted immediately, while
/// duration and outcome counters update when [`finish_operation`] runs.
pub fn start_operation(operation: &str) -> OperationHandle {
    let operation = safe_label(operation);
    let started = Instant::now();
    let started_at_ms = now_ms();
    let tracked = with_store(|state| {
        let max_duration_samples = state.config.max_duration_samples;
        let tracked = if let Some(metric) = operation_metric_mut(state, &operation, true) {
            metric.metric.calls = metric.metric.calls.saturating_add(1);
            metric.metric.currently_running = metric.metric.currently_running.saturating_add(1);
            metric.metric.last_called_at_ms = Some(started_at_ms);
            metric.call_times.push_back(started_at_ms);
            trim_call_times(metric, started_at_ms, max_duration_samples);
            true
        } else {
            false
        };
        if tracked {
            state.total_operations_recorded = state.total_operations_recorded.saturating_add(1);
            state.touch(started_at_ms);
        }
        tracked
    });
    OperationHandle {
        operation,
        started,
        tracked,
    }
}

/// Finishes an operation using its monotonic duration.
pub fn finish_operation(handle: OperationHandle, outcome: OperationOutcome) -> bool {
    if !handle.tracked {
        return false;
    }
    record_operation_duration(
        &handle.operation,
        elapsed_ms(handle.started.elapsed()),
        outcome,
        true,
    )
}

/// Compatibility helper for central wrappers such as `commands::run_blocking`.
pub fn operation_started(operation: &'static str) -> OperationHandle {
    start_operation(operation)
}

impl OperationHandle {
    /// Completes this operation from a boolean success result.
    pub fn finish(self, success: bool) {
        let _ = finish_operation(
            self,
            if success {
                OperationOutcome::Success
            } else {
                OperationOutcome::Failure
            },
        );
    }
}

/// Records an already measured operation. This is convenient for command
/// wrappers that already own an `Instant` or a native duration.
#[allow(dead_code)]
pub fn record_operation(operation: &str, duration_ms: u64, outcome: OperationOutcome) -> bool {
    let operation = safe_label(operation);
    with_store(|state| {
        let max_duration_samples = state.config.max_duration_samples;
        let called_at_ms = now_ms();
        let recorded = if let Some(metric) = operation_metric_mut(state, &operation, true) {
            metric.metric.calls = metric.metric.calls.saturating_add(1);
            metric.metric.last_called_at_ms = Some(called_at_ms);
            metric.call_times.push_back(called_at_ms);
            trim_call_times(metric, called_at_ms, max_duration_samples);
            update_operation_metric(metric, duration_ms, outcome, max_duration_samples);
            true
        } else {
            false
        };
        if recorded {
            state.total_operations_recorded = state.total_operations_recorded.saturating_add(1);
            state.touch(called_at_ms);
        }
        recorded
    })
}

fn record_operation_duration(
    operation: &str,
    duration_ms: u64,
    outcome: OperationOutcome,
    was_running: bool,
) -> bool {
    with_store(|state| {
        let max_duration_samples = state.config.max_duration_samples;
        let Some(metric) = operation_metric_mut(state, operation, false) else {
            return false;
        };
        if was_running {
            metric.metric.currently_running = metric.metric.currently_running.saturating_sub(1);
        }
        update_operation_metric(metric, duration_ms, outcome, max_duration_samples);
        state.touch(now_ms());
        true
    })
}

/// Records one resource point if profiling is enabled. The native collector
/// should populate unavailable values as `None`, never as a fake zero.
pub fn record_resource_sample(mut resource: ResourceSnapshot) -> bool {
    with_store(|state| {
        if !(state.profiler_enabled || state.profiling_recording) {
            return false;
        }
        resource.cpu_percent = resource
            .cpu_percent
            .filter(|value| value.is_finite() && *value >= 0.0);
        resource.read_bytes_per_sec = resource
            .read_bytes_per_sec
            .filter(|value| value.is_finite() && *value >= 0.0);
        resource.write_bytes_per_sec = resource
            .write_bytes_per_sec
            .filter(|value| value.is_finite() && *value >= 0.0);
        resource.observed_at_ms = if resource.observed_at_ms == 0 {
            now_ms()
        } else {
            resource.observed_at_ms
        };
        if let Some(resident) = resource.resident_bytes {
            state.peak_resident_bytes = Some(
                state
                    .peak_resident_bytes
                    .map_or(resident, |peak| peak.max(resident)),
            );
        }
        resource.peak_resident_bytes = state.peak_resident_bytes;
        resource
            .child_processes
            .truncate(state.config.max_child_processes);
        for child in &mut resource.child_processes {
            child.label = safe_label(&child.label);
            child.cpu_percent = child
                .cpu_percent
                .filter(|value| value.is_finite() && *value >= 0.0);
        }
        if let Some(duration_us) = resource.collector_duration_us {
            state.collector_samples = state.collector_samples.saturating_add(1);
            state.collector_duration_us_total = state
                .collector_duration_us_total
                .saturating_add(duration_us);
        }
        state.next_resource_sequence = state.next_resource_sequence.saturating_add(1);
        let sample = ResourceSample {
            sequence: state.next_resource_sequence,
            observed_at_ms: resource.observed_at_ms,
            snapshot: resource.clone(),
        };
        if state.resource_samples.len() >= state.config.max_resource_samples {
            let _ = state.resource_samples.pop_front();
            state.dropped_resource_samples = state.dropped_resource_samples.saturating_add(1);
        }
        state.current_resource = Some(resource);
        state.resource_samples.push_back(sample);
        state.touch(
            state
                .current_resource
                .as_ref()
                .map_or(now_ms(), |r| r.observed_at_ms),
        );
        true
    })
}

fn start_resource_window(state: &mut DiagnosticsStore) {
    if state.profiler_started.is_some() {
        return;
    }
    let now = Instant::now();
    state.profiler_started = Some(now);
    state.profiler_started_at_ms = Some(now_ms());
    state.peak_resident_bytes = None;
    state.resource_samples.clear();
    state.current_resource = None;
    state.next_resource_sequence = 0;
}

/// Returns a cheap, serializable copy of all current bounded state.
pub fn snapshot() -> DiagnosticsSnapshot {
    with_store(|state| snapshot_locked(state, now_ms()))
}

/// Frontend-facing snapshot with a smaller transfer budget than the retained
/// export ring. The backend keeps the larger bounded history for export and
/// postmortem work, while ordinary polling never serializes thousands of rows.
pub fn snapshot_for_ui() -> DiagnosticsSnapshot {
    let mut snapshot = snapshot();
    snapshot.logs = snapshot.logs.into_iter().rev().take(800).collect();
    snapshot.logs.reverse();
    snapshot.latest_failures = snapshot
        .latest_failures
        .into_iter()
        .rev()
        .take(64)
        .collect();
    snapshot.latest_failures.reverse();
    snapshot.resource_samples = snapshot
        .resource_samples
        .into_iter()
        .rev()
        .take(120)
        .collect();
    snapshot.resource_samples.reverse();
    snapshot.recent_pipelines = snapshot
        .recent_pipelines
        .into_iter()
        .rev()
        .take(32)
        .collect();
    snapshot.recent_pipelines.reverse();
    snapshot
}

/// Clears all retained state while preserving the current bounds. This is
/// public for deterministic integration tests and explicit "clear diagnostics"
/// tooling. It does not change profiler enablement.
#[allow(dead_code)]
pub fn reset_for_tests() {
    with_store(|state| {
        let config = state.config.clone();
        let profiler_enabled = state.profiler_enabled;
        let profiling_recording = state.profiling_recording;
        let profiler_started_at_ms = state.profiler_started_at_ms;
        let profiler_started = state.profiler_started;
        *state = DiagnosticsStore::new(config);
        state.profiler_enabled = profiler_enabled;
        state.profiling_recording = profiling_recording;
        state.profiler_started_at_ms = profiler_started_at_ms;
        state.profiler_started = profiler_started;
    });
}

/// Clears retained diagnostics while preserving configured bounds and the
/// current profiler enablement. This is the production-facing counterpart to
/// [`reset_for_tests`] for a user-requested new recording window.
#[allow(dead_code)]
pub fn reset() {
    reset_for_tests();
}

fn snapshot_locked(state: &DiagnosticsStore, now: u64) -> DiagnosticsSnapshot {
    let mut active_pipelines = Vec::new();
    let mut recent_pipelines = Vec::new();
    for trace in &state.traces {
        let public_trace = public_trace(trace);
        if trace.ended_at_ms.is_none() {
            active_pipelines.push(public_trace);
        } else {
            recent_pipelines.push(public_trace);
        }
    }
    let operations = state
        .operations
        .iter()
        .map(|metric| public_operation_metric(metric, now, state.config.max_duration_samples))
        .collect();
    let profiler_elapsed_ms = state
        .profiler_started
        .map(|started| elapsed_ms(started.elapsed()));
    DiagnosticsSnapshot {
        generated_at_ms: now,
        profiler_enabled: state.profiler_enabled,
        profiling_recording: state.profiling_recording,
        current_resource: state.current_resource.clone(),
        resource_samples: state.resource_samples.iter().cloned().collect(),
        latest_failures: state.failures.iter().cloned().collect(),
        failure_groups: state.failure_groups.clone(),
        active_pipelines,
        recent_pipelines,
        logs: state.logs.iter().cloned().collect(),
        operations,
        runtime: DiagnosticsRuntime::default(),
        health: DiagnosticsHealth {
            initialized: true,
            profiler_enabled: state.profiler_enabled,
            profiler_started_at_ms: state.profiler_started_at_ms,
            profiler_elapsed_ms,
            retained_log_count: state.logs.len(),
            retained_failure_count: state.failures.len(),
            retained_trace_count: state.traces.len(),
            retained_operation_count: state.operations.len(),
            retained_resource_sample_count: state.resource_samples.len(),
            active_trace_count: state
                .traces
                .iter()
                .filter(|trace| trace.ended_at_ms.is_none())
                .count(),
            active_span_count: state
                .traces
                .iter()
                .flat_map(|trace| trace.spans.iter())
                .filter(|span| span.span.ended_at_ms.is_none())
                .count(),
            total_logs_recorded: state.total_logs_recorded,
            total_failures_recorded: state.total_failures_recorded,
            total_traces_started: state.total_traces_started,
            total_traces_completed: state.total_traces_completed,
            total_operations_recorded: state.total_operations_recorded,
            dropped_logs: state.dropped_logs,
            dropped_failures: state.dropped_failures,
            dropped_traces: state.dropped_traces,
            dropped_spans: state.dropped_spans,
            dropped_resource_samples: state.dropped_resource_samples,
            collector_samples: state.collector_samples,
            collector_duration_us_total: state.collector_duration_us_total,
            collector_duration_us_average: (state.collector_samples > 0)
                .then(|| state.collector_duration_us_total as f64 / state.collector_samples as f64),
            last_event_at_ms: state.last_event_at_ms,
        },
    }
}

fn public_trace(trace: &TraceState) -> PipelineTrace {
    let spans = trace
        .spans
        .iter()
        .map(|state| {
            let mut span = state.span.clone();
            if span.ended_at_ms.is_none() {
                span.duration_ms = Some(elapsed_ms(state.started.elapsed()));
            }
            span
        })
        .collect();
    PipelineTrace {
        trace_id: trace.trace_id.clone(),
        session_id: trace.session_id.clone(),
        root_operation: trace.root_operation.clone(),
        started_at_ms: trace.started_at_ms,
        ended_at_ms: trace.ended_at_ms,
        duration_ms: trace
            .duration_ms
            .or_else(|| Some(elapsed_ms(trace.started.elapsed()))),
        outcome: trace.outcome,
        spans,
    }
}

fn operation_metric_mut<'a>(
    state: &'a mut DiagnosticsStore,
    operation: &str,
    create: bool,
) -> Option<&'a mut OperationMetricState> {
    if let Some(index) = state
        .operations
        .iter()
        .position(|metric| metric.metric.operation == operation)
    {
        return state.operations.get_mut(index);
    }
    if !create || state.operations.len() >= state.config.max_operations {
        return None;
    }
    state.operations.push(OperationMetricState {
        metric: OperationMetric {
            operation: operation.to_owned(),
            calls: 0,
            success_count: 0,
            failure_count: 0,
            cancelled_count: 0,
            skipped_count: 0,
            total_duration_ms: 0,
            average_duration_ms: None,
            recent_duration_ms: None,
            p95_duration_ms: None,
            max_duration_ms: None,
            calls_per_minute: 0,
            currently_running: 0,
            last_called_at_ms: None,
        },
        durations: VecDeque::new(),
        call_times: VecDeque::new(),
    });
    state.operations.last_mut()
}

fn update_operation_metric(
    state: &mut OperationMetricState,
    duration_ms: u64,
    outcome: OperationOutcome,
    max_duration_samples: usize,
) {
    state.metric.total_duration_ms = state.metric.total_duration_ms.saturating_add(duration_ms);
    state.metric.recent_duration_ms = Some(duration_ms);
    state.metric.max_duration_ms = Some(
        state
            .metric
            .max_duration_ms
            .map_or(duration_ms, |max| max.max(duration_ms)),
    );
    state.durations.push_back(duration_ms);
    while state.durations.len() > max_duration_samples {
        let _ = state.durations.pop_front();
    }
    match outcome {
        OperationOutcome::Success => {
            state.metric.success_count = state.metric.success_count.saturating_add(1)
        }
        OperationOutcome::Failure => {
            state.metric.failure_count = state.metric.failure_count.saturating_add(1)
        }
        OperationOutcome::Cancelled => {
            state.metric.cancelled_count = state.metric.cancelled_count.saturating_add(1)
        }
        OperationOutcome::Skipped => {
            state.metric.skipped_count = state.metric.skipped_count.saturating_add(1)
        }
        OperationOutcome::Running | OperationOutcome::Unknown => {}
    }
    state.metric.average_duration_ms = (state.metric.calls > 0)
        .then(|| state.metric.total_duration_ms as f64 / state.metric.calls as f64);
    state.metric.p95_duration_ms = percentile_95(&state.durations);
}

fn public_operation_metric(
    state: &OperationMetricState,
    now: u64,
    max_duration_samples: usize,
) -> OperationMetric {
    let mut metric = state.metric.clone();
    let mut call_times = state.call_times.clone();
    trim_call_times_at(&mut call_times, now, max_duration_samples);
    metric.calls_per_minute = call_times.len() as u64;
    metric.p95_duration_ms = percentile_95(&state.durations);
    metric
}

fn trim_call_times(state: &mut OperationMetricState, now: u64, max_samples: usize) {
    trim_call_times_at(&mut state.call_times, now, max_samples);
}

fn trim_call_times_at(call_times: &mut VecDeque<u64>, now: u64, max_samples: usize) {
    while call_times
        .front()
        .is_some_and(|at| now.saturating_sub(*at) > ROLLING_CALL_WINDOW_MS)
    {
        let _ = call_times.pop_front();
    }
    while call_times.len() > max_samples {
        let _ = call_times.pop_front();
    }
}

fn percentile_95(values: &VecDeque<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = values.iter().copied().collect();
    sorted.sort_unstable();
    let index = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
    sorted.get(index).copied()
}

fn bound_measurements(values: BTreeMap<String, f64>, limit: usize) -> BTreeMap<String, f64> {
    values
        .into_iter()
        .filter_map(|(key, value)| {
            if value.is_finite() {
                Some((safe_label(&key), value))
            } else {
                None
            }
        })
        .take(limit)
        .collect()
}

fn bound_flags(values: BTreeMap<String, bool>, limit: usize) -> BTreeMap<String, bool> {
    values
        .into_iter()
        .map(|(key, value)| (safe_label(&key), value))
        .take(limit)
        .collect()
}

fn safe_option_label(value: Option<String>) -> Option<String> {
    value
        .map(|value| safe_label(&value))
        .filter(|value| !value.is_empty())
}

fn safe_option_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| sanitize_id(&value))
        .filter(|value| !value.is_empty())
}

fn safe_label(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(MAX_LABEL_CHARS));
    for ch in value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_LABEL_CHARS)
    {
        output.push(ch);
    }
    output.trim().to_owned()
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
        .take(MAX_LABEL_CHARS)
        .collect()
}

fn redact_and_limit(value: &str, max_chars: usize) -> String {
    let mut output = value.to_owned();
    for marker in [
        "authorization:",
        "bearer ",
        "api_key=",
        "x-api-key:",
        "x-goog-api-key:",
        "?key=",
        "&key=",
        "raw_full=",
        "input_full=",
        "prompt_full=",
        "output_full=",
        "final_text_full=",
        "app_context_full=",
        "before_full=",
        "after_full=",
        "raw_text=",
        "clean_text=",
        "dictation=",
        "clipboard=",
        "raw=",
        "raw_preview=",
        "final_preview=",
        "\"api_key\":",
        "\"authorization\":",
    ] {
        output = redact_after_marker(&output, marker);
    }
    output = redact_email_and_url_tokens(&output);
    output = output
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .take(max_chars)
        .collect();
    output.trim().to_owned()
}

fn redact_after_marker(input: &str, marker: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let marker_lower = marker.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(relative) = lower[cursor..].find(&marker_lower) {
        let start = cursor + relative;
        output.push_str(&input[cursor..start + marker.len()]);
        let mut value_start = start + marker.len();
        while value_start < input.len() && input.as_bytes()[value_start].is_ascii_whitespace() {
            output.push(input.as_bytes()[value_start] as char);
            value_start += 1;
        }
        if value_start >= input.len() {
            cursor = value_start;
            break;
        }
        let quoted = input.as_bytes()[value_start] == b'"';
        let content_start = value_start + usize::from(quoted);
        let end = if quoted {
            input[content_start..]
                .find('"')
                .map_or(input.len(), |offset| content_start + offset)
        } else {
            input[value_start..]
                .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';'))
                .map_or(input.len(), |offset| value_start + offset)
        };
        output.push_str("[REDACTED]");
        if quoted && end < input.len() {
            output.push('"');
            cursor = end + 1;
        } else {
            cursor = end;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn redact_email_and_url_tokens(input: &str) -> String {
    input
        .split_whitespace()
        .map(|token| {
            let has_email = token.contains('@');
            let has_url = token.contains("://");
            let has_path = token.contains("\\")
                || token.starts_with("/Users/")
                || token.starts_with("/home/")
                || token.starts_with("C:/")
                || token.starts_with("C:\\");
            if has_email {
                "[REDACTED_EMAIL]"
            } else if has_url {
                "[REDACTED_URL]"
            } else if has_path {
                "[REDACTED_PATH]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_failure_text(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_word = String::new();
    for raw_token in value.split_whitespace() {
        let token = raw_token.trim_matches(|ch: char| ch.is_ascii_punctuation());
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        let normalized_token = if is_uuid(token) || looks_like_timestamp(token) {
            "<volatile>".to_owned()
        } else if looks_like_path(token) {
            "<path>".to_owned()
        } else if let Some((key, _)) = lower.split_once('=') {
            if is_volatile_key(key) {
                format!("{key}=<volatile>")
            } else {
                lower.clone()
            }
        } else if is_volatile_key(&previous_word) && token.chars().all(|ch| ch.is_ascii_digit()) {
            "<volatile>".to_owned()
        } else {
            lower.clone()
        };
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(&normalized_token);
        previous_word = lower
            .trim_matches(|ch: char| ch.is_ascii_punctuation())
            .to_owned();
    }
    normalized
        .chars()
        .take(MAX_FINGERPRINT_INPUT_CHARS)
        .collect()
}

fn normalize_identity(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_volatile_key(key: &str) -> bool {
    matches!(
        key.trim_matches(|ch: char| ch.is_ascii_punctuation()),
        "id" | "request"
            | "request_id"
            | "request-id"
            | "trace_id"
            | "trace-id"
            | "span_id"
            | "span-id"
            | "offset"
            | "line"
            | "column"
            | "position"
            | "pid"
            | "file_offset"
            | "file-offset"
    )
}

fn looks_like_path(value: &str) -> bool {
    value.contains('\\')
        || value.starts_with('/')
        || value.starts_with("C:/")
        || value.starts_with("C:\\")
        || value.starts_with("[REDACTED_PATH]")
}

fn looks_like_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn next_id(prefix: &str) -> String {
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{timestamp:x}-{sequence:x}", timestamp = now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn small_config() -> DiagnosticsConfig {
        DiagnosticsConfig {
            max_logs: 2,
            max_failures: 2,
            max_failure_groups: 2,
            max_traces: 2,
            max_spans_per_trace: 3,
            max_operations: 2,
            max_duration_samples: 3,
            max_resource_samples: 2,
            max_child_processes: 1,
            max_measurements: 1,
            max_flags: 1,
        }
    }

    fn reset() {
        init_with_config(small_config());
        reset_for_tests();
        set_profiler_enabled(false);
    }

    #[test]
    fn log_ring_and_fields_are_bounded() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        for index in 0..4 {
            let mut fields = StructuredFields::default();
            fields.measurements.insert("one".into(), index as f64);
            fields.measurements.insert("two".into(), 2.0);
            fields.flags.insert("one".into(), true);
            fields.flags.insert("two".into(), false);
            record_log_with_fields(
                LogLevel::Info,
                "pipeline",
                &format!("event {index}"),
                fields,
            );
        }
        let snapshot = snapshot();
        assert_eq!(snapshot.logs.len(), 2);
        assert_eq!(snapshot.logs[0].message, "event 2");
        assert_eq!(snapshot.logs[0].measurements.len(), 1);
        assert_eq!(snapshot.health.dropped_logs, 2);
    }

    #[test]
    fn sensitive_log_text_is_redacted() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        let entry = record_log_with_fields(
            LogLevel::Error,
            "provider",
            r#"request_id=abc raw_full="private words" api_key=secret {"api_key":"secret2"} alice@example.com"#,
            StructuredFields::default(),
        );
        assert!(!entry.message.contains("private words"));
        assert!(!entry.message.contains("secret"));
        assert!(!entry.message.contains("alice@example.com"));
    }

    #[test]
    fn failure_groups_normalize_volatile_values_but_keep_context() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        let make = |request_id: &str, subsystem: &str| FailureEventInput {
            subsystem: subsystem.into(),
            operation: Some("request".into()),
            stage: Some("transcription".into()),
            cause: format!("timeout request_id={request_id} offset=12345 at 2026-09-09T10:11:12Z"),
            error_category: Some("timeout".into()),
            ..FailureEventInput::default()
        };
        let first = record_failure(make("one", "provider"));
        let second = record_failure(make("two", "provider"));
        let other = record_failure(make("three", "cleanup"));
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_ne!(first.fingerprint, other.fingerprint);
        let snapshot = snapshot();
        assert_eq!(snapshot.failure_groups.len(), 2);
        assert!(snapshot.failure_groups.iter().any(|group| group.count == 2));
        assert!(normalize_failure_cause(&first.cause).contains("request_id=<volatile>"));
        assert_eq!(
            normalize_failure_cause("request-id: first line: 42"),
            "request-id first line <volatile>"
        );
    }

    #[test]
    fn failure_retention_is_bounded() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        for index in 0..4 {
            record_failure(FailureEventInput {
                subsystem: "db".into(),
                cause: format!("failure {index}"),
                ..FailureEventInput::default()
            });
        }
        let snapshot = snapshot();
        assert_eq!(snapshot.latest_failures.len(), 2);
        assert_eq!(snapshot.health.dropped_failures, 2);
    }

    #[test]
    fn trace_and_parallel_span_lifecycle_is_bounded() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        let trace = start_trace("dictation", Some("session-1"));
        let capture =
            start_span(&trace.trace_id, "capture", Some("capture"), None).expect("capture span");
        let left = start_parallel_span(
            &trace.trace_id,
            "transcribe",
            Some("transcription"),
            None,
            Some("dual-transcription"),
        )
        .expect("left span");
        let right = start_parallel_span(
            &trace.trace_id,
            "transcribe",
            Some("transcription"),
            None,
            Some("dual-transcription"),
        )
        .expect("right span");
        assert!(finish_span(&capture, OperationOutcome::Success));
        assert!(finish_span(&left, OperationOutcome::Success));
        assert!(finish_span(&right, OperationOutcome::Failure));
        assert!(finish_trace(&trace.trace_id, OperationOutcome::Failure));
        let snapshot = snapshot();
        let trace = &snapshot.recent_pipelines[0];
        assert_eq!(trace.spans.len(), 3);
        assert_eq!(trace.outcome, OperationOutcome::Failure);
        assert!(trace.spans.iter().all(|span| span.ended_at_ms.is_some()));
        assert!(
            trace
                .spans
                .iter()
                .filter_map(|span| span.parallel_group.as_ref())
                .count()
                == 2
        );
    }

    #[test]
    fn operation_metrics_keep_recent_samples_and_p95() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        for duration in [10, 20, 30, 40, 100] {
            assert!(record_operation(
                "commands::read",
                duration,
                OperationOutcome::Success
            ));
        }
        let handle = start_operation("commands::read");
        let initial = snapshot();
        assert_eq!(initial.operations[0].calls, 6);
        assert_eq!(initial.operations[0].currently_running, 1);
        assert_eq!(initial.operations[0].p95_duration_ms, Some(100));
        assert!(finish_operation(handle, OperationOutcome::Success));
        let completed_snapshot = snapshot();
        assert_eq!(completed_snapshot.operations[0].currently_running, 0);
        assert_eq!(completed_snapshot.operations[0].success_count, 6);
    }

    #[test]
    fn profiler_gates_samples_and_keeps_unknown_values_null() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        let unavailable = ResourceSnapshot {
            observed_at_ms: 1,
            collector_duration_us: Some(4),
            ..ResourceSnapshot::default()
        };
        assert!(!record_resource_sample(unavailable.clone()));
        set_profiling_recording(true);
        assert!(should_sample_resources());
        assert!(record_resource_sample(unavailable.clone()));
        set_profiling_recording(false);
        set_profiler_enabled(true);
        assert!(record_resource_sample(unavailable));
        let snapshot = snapshot();
        assert_eq!(snapshot.resource_samples.len(), 1);
        assert_eq!(snapshot.resource_samples[0].snapshot.cpu_percent, None);
        assert_eq!(snapshot.health.collector_samples, 2);
        let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
        assert!(json.contains("\"cpu_percent\":null"));
        set_profiler_enabled(false);
        assert!(!record_resource_sample(ResourceSnapshot::default()));
        set_profiling_recording(true);
        assert!(record_resource_sample(ResourceSnapshot::default()));
        assert!(profiling_recording());
        set_profiling_recording(false);
        assert!(!should_sample_resources());
    }

    #[test]
    fn resource_and_trace_histories_have_explicit_bounds() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        reset();
        set_profiler_enabled(true);
        for index in 0..4 {
            assert!(record_resource_sample(ResourceSnapshot {
                observed_at_ms: index,
                resident_bytes: Some(index),
                ..ResourceSnapshot::default()
            }));
            let trace = start_trace("trace", None);
            assert!(finish_trace(&trace.trace_id, OperationOutcome::Success));
        }
        let snapshot = snapshot();
        assert_eq!(snapshot.resource_samples.len(), 2);
        assert_eq!(snapshot.recent_pipelines.len(), 2);
        assert_eq!(snapshot.health.dropped_resource_samples, 2);
        assert_eq!(snapshot.health.dropped_traces, 2);
    }
}
