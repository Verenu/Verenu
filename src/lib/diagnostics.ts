export type DiagnosticLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace' | string;

export interface StructuredLogEntry {
  timestamp_ms: number; level: DiagnosticLevel; subsystem: string;
  operation?: string | null; stage?: string | null; message: string;
  trace_id?: string | null; session_id?: string | null; duration_ms?: number | null;
  outcome?: string | null; retry_number?: number | null; fallback_number?: number | null;
  cache?: string | null; error_category?: string | null; error_fingerprint?: string | null;
  measurements?: Record<string, number>; flags?: Record<string, boolean>;
}

export interface FailureEvent {
  id: string; timestamp_ms: number; subsystem: string;
  operation?: string | null; stage?: string | null; cause: string; duration_ms?: number | null;
  provider?: string | null; model?: string | null; trace_id?: string | null; session_id?: string | null;
  fingerprint: string; error_category?: string | null; retry_number?: number | null; fallback_number?: number | null;
  measurements?: Record<string, number>; flags?: Record<string, boolean>;
}

export interface FailureGroup {
  fingerprint: string; count: number; first_seen_ms: number; last_seen_ms: number;
  subsystem: string; operation?: string | null; stage?: string | null; error_category?: string | null;
  representative_cause: string; last_trace_id?: string | null; provider?: string | null; model?: string | null;
}

export interface SpanRecord {
  span_id: string; trace_id: string; parent_span_id?: string | null; parallel_group?: string | null;
  operation: string; stage?: string | null; started_at_ms: number; ended_at_ms?: number | null;
  duration_ms?: number | null; outcome: string; provider?: string | null; model?: string | null;
  retry_number?: number | null; fallback_number?: number | null; cache?: string | null;
  error_category?: string | null; error_fingerprint?: string | null;
  measurements?: Record<string, number>; flags?: Record<string, boolean>;
}

export interface PipelineTrace {
  trace_id: string; session_id?: string | null; root_operation: string; started_at_ms: number;
  ended_at_ms?: number | null; duration_ms?: number | null; outcome: string; spans: SpanRecord[];
}

export interface OperationMetric {
  operation: string; calls: number; success_count: number; failure_count: number;
  cancelled_count: number; skipped_count: number; total_duration_ms: number;
  average_duration_ms?: number | null; recent_duration_ms?: number | null; p95_duration_ms?: number | null;
  max_duration_ms?: number | null; calls_per_minute: number; currently_running: number;
  last_called_at_ms?: number | null;
}

export interface ChildProcessSnapshot {
  label: string; pid?: number | null; cpu_percent?: number | null; resident_bytes?: number | null;
  private_bytes?: number | null; read_bytes_total?: number | null; write_bytes_total?: number | null;
  thread_count?: number | null;
}

export interface ResourceSnapshot {
  observed_at_ms: number; cpu_percent?: number | null; resident_bytes?: number | null;
  private_bytes?: number | null; peak_resident_bytes?: number | null; process_count?: number | null;
  child_processes: ChildProcessSnapshot[]; read_bytes_total?: number | null; write_bytes_total?: number | null;
  read_bytes_per_sec?: number | null; write_bytes_per_sec?: number | null; thread_count?: number | null;
  handle_count?: number | null; gpu_memory_bytes?: number | null; uptime_ms?: number | null;
  local_stt_state?: string | null; local_llm_state?: string | null; collector_duration_us?: number | null;
}

export interface ResourceSample { sequence: number; observed_at_ms: number; snapshot: ResourceSnapshot; }

export interface DiagnosticsHealth {
  initialized: boolean; profiler_enabled: boolean; profiler_started_at_ms?: number | null;
  profiler_elapsed_ms?: number | null; retained_log_count: number; retained_failure_count: number;
  retained_trace_count: number; retained_operation_count: number; retained_resource_sample_count: number;
  active_trace_count: number; active_span_count: number; total_logs_recorded: number; total_failures_recorded: number;
  total_traces_started: number; total_traces_completed: number; total_operations_recorded: number;
  dropped_logs: number; dropped_failures: number; dropped_traces: number; dropped_spans: number;
  dropped_resource_samples: number; collector_samples: number; collector_duration_us_total: number;
  collector_duration_us_average?: number | null; last_event_at_ms?: number | null;
}

export interface DiagnosticsRuntime {
  local_stt?: Record<string, unknown> | null;
  local_llm?: Record<string, unknown> | null;
  sync?: Record<string, unknown> | null;
  auto_learn?: Record<string, unknown> | null;
  cleanup_cache?: Record<string, unknown> | null;
  audio?: AudioDiagnostics | null;
}

export interface AudioDiagnostics {
  active: boolean; raw_rms?: number | null; processed_level?: number | null;
  gate_rms?: number | null; would_pass_gate?: boolean | null;
  speech_detected: boolean; stream_error: boolean;
  adaptive_sensitivity?: number | null; microphone_gain?: number | null;
}

