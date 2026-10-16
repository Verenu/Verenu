import type { InsightsProviderUsage } from './types';


type Rate =
  | { kind: 'audio'; usd_per_hour: number }
  | { kind: 'token'; usd_per_m_in: number; usd_per_m_out: number };

export interface PricingRate {
  model_id: string;
  prompt_usd_per_token: number;
  completion_usd_per_token: number;
}

export interface PricingSnapshot {
  fetched_at: number;
  rates: PricingRate[];
}

export const PRICING: Record<string, Rate> = {
  'whisper-large-v3-turbo': { kind: 'audio', usd_per_hour: 0.04 },
  'whisper-large-v3': { kind: 'audio', usd_per_hour: 0.111 },
  'distil-whisper-large-v3-en': { kind: 'audio', usd_per_hour: 0.02 },
  'whisper-1': { kind: 'audio', usd_per_hour: 0.36 },
  'gpt-4o-transcribe': { kind: 'audio', usd_per_hour: 0.36 },
  'gpt-4o-mini-transcribe': { kind: 'audio', usd_per_hour: 0.18 },

  'llama-3.3-70b-versatile': { kind: 'token', usd_per_m_in: 0.59, usd_per_m_out: 0.79 },
  'qwen3.6-27b': { kind: 'token', usd_per_m_in: 0.60, usd_per_m_out: 3.00 },
  'qwen3.8-27b': { kind: 'token', usd_per_m_in: 0.80, usd_per_m_out: 4.00 },
  'gpt-4o-mini': { kind: 'token', usd_per_m_in: 0.15, usd_per_m_out: 0.6 },
  'gemini-3.5-flash': { kind: 'token', usd_per_m_in: 2.70, usd_per_m_out: 16.20 },
  'gemini-3.5-flash-lite': { kind: 'token', usd_per_m_in: 0.30, usd_per_m_out: 2.50 },
  'gemini-2.5-flash': { kind: 'token', usd_per_m_in: 0.15, usd_per_m_out: 1.25 },
  'gemini-2.5-flash-lite': { kind: 'token', usd_per_m_in: 0.10, usd_per_m_out: 0.40 },
};

const TASK_OVERRIDES: Record<string, Rate> = {
  'gemini-3.5-transcribe#transcription': { kind: 'audio', usd_per_hour: 0.306 },
};

const CHARS_PER_TOKEN = 4;

function normalizeModelId(model: string | null | undefined): string {
  return String(model ?? '').trim().toLowerCase();
}

function shortModelId(model: string): string {
  return model.replace(/^.*\//, '');
}

function indexRates(snapshot: PricingSnapshot | null) {
  const exact = new Map<string, number>();
  const short = new Map<string, number>();
  snapshot?.rates.forEach((rate, i) => {
    const id = normalizeModelId(rate.model_id);
    if (!exact.has(id)) exact.set(id, i);
    const name = shortModelId(id);
    if (!short.has(name)) short.set(name, i);
  });
  return { rates: snapshot?.rates ?? [], exact, short };
}

type RateIndex = ReturnType<typeof indexRates>;

function lookupOpenRouterRate(usage: InsightsProviderUsage, index: RateIndex): Rate | null {
  if (usage.task === 'transcription') return null;
  const model = normalizeModelId(usage.model);
  const short = shortModelId(model);
  const provider = String(usage.provider ?? '').trim().toLowerCase();
  const first = Math.min(
    index.exact.get(model) ?? Infinity,
    (provider ? index.exact.get(`${provider}/${short}`) : undefined) ?? Infinity,
  );
  const published = index.rates[Number.isFinite(first) ? first : (index.short.get(short) ?? -1)];
  if (!published) return null;
  return {
    kind: 'token',
    usd_per_m_in: published.prompt_usd_per_token * 1e6,
    usd_per_m_out: published.completion_usd_per_token * 1e6,
  };
}

function lookupRate(usage: InsightsProviderUsage, index: RateIndex): Rate | null {
  const key = shortModelId(normalizeModelId(usage.model));
  const openRouterRate = lookupOpenRouterRate(usage, index);
  if (openRouterRate) return openRouterRate;
  const task = String(usage.task ?? '').trim().toLowerCase();
  return TASK_OVERRIDES[`${key}#${task}`] ?? PRICING[key] ?? null;
}

function modelCost(usage: InsightsProviderUsage, index: RateIndex): number | null {
  if (usage.provider === 'apple-intelligence' && usage.task === 'cleanup'
    && normalizeModelId(usage.model) === 'system') return 0;
  const rate = lookupRate(usage, index);
  if (!rate) return null;
  if (rate.kind === 'audio') {
    return (usage.audio_ms / 3_600_000) * rate.usd_per_hour;
  }
  const inTokens = usage.input_chars / CHARS_PER_TOKEN;
  const outTokens = usage.output_chars / CHARS_PER_TOKEN;
  return (inTokens / 1e6) * rate.usd_per_m_in + (outTokens / 1e6) * rate.usd_per_m_out;
}

export interface CostRow extends InsightsProviderUsage {
  cost: number | null;
  /** Fraction of the priced total, 0–1. Zero for unpriced models. */
  share: number;
}

export interface CostSummary {
  rows: CostRow[];
  total: number;
  /** True when at least one model had no rate, so `total` understates reality. */
  hasUnpriced: boolean;
}

export function estimateCost(providers: InsightsProviderUsage[], snapshot: PricingSnapshot | null = null): CostSummary {
  const index = indexRates(providers.some((usage) => usage.task !== 'transcription') ? snapshot : null);
  const priced = providers.map((usage) => ({ usage, cost: modelCost(usage, index) }));
  const total = priced.reduce((sum, p) => sum + (p.cost ?? 0), 0);

  const rows: CostRow[] = priced
    .map(({ usage, cost }) => ({
      ...usage,
      cost,
      share: cost !== null && total > 0 ? cost / total : 0,
    }))
    .sort((a, b) => (b.cost ?? -1) - (a.cost ?? -1));

  return { rows, total, hasUnpriced: priced.some((p) => p.cost === null) };
}
