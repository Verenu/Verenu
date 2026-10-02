import { describe, expect, it } from 'vitest';
import { MEMORY_BASELINE_SAMPLES, memoryMeterPercent, parseMemoryBaseline, sampleMemoryBaseline } from './memoryBaseline';

describe('memory baseline', () => {
  it('resumes calibration across restarts, including unchanged readings, and freezes after an hour', () => {
    let state = sampleMemoryBaseline(parseMemoryBaseline(null), 500);
    state = sampleMemoryBaseline(parseMemoryBaseline(JSON.stringify(state)), 600);
    expect(state.averageMb).toBe(550);
    for (let i = state.samples; i < MEMORY_BASELINE_SAMPLES; i++) state = sampleMemoryBaseline(state, 550);
    expect(state).toEqual({ averageMb: 550, samples: MEMORY_BASELINE_SAMPLES });
    expect(sampleMemoryBaseline(state, 2000)).toBe(state);
  });

  it('puts the average at half, lower usage below it, and twice the average at full', () => {
    expect(memoryMeterPercent(548, 548)).toBe(50);
    expect(memoryMeterPercent(274, 548)).toBe(25);
    expect(memoryMeterPercent(1096, 548)).toBe(100);
    expect(memoryMeterPercent(2000, 548)).toBe(100);
    expect(memoryMeterPercent(0, 0)).toBe(0);
  });

  it('rejects corrupt storage and ignores failed measurements', () => {
    for (const raw of ['broken', 'null', '{"averageMb":500,"samples":-1}', '{"averageMb":0,"samples":720}']) {
      expect(parseMemoryBaseline(raw)).toEqual({ averageMb: 0, samples: 0 });
    }
    const state = sampleMemoryBaseline(parseMemoryBaseline(null), 548);
    for (const mb of [0, -1, NaN, Infinity]) expect(sampleMemoryBaseline(state, mb)).toBe(state);
  });
});
