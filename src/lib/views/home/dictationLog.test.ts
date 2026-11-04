import { describe, expect, it } from 'vitest';
import {
  fmtCost,
  formatLogForCopy,
  isOnDevice,
  parseDictationLog,
  requestTarget,
  summarizeLog,
} from './dictationLog';

const fullLog = {
  entry_id: 7,
  total_ms: 1840,
  audio_ms: 12_000,
  estimated_cost_usd: 0.00123,
  stages: [
    { stage: 'transcription', provider: 'openai', model: 'gpt-4o-mini-transcribe', duration_ms: 900, cost_usd: 0.0006, status: 'ok' },
    { stage: 'cleanup', provider: 'anthropic', model: 'claude-haiku', duration_ms: 700, input_tokens: 420, output_tokens: 95, status: 'ok' },
  ],
};

describe('parseDictationLog', () => {
  it('rejects missing or malformed payloads', () => {
    expect(parseDictationLog(null)).toBeNull();
    expect(parseDictationLog('nope')).toBeNull();
    expect(parseDictationLog({ stages: [] })).toBeNull();
    expect(parseDictationLog({ entry_id: -1 })).toBeNull();
  });

  it('drops invalid stage fields instead of throwing', () => {
    const log = parseDictationLog({
      entry_id: 1,
      total_ms: 'fast',
      stages: [{ stage: 'cleanup', duration_ms: Number.NaN, input_tokens: 5, status: 'weird' }, 42, { provider: 'x' }],
    });
    expect(log?.total_ms).toBeNull();
    expect(log?.stages).toHaveLength(1);
    expect(log?.stages?.[0]).toMatchObject({ stage: 'cleanup', duration_ms: null, input_tokens: 5, status: null });
  });
});

describe('summarizeLog', () => {
  it('summarizes a complete log in plain language', () => {
    const summary = summarizeLog({ duration_ms: 12_000 }, parseDictationLog(fullLog));
    expect(summary.headline).toBe('This dictation finished normally.');
    expect(summary.rows).toEqual([
      { label: 'Dictation length', value: '12s' },
      { label: 'Time to finish', value: '1.84 s' },
      { label: 'Transcribed with', value: 'gpt-4o-mini-transcribe' },
      { label: 'Cleaned up with', value: 'claude-haiku' },
      { label: 'Estimated cost', value: '$0.0012' },
    ]);
  });

  it('handles older entries with no log by falling back to stored duration', () => {
    const summary = summarizeLog({ duration_ms: 4000 }, null);
    expect(summary.hasLog).toBe(false);
    expect(summary.headline).toBe('No log was saved for this dictation.');
    expect(summary.rows).toEqual([{ label: 'Dictation length', value: '4s' }]);
  });

  it('shows nothing extra when an old entry has no duration', () => {
    const summary = summarizeLog({ duration_ms: null }, null);
    expect(summary.rows).toEqual([]);
    expect(summary.failures).toEqual([]);
  });

  it('reports failures once even when stage and failure list repeat them', () => {
    const summary = summarizeLog({}, parseDictationLog({
      entry_id: 2,
      failures: [{ stage: 'cleanup', message: 'Rate limited' }],
      stages: [{ stage: 'cleanup', status: 'failed', error: 'Rate limited' }, { stage: 'transcription', status: 'failed', error: 'Timeout' }],
    }));
    expect(summary.headline).toBe('This dictation had a problem.');
    expect(summary.failures).toEqual(['Cleanup: Rate limited', 'Transcription: Timeout']);
  });

  it('marks on-device models and ignores skipped cleanup', () => {
    const summary = summarizeLog({}, parseDictationLog({
      entry_id: 3,
      stages: [
        { stage: 'transcription', model: 'whisper-small', local: true },
        { stage: 'cleanup', model: 'skipped-model', status: 'skipped' },
      ],
    }));
    expect(summary.rows.map((r) => r.value)).toEqual(['whisper-small (on this device)']);
  });
});

describe('formatting', () => {
  it('formats tiny and zero costs', () => {
    expect(fmtCost(0)).toBe('$0.00');
    expect(fmtCost(0.00001)).toBe('< $0.0001');
    expect(fmtCost(1.5)).toBe('$1.50');
  });

  it('copies stage detail with tokens and no dictated text', () => {
    const copy = formatLogForCopy(summarizeLog({}, parseDictationLog(fullLog)));
    expect(copy).toContain('Cleanup | claude-haiku | 700 ms | 420 in, 95 out tokens');
    expect(copy).not.toMatch(/clean_text|raw_text/);
  });
});

describe('fallbacks and requests', () => {
  it('treats the backend fallback note as a fallback, not a failure', () => {
    const summary = summarizeLog({}, parseDictationLog({
      entry_id: 4,
      stages: [{ stage: 'transcription', duration_ms: 300 }],
      failures: [{ stage: 'cleanup', message: 'The primary option failed; a fallback was used.' }],
    }));
    expect(summary.failures).toEqual([]);
    expect(summary.fallbacks).toEqual(['Cleanup: a backup was used.']);
    expect(summary.headline).toBe('This dictation finished normally.');
  });

  it('reports a request only for remote providers and never invents tokens', () => {
    const [remote, local, none] = [
      { stage: 'cleanup', provider: 'groq', model: 'm' },
      { stage: 'transcription', provider: 'local', model: 'w' },
      { stage: 'capture', duration_ms: 5 },
    ];
    expect(requestTarget(remote)).toBe('groq');
    expect(isOnDevice(local)).toBe(true);
    expect(requestTarget(local)).toBeNull();
    expect(requestTarget(none)).toBeNull();
    const copy = formatLogForCopy(summarizeLog({}, parseDictationLog({ entry_id: 5, stages: [remote] })));
    expect(copy).toContain('request sent to groq');
    expect(copy).not.toContain('tokens');
  });
});
