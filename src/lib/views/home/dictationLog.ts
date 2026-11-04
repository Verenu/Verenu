import { fmtDuration } from '../insights/helpers';
import type { Entry } from './helpers';

/**
 * Per-dictation log returned by the `get_dictation_log` command.
 * Every field except `entry_id` is optional so older rows, partial pipelines,
 * and future fields never break the dialog.
 */
export interface DictationLogStage {
  stage: string;
  provider?: string | null;
  model?: string | null;
  local?: boolean | null;
  duration_ms?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  cost_usd?: number | null;
  status?: 'ok' | 'failed' | 'skipped' | null;
  error?: string | null;
}

export interface DictationLog {
  entry_id: number;
  total_ms?: number | null;
  audio_ms?: number | null;
  estimated_cost_usd?: number | null;
  stages?: DictationLogStage[] | null;
  failures?: { stage: string; message: string }[] | null;
  fallbacks?: { stage: string; message?: string | null }[] | null;
}

export interface LogRow {
  label: string;
  value: string;
}

export interface LogSummary {
  headline: string;
  rows: LogRow[];
  failures: string[];
  fallbacks: string[];
  stages: DictationLogStage[];
  hasLog: boolean;
}

const STAGE_LABELS: Record<string, string> = {
  permission: 'Permission check',
  capture: 'Recording',
  vad: 'Speech detection',
  preprocessing: 'Audio preparation',
  transcription: 'Transcription',
  dual_transcription: 'Second transcription',
  cleanup: 'Cleanup',
  formatting: 'Formatting',
  injection: 'Insert text',
  insertion: 'Insert text',
  clipboard: 'Clipboard',
  local_model: 'Local model',
  sync: 'Sync',
};

const ON_DEVICE_PROVIDERS = new Set(['local', 'apple-intelligence']);

/** True only when the log says so, or the provider is known to run on this device. */
export function isOnDevice(stage: DictationLogStage): boolean {
  if (stage.local != null) return stage.local;
  return stage.provider ? ON_DEVICE_PROVIDERS.has(stage.provider) : false;
}

/** A request was sent only when a remote provider is recorded for the step. */
export function requestTarget(stage: DictationLogStage): string | null {
  if (!stage.provider || isOnDevice(stage)) return null;
  return stage.provider;
}

/** The backend reports a used fallback through `failures`; it is not a failure of the dictation. */
const FALLBACK_MESSAGE = /fallback was used/i;

export function stageLabel(stage: string): string {
  return STAGE_LABELS[stage] ?? stage.replace(/[_-]+/g, ' ').replace(/^\w/, (c) => c.toUpperCase());
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/** Validates untrusted IPC data. Returns null when nothing usable arrived. */
export function parseDictationLog(raw: unknown): DictationLog | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const entryId = finiteNumber(value.entry_id);
  if (entryId === null) return null;

  const stages: DictationLogStage[] = Array.isArray(value.stages)
    ? value.stages.flatMap((item): DictationLogStage[] => {
        if (!item || typeof item !== 'object') return [];
        const s = item as Record<string, unknown>;
        const stage = text(s.stage);
        if (!stage) return [];
        const status = s.status === 'ok' || s.status === 'failed' || s.status === 'skipped' ? s.status : null;
        return [{
          stage,
          provider: text(s.provider),
          model: text(s.model),
          local: typeof s.local === 'boolean' ? s.local : null,
          duration_ms: finiteNumber(s.duration_ms),
          input_tokens: finiteNumber(s.input_tokens),
          output_tokens: finiteNumber(s.output_tokens),
          cost_usd: finiteNumber(s.cost_usd),
          status,
          error: text(s.error),
        }];
      })
    : [];

  const failures = Array.isArray(value.failures)
    ? value.failures.flatMap((item) => {
        if (!item || typeof item !== 'object') return [];
        const f = item as Record<string, unknown>;
        const message = text(f.message);
        return message ? [{ stage: text(f.stage) ?? 'pipeline', message }] : [];
      })
    : [];

  const fallbacks = Array.isArray(value.fallbacks)
    ? value.fallbacks.flatMap((item) => {
        if (!item || typeof item !== 'object') return [];
        const f = item as Record<string, unknown>;
        const stage = text(f.stage);
        return stage ? [{ stage, message: text(f.message) }] : [];
      })
    : [];

  return {
    entry_id: entryId,
    total_ms: finiteNumber(value.total_ms),
    audio_ms: finiteNumber(value.audio_ms),
    estimated_cost_usd: finiteNumber(value.estimated_cost_usd),
    stages,
    failures,
    fallbacks,
  };
}

