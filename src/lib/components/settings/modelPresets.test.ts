import { describe, expect, it } from 'vitest';
import { buildPresets, type Hardware } from './modelPresets';

const noKeys = { groq: false, openai: false, google: false, assemblyai: false, openrouter: false, xai: false, local: false };
const phone: Hardware = { totalRamMb: 4096, freeRamMb: 2048, gpus: [], unknown: false, isAndroid: true };

describe('Android offline presets', () => {
  it('offers the small English speech and cleanup pair on a 4 GB phone', () => {
    const presets = buildPresets(noKeys, phone, true);
    expect(presets).toHaveLength(1);
    expect(presets[0].target?.transcriptionDefaultModel).toBe('local/moonshine-tiny');
    expect(presets[0].target?.cleanupDefaultModel).toBe('local/qwen2.5-0.5b-instruct');
    expect(presets[0].target?.transcriptionFallbacks).toEqual([]);
    expect(presets[0].target?.cleanupFallbacks).toEqual([]);
    expect(presets[0].offline).toBe(true);
  });
  it('keeps a transcription-only offline choice on a 2 GB phone with cloud keys', () => {
    const local = buildPresets({ ...noKeys, groq: true }, { ...phone, totalRamMb: 2048 }, true).find(p => p.offline);
    expect(local?.target?.transcriptionDefaultModel).toBe('local/moonshine-tiny');
    expect(local?.target?.cleanupEnabled).toBe(false);
  });
  it('limits unknown phone hardware to the small pair', () => {
    const presets = buildPresets(noKeys, { ...phone, totalRamMb: 0, unknown: true }, true);
    expect(presets).toHaveLength(1);
    expect(presets[0].target?.cleanupDefaultModel).toBe('local/qwen2.5-0.5b-instruct');
  });
  it('offers stronger cleanup without recommending multi-GB desktop speech models', () => {
    const presets = buildPresets(noKeys, { ...phone, totalRamMb: 12288 }, true);
    expect(presets.map(p => p.target?.cleanupDefaultModel)).toEqual(['local/qwen2.5-0.5b-instruct', 'local/qwen2.5-1.5b-instruct']);
    expect(presets.every(p => p.target?.transcriptionDefaultModel === 'local/moonshine-tiny')).toBe(true);
  });
  it('still hides offline choices when packaged runtimes are missing', () => {
    expect(buildPresets(noKeys, phone, false).map(p => p.kind)).toEqual(['add-key']);
  });
  it('preserves desktop tier selection', () => {
    const presets = buildPresets(noKeys, { ...phone, isAndroid: false, totalRamMb: 16384 }, true);
    expect(presets.map(p => p.target?.transcriptionDefaultModel)).toEqual(['local/parakeet-v3', 'local/parakeet-v3', 'local/cohere']);
  });
});