export interface DiagnosticsSnapshot {
  generated_at_ms: number; profiler_enabled: boolean; profiling_recording: boolean;
  current_resource?: ResourceSnapshot | null; resource_samples: ResourceSample[];
  latest_failures: FailureEvent[]; failure_groups: FailureGroup[]; active_pipelines: PipelineTrace[];
  recent_pipelines: PipelineTrace[]; logs: StructuredLogEntry[]; operations: OperationMetric[];
  runtime: DiagnosticsRuntime; health: DiagnosticsHealth;
}

const MAX_CLIENT_ITEMS = 1_000;
const MAX_FRONTEND_METRICS = 128;
const MAX_FRONTEND_SAMPLES = 64;

export function pushBounded<T>(items: readonly T[], item: T, limit = MAX_CLIENT_ITEMS): T[] {
  const next = [...items, item]; return next.length > limit ? next.slice(next.length - limit) : next;
}

export function p95(values: readonly number[]): number | null {
  if (!values.length) return null; const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(sorted.length * 0.95) - 1)] ?? null;
}

export function filterLogs(logs: readonly StructuredLogEntry[], query: string, level: string, subsystem: string, trace: string): StructuredLogEntry[] {
  const needle = query.trim().toLowerCase(); const traceNeedle = trace.trim().toLowerCase();
  return logs.filter((entry) => {
    if (level !== 'all' && entry.level.toLowerCase() !== level) return false;
    if (subsystem !== 'all' && entry.subsystem !== subsystem) return false;
    if (traceNeedle && !(entry.trace_id ?? '').toLowerCase().includes(traceNeedle)) return false;
    if (!needle) return true;
    return [entry.message, entry.subsystem, entry.operation ?? '', entry.stage ?? '', entry.trace_id ?? ''].some((value) => value.toLowerCase().includes(needle));
  });
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return 'Unavailable';
  if (ms < 1_000) return `${Math.round(ms)} ms`; if (ms < 60_000) return `${(ms / 1_000).toFixed(2)} s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1_000)}s`;
}

export function formatRate(value: number | null | undefined, unit = '/s'): string {
  if (value == null || !Number.isFinite(value)) return 'Unavailable';
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M${unit}`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k${unit}`;
  return `${value.toFixed(value < 10 ? 1 : 0)}${unit}`;
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return 'Unavailable'; if (bytes < 1_024) return `${Math.round(bytes)} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB']; let value = bytes; let unit = 'B';
  for (const next of units) { value /= 1_024; unit = next; if (value < 1_024) break; }
  return `${value.toFixed(value >= 100 ? 0 : value >= 10 ? 1 : 2)} ${unit}`;
}

export function unknown(value: unknown): string { if (value == null || value === '') return 'Unavailable'; if (typeof value === 'boolean') return value ? 'Yes' : 'No'; return String(value); }
export function spanWidth(duration: number | null | undefined, total: number | null | undefined): number { if (!duration || !total || total <= 0) return 0; return Math.min(100, Math.max(1, duration / total * 100)); }

export interface FrontendIpcMetric { command: string; calls: number; failures: number; total_duration_ms: number; average_duration_ms: number | null; p95_duration_ms: number | null; max_duration_ms: number | null; currently_running: number; hidden_calls: number; first_seen: number; last_seen: number; samples: number[]; last_error: string | null; }

class FrontendIpcActivity {
  private readonly metrics = new Map<string, FrontendIpcMetric>();
  start(command: string): number {
    const now = Date.now(); let metric = this.metrics.get(command);
    if (!metric) {
      if (this.metrics.size >= MAX_FRONTEND_METRICS) { const oldest = [...this.metrics.values()].sort((a, b) => a.last_seen - b.last_seen)[0]; if (oldest) this.metrics.delete(oldest.command); }
      metric = { command, calls: 0, failures: 0, total_duration_ms: 0, average_duration_ms: null, p95_duration_ms: null, max_duration_ms: null, currently_running: 0, hidden_calls: 0, first_seen: now, last_seen: now, samples: [], last_error: null };
      this.metrics.set(command, metric);
    }
    metric.currently_running += 1; return now;
  }
  finish(command: string, started: number, success: boolean, error?: string): void {
    const metric = this.metrics.get(command); if (!metric) return; const duration = Math.max(0, Date.now() - started);
    metric.currently_running = Math.max(0, metric.currently_running - 1); metric.calls += 1; if (!success) metric.failures += 1;
    metric.total_duration_ms += duration; metric.average_duration_ms = metric.total_duration_ms / metric.calls;
    if (!success) metric.last_error = error ? error.slice(0, 160) : 'Unknown IPC error';
    metric.max_duration_ms = Math.max(metric.max_duration_ms ?? 0, duration); metric.samples = pushBounded(metric.samples, duration, MAX_FRONTEND_SAMPLES); metric.p95_duration_ms = p95(metric.samples); metric.last_seen = Date.now();
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') metric.hidden_calls += 1;
  }
  snapshot(): FrontendIpcMetric[] { return [...this.metrics.values()].map((metric) => ({ ...metric, samples: [...metric.samples] })).sort((a, b) => b.total_duration_ms - a.total_duration_ms); }
}

export const frontendIpcActivity = new FrontendIpcActivity();
