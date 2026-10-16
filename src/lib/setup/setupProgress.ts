import type { ProviderId } from '../settings';

export interface SetupProgress {
  step: number;
  provider?: ProviderId;
}

export function parseSetupProgress(raw: unknown, providers: readonly string[]): SetupProgress | null {
  if (!raw || typeof raw !== 'object') return null;
  const { step, provider } = raw as { step?: unknown; provider?: unknown };
  if (typeof step !== 'number' || !Number.isInteger(step) || step < 0) return null;
  const known = typeof provider === 'string' && providers.includes(provider);
  return known ? { step, provider: provider as ProviderId } : { step };
}

export function resumeStep(
  saved: number,
  { apiKeyStep, doneStep, keyReady }: { apiKeyStep: number; doneStep: number; keyReady: boolean },
): number {
  const bounded = Math.min(Math.max(saved, 0), doneStep);
  return !keyReady && bounded > apiKeyStep ? apiKeyStep : bounded;
}
