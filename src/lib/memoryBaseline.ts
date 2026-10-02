export const MEMORY_SAMPLE_INTERVAL_MS = 5000;
export const MEMORY_BASELINE_SAMPLES = 720;
export const MEMORY_BASELINE_KEY = 'verenu.memory-baseline.v1';

export interface MemoryBaseline {
  averageMb: number;
  samples: number;
}

export function parseMemoryBaseline(raw: string | null): MemoryBaseline {
  try {
    const value = JSON.parse(raw ?? 'null');
    if (value && Number.isFinite(value.averageMb) && value.averageMb > 0 &&
      Number.isInteger(value.samples) && value.samples > 0 &&
      value.samples <= MEMORY_BASELINE_SAMPLES) return value;
  } catch { /* Start calibration again if local storage is malformed. */ }
  return { averageMb: 0, samples: 0 };
}

export function sampleMemoryBaseline(state: MemoryBaseline, mb: number): MemoryBaseline {
  if (!Number.isFinite(mb) || mb <= 0 || state.samples >= MEMORY_BASELINE_SAMPLES) return state;
  const samples = state.samples + 1;
  return { averageMb: state.averageMb + (mb - state.averageMb) / samples, samples };
}

export function memoryMeterPercent(mb: number, baselineMb: number): number {
  if (!Number.isFinite(mb) || mb <= 0 || !Number.isFinite(baselineMb) || baselineMb <= 0) return 0;
  return Math.min(100, mb / baselineMb * 50);
}
