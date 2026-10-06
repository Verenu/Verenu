import type { ProviderId } from '../settings';

/** An unfinished wizard's place, saved so a restart resumes it. Mirrors `setup_progress` in Rust. */
export interface SetupProgress {
  step: number;
  provider?: ProviderId;
}

/** Read a saved value defensively: anything malformed means "start from the top". */
export function parseSetupProgress(raw: unknown, providers: readonly string[]): SetupProgress | null {
  if (!raw || typeof raw !== 'object') return null;
  const { step, provider } = raw as { step?: unknown; provider?: unknown };
  if (typeof step !== 'number' || !Number.isInteger(step) || step < 0) return null;
  const known = typeof provider === 'string' && providers.includes(provider);
  return known ? { step, provider: provider as ProviderId } : { step };
}

/**
 * The step to reopen on. Never past the API key step without a key to show for
 * it (the later steps need one), and never past the Done screen.
 */
export function resumeStep(
  saved: number,
  { apiKeyStep, doneStep, keyReady }: { apiKeyStep: number; doneStep: number; keyReady: boolean },
): number {
  const bounded = Math.min(Math.max(saved, 0), doneStep);
  return !keyReady && bounded > apiKeyStep ? apiKeyStep : bounded;
}