export function fmtMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

export function fmtCost(usd: number): string {
  if (usd === 0) return '$0.00';
  if (usd < 0.0001) return '< $0.0001';
  return `$${usd.toFixed(usd < 0.01 ? 4 : 2)}`;
}

export function fmtTokens(stage: DictationLogStage): string | null {
  const input = stage.input_tokens;
  const output = stage.output_tokens;
  if (input == null && output == null) return null;
  const parts: string[] = [];
  if (input != null) parts.push(`${input.toLocaleString('en-US')} in`);
  if (output != null) parts.push(`${output.toLocaleString('en-US')} out`);
  return parts.join(', ');
}

export function stageModel(stage: DictationLogStage): string | null {
  const name = stage.model ?? stage.provider ?? null;
  if (!name) return null;
  return stage.local ? `${name} (on this device)` : name;
}

/** Builds the dialog content. `log` may be null for entries recorded before logs existed. */
export function summarizeLog(entry: Pick<Entry, 'duration_ms'>, log: DictationLog | null): LogSummary {
  const audioMs = finiteNumber(log?.audio_ms) ?? finiteNumber(entry.duration_ms);
  const stages = log?.stages ?? [];
  const allReported = log?.failures ?? [];
  const reported = allReported.filter((f) => !FALLBACK_MESSAGE.test(f.message));
  const fallbacks = [
    ...(log?.fallbacks ?? []).map((f) => f.stage),
    ...allReported.filter((f) => FALLBACK_MESSAGE.test(f.message)).map((f) => f.stage),
  ]
    .filter((stage, i, all) => all.indexOf(stage) === i)
    .map((stage) => `${stageLabel(stage)}: a backup was used.`);
  const failures = [
    ...reported.map((f) => `${stageLabel(f.stage)}: ${f.message}`),
    ...stages
      .filter((s) => s.status === 'failed' && s.error && !reported.some((f) => f.message === s.error))
      .map((s) => `${stageLabel(s.stage)}: ${s.error}`),
  ];

  const rows: LogRow[] = [];
  if (audioMs !== null && audioMs > 0) rows.push({ label: 'Dictation length', value: fmtDuration(audioMs) });
  if (log?.total_ms != null) rows.push({ label: 'Time to finish', value: fmtMs(log.total_ms) });
  const transcription = stages.find((s) => s.stage === 'transcription');
  const cleanup = stages.find((s) => s.stage === 'cleanup' && s.status !== 'skipped');
  const transcriptionModel = transcription && stageModel(transcription);
  const cleanupModel = cleanup && stageModel(cleanup);
  if (transcriptionModel) rows.push({ label: 'Transcribed with', value: transcriptionModel });
  if (cleanupModel) rows.push({ label: 'Cleaned up with', value: cleanupModel });
  if (log?.estimated_cost_usd != null) rows.push({ label: 'Estimated cost', value: fmtCost(log.estimated_cost_usd) });

  let headline: string;
  if (!log) headline = 'No log was saved for this dictation.';
  else if (failures.length > 0) headline = 'This dictation had a problem.';
  else if (rows.length === 0 && stages.length === 0) headline = 'This dictation finished without recorded details.';
  else headline = 'This dictation finished normally.';

  return { headline, rows, failures, fallbacks, stages, hasLog: log !== null };
}

/** Plain-text copy of the technical detail. Never includes dictated or cleaned text. */
export function formatLogForCopy(summary: LogSummary): string {
  const lines = summary.rows.map((r) => `${r.label}: ${r.value}`);
  for (const s of summary.stages) {
    const parts = [stageLabel(s.stage)];
    const model = stageModel(s);
    if (model) parts.push(model);
    if (s.duration_ms != null) parts.push(fmtMs(s.duration_ms));
    const tokens = fmtTokens(s);
    if (tokens) parts.push(`${tokens} tokens`);
    if (s.cost_usd != null) parts.push(fmtCost(s.cost_usd));
    const target = requestTarget(s);
    if (target) parts.push(`request sent to ${target}`);
    if (s.status && s.status !== 'ok') parts.push(s.status);
    lines.push(parts.join(' | '));
  }
  for (const f of summary.fallbacks) lines.push(`Fallback: ${f}`);
  for (const f of summary.failures) lines.push(`Failure: ${f}`);
  return lines.join('\n');
}
