import {
  formatBytes,
  formatDuration,
  p95,
  type DiagnosticsSnapshot,
  type FrontendIpcMetric,
  type OperationMetric,
  type PipelineTrace,
  type StructuredLogEntry,
} from './diagnostics';

export type FindingSeverity = 'bad' | 'warn' | 'info';
export type FindingView = 'pipeline' | 'failures' | 'logs' | 'resources';

export interface Finding {
  id: string;
  severity: FindingSeverity;
  title: string;
  metric: string;
  detail: string;
  view: FindingView;
}

export interface StageStat {
  stage: string;
  runs: number;
  avgMs: number;
  p95Ms: number;
  maxMs: number;
  /** Share of the summed average stage time, 0-1. */
  share: number;
}

const SEVERITY_RANK: Record<FindingSeverity, number> = { bad: 0, warn: 1, info: 2 };
const SELF_OPERATIONS = /^(get_diagnostics_snapshot|set_diagnostics_|subscribe_log_stream|unsubscribe_log_stream|frontend\.invoke:(get_diagnostics_snapshot|set_diagnostics_))/;

export function stageDurations(trace: PipelineTrace): Map<string, number> {
  const out = new Map<string, number>();
  for (const span of trace.spans) {
    const stage = span.stage;
    if (!stage || span.duration_ms == null) continue;
    out.set(stage, (out.get(stage) ?? 0) + span.duration_ms);
  }
  return out;
}

export function stageStats(traces: readonly PipelineTrace[]): StageStat[] {
  const samples = new Map<string, number[]>();
  for (const trace of traces) {
    if (trace.ended_at_ms == null) continue;
    for (const [stage, ms] of stageDurations(trace)) {
      const list = samples.get(stage) ?? [];
      list.push(ms);
      samples.set(stage, list);
    }
  }
  const rows = [...samples].map(([stage, values]) => ({
    stage,
    runs: values.length,
    avgMs: values.reduce((sum, v) => sum + v, 0) / values.length,
    p95Ms: p95(values) ?? 0,
    maxMs: Math.max(...values),
    share: 0,
  }));
  const total = rows.reduce((sum, row) => sum + row.avgMs, 0);
  for (const row of rows) row.share = total > 0 ? row.avgMs / total : 0;
  return rows.sort((a, b) => b.avgMs - a.avgMs);
}

export function stageLabel(stage: string): string {
  const clean = stage.replace(/[_-]+/g, ' ').trim();
  return clean ? clean[0].toUpperCase() + clean.slice(1) : stage;
}

export function completedTraces(snapshot: DiagnosticsSnapshot): PipelineTrace[] {
  return snapshot.recent_pipelines.filter((trace) => trace.ended_at_ms != null);
}

export function whereLabel(item: { operation?: string | null; stage?: string | null; error_category?: string | null }): string {
  return item.operation ?? item.stage ?? item.error_category ?? 'event';
}

export function operationRows(snapshot: DiagnosticsSnapshot, frontend: readonly FrontendIpcMetric[]): OperationMetric[] {
  const mapped: OperationMetric[] = frontend.map((m) => ({
    operation: `invoke:${m.command}`,
    calls: m.calls,
    success_count: m.calls - m.failures,
    failure_count: m.failures,
    cancelled_count: 0,
    skipped_count: 0,
    total_duration_ms: m.total_duration_ms,
    average_duration_ms: m.average_duration_ms,
    recent_duration_ms: m.samples[m.samples.length - 1] ?? null,
    p95_duration_ms: m.p95_duration_ms,
    max_duration_ms: m.max_duration_ms,
    calls_per_minute: 0,
    currently_running: m.currently_running,
    last_called_at_ms: m.last_seen,
  }));
  return [...snapshot.operations, ...mapped]
    .filter((row) => !SELF_OPERATIONS.test(row.operation) && !SELF_OPERATIONS.test(row.operation.replace(/^invoke:/, '')))
    .sort((a, b) => b.total_duration_ms - a.total_duration_ms);
}

function trendGrowthBytes(snapshot: DiagnosticsSnapshot): { grown: number; spanMs: number; ratio: number } | null {
  const samples = snapshot.resource_samples.filter((s) => s.snapshot.resident_bytes != null);
  if (samples.length < 20) return null;
  const first = samples[0];
  const last = samples[samples.length - 1];
  const start = first.snapshot.resident_bytes!;
  const end = last.snapshot.resident_bytes!;
  return { grown: end - start, spanMs: last.observed_at_ms - first.observed_at_ms, ratio: start > 0 ? end / start - 1 : 0 };
}

