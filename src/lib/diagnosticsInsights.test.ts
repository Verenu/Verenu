import { describe, expect, it } from 'vitest';
import type { DiagnosticsSnapshot, PipelineTrace, SpanRecord } from './diagnostics';
import { filterLogs } from './diagnostics';
import { analyze, buildReport, stageStats, verdictFor } from './diagnosticsInsights';

const health = {
  initialized: true, profiler_enabled: true, retained_log_count: 0, retained_failure_count: 0,
  retained_trace_count: 0, retained_operation_count: 0, retained_resource_sample_count: 0,
  active_trace_count: 0, active_span_count: 0, total_logs_recorded: 0, total_failures_recorded: 0,
  total_traces_started: 0, total_traces_completed: 0, total_operations_recorded: 0,
  dropped_logs: 0, dropped_failures: 0, dropped_traces: 0, dropped_spans: 0,
  dropped_resource_samples: 0, collector_samples: 0, collector_duration_us_total: 0,
};

function snapshot(over: Partial<DiagnosticsSnapshot> = {}): DiagnosticsSnapshot {
  return {
    generated_at_ms: 1_000, profiler_enabled: true, profiling_recording: false, current_resource: null,
    resource_samples: [], latest_failures: [], failure_groups: [], active_pipelines: [], recent_pipelines: [],
    logs: [], operations: [], runtime: {}, health, ...over,
  };
}

function span(stage: string, duration_ms: number, over: Partial<SpanRecord> = {}): SpanRecord {
  return { span_id: stage, trace_id: 't', operation: stage, stage, started_at_ms: 0, ended_at_ms: duration_ms, duration_ms, outcome: 'success', ...over };
}

function trace(id: string, spans: SpanRecord[]): PipelineTrace {
  const total = spans.reduce((sum, s) => sum + (s.duration_ms ?? 0), 0);
  return { trace_id: id, root_operation: 'dictation', started_at_ms: 0, ended_at_ms: total, duration_ms: total, outcome: 'success', spans };
}

describe('stageStats', () => {
  it('averages per stage and ranks the slowest first', () => {
    const stats = stageStats([
      trace('a', [span('capture', 100), span('transcription', 900)]),
      trace('b', [span('capture', 300), span('transcription', 1100)]),
    ]);
    expect(stats.map((s) => s.stage)).toEqual(['transcription', 'capture']);
    expect(stats[0]).toMatchObject({ avgMs: 1000, maxMs: 1100, runs: 2 });
    expect(stats[0].share + stats[1].share).toBeCloseTo(1);
  });

  it('ignores traces that are still running', () => {
    const running = { ...trace('a', [span('capture', 100)]), ended_at_ms: null };
    expect(stageStats([running])).toEqual([]);
  });
});

describe('analyze', () => {
  it('is quiet for an empty healthy snapshot', () => {
    const findings = analyze(snapshot(), []);
    expect(findings).toEqual([]);
    expect(verdictFor(snapshot(), findings)).toBe('healthy');
  });

  it('flags a dominant stage in the last dictation', () => {
    const snap = snapshot({ recent_pipelines: [trace('a', [span('capture', 200), span('transcription', 1500)])] });
    const [finding] = analyze(snap, []);
    expect(finding).toMatchObject({ id: 'slow-stage', view: 'pipeline' });
    expect(finding.title).toContain('Transcription');
  });

  it('ranks recent failures above everything and fails the verdict', () => {
    const snap = snapshot({
      recent_pipelines: [trace('a', [span('transcription', 1500)])],
      failure_groups: [{ fingerprint: 'f1', count: 3, first_seen_ms: 0, last_seen_ms: 99_000, subsystem: 'api', stage: 'cleanup', representative_cause: 'timeout' }],
    });
    const findings = analyze(snap, [], 100_000);
    expect(findings[0]).toMatchObject({ severity: 'bad', metric: '3×' });
    expect(verdictFor(snap, findings)).toBe('failing');
  });

  it('detects memory growth and idle CPU', () => {
    const MiB = 1024 * 1024;
    const resource = (bytes: number, at: number) => ({
      sequence: at, observed_at_ms: at * 15_000,
      snapshot: { observed_at_ms: at * 15_000, resident_bytes: bytes, child_processes: [] },
    });
    const snap = snapshot({
      current_resource: { observed_at_ms: 0, cpu_percent: 40, resident_bytes: 900 * MiB, child_processes: [] },
      resource_samples: Array.from({ length: 24 }, (_, i) => resource((400 + i * 25) * MiB, i)),
    });
    const ids = analyze(snap, []).map((f) => f.id);
    expect(ids).toContain('memory-growth');
    expect(ids).toContain('idle-cpu');
  });

  it('ignores the diagnostics page polling itself', () => {
    const snap = snapshot({
      operations: [{ operation: 'get_diagnostics_snapshot', calls: 500, success_count: 500, failure_count: 0, cancelled_count: 0, skipped_count: 0, total_duration_ms: 9000, calls_per_minute: 400, currently_running: 0 }],
    });
    expect(analyze(snap, [])).toEqual([]);
  });
});

describe('buildReport', () => {
  it('produces a compact text report with the last pipeline and recent errors', () => {
    const snap = snapshot({
      recent_pipelines: [trace('trace-1', [span('transcription', 900)])],
      logs: [{ timestamp_ms: 0, level: 'error', subsystem: 'verenu::api', message: 'timeout', trace_id: 'trace-1' }],
    });
    const report = buildReport(snap, [], analyze(snap, []));
    expect(report).toContain('## Last pipeline');
    expect(report).toContain('trace trace-1');
    expect(report).toContain('ERROR api: timeout [trace-1]');
  });
});

describe('filterLogs problems level', () => {
  it('keeps only warnings and errors', () => {
    const entry = (level: string) => ({ timestamp_ms: 0, level, subsystem: 's', message: level });
    const kept = filterLogs([entry('info'), entry('warn'), entry('error'), entry('debug')], '', 'problems', 'all', '');
    expect(kept.map((e) => e.level)).toEqual(['warn', 'error']);
  });
});
