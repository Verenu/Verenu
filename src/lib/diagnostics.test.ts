import { describe, expect, it } from 'vitest';
import {
  filterLogs,
  formatBytes,
  formatDuration,
  frontendIpcActivity,
  p95,
  pushBounded,
  spanWidth,
} from './diagnostics';

const log = (level: string, message: string, subsystem = 'pipeline') => ({
  timestamp_ms: Date.parse('2026-09-09T00:00:00.000Z'),
  level,
  subsystem,
  operation: null,
  stage: null,
  message,
  trace_id: 'tr-a',
  session_id: null,
  duration_ms: null,
  outcome: null,
});

describe('diagnostics helpers', () => {
  it('keeps bounded history newest-first at the limit', () => {
    expect(pushBounded([1, 2, 3], 4, 3)).toEqual([2, 3, 4]);
  });

  it('filters logs by level, subsystem, query, and trace', () => {
    const logs = [log('info', 'audio accepted'), log('error', 'provider timeout', 'api')];
    expect(filterLogs(logs, 'timeout', 'all', 'all', '')).toHaveLength(1);
    expect(filterLogs(logs, '', 'error', 'api', 'tr-a')).toHaveLength(1);
    expect(filterLogs(logs, '', 'warn', 'all', '')).toHaveLength(0);
  });

  it('calculates a bounded percentile and timeline widths', () => {
    expect(p95([3, 1, 2, 10])).toBe(10);
    expect(spanWidth(25, 100)).toBe(25);
    expect(spanWidth(null, 100)).toBe(0);
  });

  it('formats unavailable and machine values without fake zeroes', () => {
    expect(formatDuration(null)).toBe('Unavailable');
    expect(formatBytes(null)).toBe('Unavailable');
    expect(formatBytes(1024 * 1024)).toBe('1.00 MiB');
    expect(formatDuration(1_250)).toBe('1.25 s');
  });

  it('keeps frontend IPC metrics metadata-only and bounded', () => {
    const started = frontendIpcActivity.start('diagnostics-test');
    frontendIpcActivity.finish('diagnostics-test', started, false);
    const metric = frontendIpcActivity.snapshot().find((item) => item.command === 'diagnostics-test');
    expect(metric?.calls).toBe(1);
    expect(metric?.failures).toBe(1);
    expect(metric?.samples.length).toBeLessThanOrEqual(64);
  });
});