function formatSpan(ms: number): string {
  if (ms < 90_000) return `${Math.round(ms / 1000)}s`;
  if (ms < 5_400_000) return `${Math.round(ms / 60_000)} min`;
  return `${(ms / 3_600_000).toFixed(1)} h`;
}

function repeatedLogs(logs: readonly StructuredLogEntry[]): { message: string; count: number } | null {
  const counts = new Map<string, number>();
  for (const entry of logs) {
    const key = `${entry.subsystem}|${entry.message.slice(0, 140)}`;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  let top: { message: string; count: number } | null = null;
  for (const [key, count] of counts) {
    if (!top || count > top.count) top = { message: key.slice(key.indexOf('|') + 1), count };
  }
  return top && top.count >= 20 ? top : null;
}

export function analyze(snapshot: DiagnosticsSnapshot, frontend: readonly FrontendIpcMetric[], now = Date.now()): Finding[] {
  const findings: Finding[] = [];
  const add = (finding: Finding) => findings.push(finding);
  const idle = snapshot.active_pipelines.length === 0 && snapshot.runtime.audio?.active !== true;
  const traces = completedTraces(snapshot);

  const isRecent = (lastSeen: number) => now - lastSeen < 10 * 60_000;
  const groups = [...snapshot.failure_groups].sort(
    (a, b) => Number(isRecent(b.last_seen_ms)) - Number(isRecent(a.last_seen_ms)) || b.count - a.count || b.last_seen_ms - a.last_seen_ms,
  );
  for (const group of groups.slice(0, 3)) {
    const recent = isRecent(group.last_seen_ms);
    add({
      id: `failure:${group.fingerprint}`,
      severity: recent ? 'bad' : 'warn',
      title: `${whereLabel(group)} failing`,
      metric: `${group.count}×`,
      detail: group.representative_cause,
      view: 'failures',
    });
  }

  const last = traces[traces.length - 1];
  if (last?.duration_ms) {
    const [stage, ms] = [...stageDurations(last)].sort((a, b) => b[1] - a[1])[0] ?? [];
    if (stage && ms && ms / last.duration_ms >= 0.5 && ms >= 800) {
      add({
        id: 'slow-stage',
        severity: 'warn',
        title: `${stageLabel(stage)} dominates the last dictation`,
        metric: `${Math.round((ms / last.duration_ms) * 100)}%`,
        detail: `${formatDuration(ms)} of ${formatDuration(last.duration_ms)}. Cheapest win is a faster model or provider for this stage.`,
        view: 'pipeline',
      });
    }
  }

  const stats = stageStats(traces);
  for (const stat of stats) {
    if (stat.runs >= 4 && stat.avgMs >= 400 && stat.p95Ms > stat.avgMs * 2.5) {
      add({
        id: `variance:${stat.stage}`,
        severity: 'info',
        title: `${stageLabel(stat.stage)} is inconsistent`,
        metric: formatDuration(stat.p95Ms),
        detail: `p95 is ${(stat.p95Ms / stat.avgMs).toFixed(1)}× the ${formatDuration(stat.avgMs)} average across ${stat.runs} runs.`,
        view: 'pipeline',
      });
    }
  }

  const growth = trendGrowthBytes(snapshot);
  if (growth && growth.grown > 100 * 1024 * 1024 && growth.ratio > 0.25) {
    add({
      id: 'memory-growth',
      severity: 'warn',
      title: 'Memory keeps climbing',
      metric: `+${formatBytes(growth.grown)}`,
      detail: `Resident memory grew ${Math.round(growth.ratio * 100)}% over ${formatSpan(growth.spanMs)}. Possible leak.`,
      view: 'resources',
    });
  }

  const resource = snapshot.current_resource;
  if (idle && resource?.cpu_percent != null && resource.cpu_percent > 15) {
    add({
      id: 'idle-cpu',
      severity: 'warn',
      title: 'Busy while idle',
      metric: `${resource.cpu_percent.toFixed(0)}% CPU`,
      detail: 'No dictation is running, so something is burning CPU in the background.',
      view: 'resources',
    });
  }

  const total = resource?.resident_bytes ?? 0;
  if (idle && total > 0) {
    const heavy = [...(resource?.child_processes ?? [])]
      .filter((child) => (child.resident_bytes ?? 0) > 300 * 1024 * 1024 && (child.resident_bytes ?? 0) / total > 0.4)
      .sort((a, b) => (b.resident_bytes ?? 0) - (a.resident_bytes ?? 0))[0];
    if (heavy) {
      add({
        id: `heavy-child:${heavy.label}`,
        severity: 'info',
        title: `${heavy.label} holds memory while idle`,
        metric: formatBytes(heavy.resident_bytes),
        detail: `${Math.round(((heavy.resident_bytes ?? 0) / total) * 100)}% of the process tree. Unload it when not dictating to free RAM.`,
        view: 'resources',
      });
    }
  }

  const ops = operationRows(snapshot, frontend);
  const chatty = ops.filter((op) => op.calls_per_minute >= 60).sort((a, b) => b.calls_per_minute - a.calls_per_minute)[0];
  if (chatty) {
    add({
      id: `chatty:${chatty.operation}`,
      severity: 'warn',
      title: `${chatty.operation} is chatty`,
      metric: `${Math.round(chatty.calls_per_minute)}/min`,
      detail: `${formatDuration(chatty.total_duration_ms)} total across ${chatty.calls} calls. Poll less or push events instead.`,
      view: 'resources',
    });
  }
  const slow = ops
    .filter((op) => op.calls >= 3 && (op.p95_duration_ms ?? 0) > 1500 && !op.operation.startsWith('pipeline'))
    .sort((a, b) => (b.p95_duration_ms ?? 0) - (a.p95_duration_ms ?? 0))[0];
  if (slow) {
    add({
      id: `slow-op:${slow.operation}`,
      severity: 'info',
      title: `${slow.operation} is slow`,
      metric: formatDuration(slow.p95_duration_ms),
      detail: `p95 over ${slow.calls} calls, average ${formatDuration(slow.average_duration_ms)}.`,
      view: 'resources',
    });
  }

  const spam = repeatedLogs(snapshot.logs);
  if (spam) {
    add({
      id: 'log-spam',
      severity: 'info',
      title: 'One log line is repeating',
      metric: `${spam.count}×`,
      detail: spam.message.slice(0, 120),
      view: 'logs',
    });
  }

  const dropped = snapshot.health.dropped_logs + snapshot.health.dropped_failures + snapshot.health.dropped_traces;
  if (dropped > 0) {
    add({
      id: 'dropped',
      severity: 'info',
      title: 'Older diagnostics were dropped',
      metric: `${dropped}`,
      detail: 'Retention is bounded. Copy the report before it rolls off.',
      view: 'logs',
    });
  }

  return findings.sort((a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity]);
}

export type Verdict = 'waiting' | 'healthy' | 'attention' | 'failing';

export function verdictFor(snapshot: DiagnosticsSnapshot, findings: readonly Finding[]): Verdict {
  if (snapshot.generated_at_ms === 0) return 'waiting';
  if (findings.some((f) => f.severity === 'bad')) return 'failing';
  if (findings.some((f) => f.severity === 'warn')) return 'attention';
  return 'healthy';
}

function clock(ms: number): string {
  return new Date(ms).toISOString().slice(11, 19);
}

export function formatLogLine(entry: StructuredLogEntry): string {
  const where = [entry.subsystem.replace(/^verenu::/, ''), entry.operation ?? entry.stage].filter(Boolean).join(' ');
  const trace = entry.trace_id ? ` [${entry.trace_id}]` : '';
  const duration = entry.duration_ms != null ? ` (${formatDuration(entry.duration_ms)})` : '';
  return `${clock(entry.timestamp_ms)} ${entry.level.toUpperCase().padEnd(5)} ${where}: ${entry.message}${duration}${trace}`;
}

export function formatLogs(entries: readonly StructuredLogEntry[]): string {
  return entries.map(formatLogLine).join('\n');
}

export function formatTrace(trace: PipelineTrace): string {
  const lines = [`trace ${trace.trace_id} ${trace.outcome} ${formatDuration(trace.duration_ms)}`];
  for (const span of trace.spans) {
    const extras = [
      span.provider,
      span.model,
      span.cache && `cache ${span.cache}`,
      (span.retry_number ?? 0) > 0 && `retry ${span.retry_number}`,
      (span.fallback_number ?? 0) > 0 && `fallback ${span.fallback_number}`,
      span.error_category,
    ].filter(Boolean);
    lines.push(`  ${(span.stage ?? span.operation).padEnd(14)} ${formatDuration(span.duration_ms).padStart(9)} ${span.outcome}${extras.length ? ` · ${extras.join(' · ')}` : ''}`);
  }
  return lines.join('\n');
}

export function formatFailures(snapshot: DiagnosticsSnapshot): string {
  const groups = [...snapshot.failure_groups].sort((a, b) => b.count - a.count).slice(0, 10);
  const lines = groups.map((g) => {
    const model = [g.provider, g.model].filter(Boolean).join('/');
    return `${g.count}× ${g.subsystem.replace(/^verenu::/, '')} ${whereLabel(g)}${model ? ` (${model})` : ''}: ${g.representative_cause} [${g.fingerprint}] last ${clock(g.last_seen_ms)}${g.last_trace_id ? ` trace ${g.last_trace_id}` : ''}`;
  });
  return lines.join('\n');
}

/** Compact plain text meant to be pasted straight into an agent. Metadata only. */
export function buildReport(snapshot: DiagnosticsSnapshot, frontend: readonly FrontendIpcMetric[], findings: readonly Finding[], now = Date.now()): string {
  const out: string[] = [`# Verenu diagnostics ${new Date(now).toISOString()}`];

  out.push('', '## Findings');
  out.push(...(findings.length ? findings.map((f) => `- [${f.severity}] ${f.title} (${f.metric}): ${f.detail}`) : ['- none']));

  const resource = snapshot.current_resource;
  if (resource) {
    out.push('', '## Resources');
    out.push(`cpu ${resource.cpu_percent?.toFixed(1) ?? '?'}% · rss ${formatBytes(resource.resident_bytes)} (peak ${formatBytes(resource.peak_resident_bytes)}) · procs ${resource.process_count ?? '?'} · threads ${resource.thread_count ?? '?'}`);
    for (const child of [...resource.child_processes].sort((a, b) => (b.resident_bytes ?? 0) - (a.resident_bytes ?? 0)).slice(0, 5)) {
      out.push(`  ${child.label}: ${formatBytes(child.resident_bytes)} cpu ${child.cpu_percent?.toFixed(1) ?? '?'}%`);
    }
  }

  const stats = stageStats(completedTraces(snapshot));
  if (stats.length) {
    out.push('', '## Stage timing (completed runs)');
    for (const s of stats) out.push(`${stageLabel(s.stage).padEnd(14)} avg ${formatDuration(s.avgMs)} · p95 ${formatDuration(s.p95Ms)} · max ${formatDuration(s.maxMs)} · ${s.runs} runs`);
  }

  const last = snapshot.recent_pipelines[snapshot.recent_pipelines.length - 1];
  const bad = [...snapshot.recent_pipelines].reverse().find((t) => t.outcome === 'failure');
  if (snapshot.active_pipelines.length) out.push('', '## Active pipeline', formatTrace(snapshot.active_pipelines[0]));
  if (last) out.push('', '## Last pipeline', formatTrace(last));
  if (bad && bad !== last) out.push('', '## Last failed pipeline', formatTrace(bad));

  if (snapshot.failure_groups.length) out.push('', '## Failures', formatFailures(snapshot));

  const ops = operationRows(snapshot, frontend).slice(0, 8);
  if (ops.length) {
    out.push('', '## Heaviest operations');
    for (const op of ops) out.push(`${op.operation}: ${op.calls} calls · ${formatDuration(op.total_duration_ms)} total · p95 ${formatDuration(op.p95_duration_ms)} · ${op.failure_count} failed`);
  }

  const problems = snapshot.logs.filter((e) => e.level === 'error' || e.level === 'warn').slice(-40);
  if (problems.length) out.push('', '## Recent warnings and errors', formatLogs(problems));

  return out.join('\n');
}

export function clockTime(ms: number | null | undefined): string {
  return ms ? new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—';
}

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 5) return 'now';
  if (s < 90) return `${s}s ago`;
  if (s < 5400) return `${Math.round(s / 60)}m ago`;
  if (s < 129_600) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86_400)}d ago`;
}

export function subsystemLabel(value: string): string {
  return value.replace(/^verenu::/, '').split('::').map((part) => part.replace(/_/g, ' ')).join(' / ');
}
