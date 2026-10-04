import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '../../tauri';
import { buildPresets, getHardware, matchActivePreset, type Hardware, type ModelPerformance } from './modelPresets';
import type { ModelCatalogCache } from '../../modelCatalogStore.svelte';

vi.mock('../../platform', () => ({ isAndroid: true }));
vi.mock('../../tauri', () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

const noKeys = { groq: false, openai: false, google: false, assemblyai: false, openrouter: false, xai: false, local: false };
const phone: Hardware = { totalRamMb: 4096, freeRamMb: 2048, gpus: [], unknown: false, isAndroid: true };

beforeEach(() => invokeMock.mockReset());

describe('hardware capability fallback', () => {
  it('keeps phone-sized presets when the native hardware command fails', async () => {
    invokeMock.mockRejectedValueOnce(new Error('hardware unavailable'));

    const hardware = await getHardware();
    const presets = buildPresets(noKeys, hardware, true);

    expect(hardware).toMatchObject({ isAndroid: true, unknown: true });
    expect(presets).toHaveLength(1);
    expect(presets[0].target?.transcriptionDefaultModel).toBe('local/moonshine-tiny');
    expect(presets[0].target?.cleanupDefaultModel).toBe('local/qwen2.5-0.5b-instruct');
  });

  it('keeps the backend platform result when the native hardware command succeeds', async () => {
    invokeMock.mockResolvedValueOnce({
      is_android: false,
      total_ram_mb: 16384,
      free_ram_mb: 12288,
      gpus: [],
    });

    await expect(getHardware()).resolves.toMatchObject({ isAndroid: false, unknown: false });
  });
});

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
  it('defaults every phone to the 0.5B cleanup model, however much RAM it has', () => {
    for (const totalRamMb of [4096, 8192, 12288, 16384]) {
      const presets = buildPresets(noKeys, { ...phone, totalRamMb }, true);
      expect(presets).toHaveLength(1);
      expect(presets[0].target?.cleanupDefaultModel).toBe('local/qwen2.5-0.5b-instruct');
      expect(presets[0].target?.transcriptionDefaultModel).toBe('local/moonshine-tiny');
    }
  });
  it('still hides offline choices when packaged runtimes are missing', () => {
    expect(buildPresets(noKeys, phone, false).map(p => p.kind)).toEqual(['add-key']);
  });
  it('preserves desktop tier selection', () => {
    const presets = buildPresets(noKeys, { ...phone, isAndroid: false, totalRamMb: 16384 }, true);
    expect(presets.map(p => p.target?.transcriptionDefaultModel)).toEqual(['local/parakeet-v3', 'local/parakeet-v3', 'local/cohere']);
  });
});

const desktop: Hardware = { ...phone, isAndroid: false, totalRamMb: 16384 };
const installedLocal = { transcription: ['parakeet-v3', 'moonshine-tiny'], cleanup: ['qwen2.5-1.5b-instruct'] };

describe('automatic selection and prepared recovery', () => {
  it('offers separate local priorities even with cloud credentials', () => {
    const presets = buildPresets({ ...noKeys, groq: true }, desktop, true);
    expect(presets.filter(p => !p.offline).map(p => p.name)).toEqual(['Fastest', 'Balanced', 'Quality']);
    expect(presets.filter(p => p.offline).map(p => p.name)).toEqual(['Fastest', 'Balanced', 'Quality']);
  });
  it('keeps all cloud alternatives before installed local models for both tasks', () => {
    const presets = buildPresets({ ...noKeys, groq: true, openai: true }, desktop, true, { installedLocal });
    const target = presets.find(p => p.id === 'cloud-balanced')!.target!;
    expect(target.transcriptionFallbacks).toEqual(['openai/gpt-4o-transcribe', 'local/parakeet-v3', 'local/moonshine-tiny']);
    expect(target.cleanupFallbacks).toEqual(['openai/gpt-4o-mini', 'local/qwen2.5-1.5b-instruct']);
    expect(target.requiredLocalModels).toEqual([]);
  });
  it('retains dual comparison and does not duplicate its primary', () => {
    const quality = buildPresets({ ...noKeys, groq: true }, desktop, true, { installedLocal }).find(p => p.id === 'cloud-accurate')!.target!;
    expect(quality.dualTranscription).toBe(true);
    expect(quality.transcriptionFallbacks[0]).toBe('groq/whisper-large-v3-turbo');
    expect(quality.transcriptionFallbacks).not.toContain(quality.transcriptionDefaultModel);
    const localQuality = buildPresets(noKeys, desktop, true).find(p => p.name === 'Quality')!.target!;
    expect(localQuality.dualTranscription).toBe(true);
    expect(localQuality.transcriptionFallbacks).toEqual(['local/parakeet-v3']);
  });
  it('keeps local-only speech, cleanup, and comparison entirely on-device', () => {
    for (const preset of buildPresets({ ...noKeys, groq: true }, desktop, true).filter(p => p.offline)) {
      const target = preset.target!;
      expect([target.transcriptionDefaultModel, target.cleanupDefaultModel!, ...target.transcriptionFallbacks, ...target.cleanupFallbacks].every(id => id.startsWith('local/'))).toBe(true);
    }
  });
  it('does not prepare unsupported speech or cleanup without its runtime', () => {
    const presets = buildPresets({ ...noKeys, assemblyai: true }, phone, true, { language: 'ja', installedLocal, localCleanupReady: false });
    const target = presets.find(p => p.id === 'cloud-balanced')!.target!;
    expect(target.transcriptionDefaultModel).toBe('assemblyai/universal-2');
    expect(target.transcriptionFallbacks).toEqual([]);
    expect(target.cleanupEnabled).toBe(false);
    expect(presets.some(p => p.offline)).toBe(false);
  });
  it('filters confirmed retirement but retains recommendations during an outage', () => {
    const cache: ModelCatalogCache = { groq: { ids: ['whisper-large-v3-turbo', 'unreviewed-new-model'], everSeen: ['whisper-large-v3'], lastSuccessAt: 1, lastAttemptAt: 1, lastError: null, missing: { 'groq/whisper-large-v3': { count: 2, lastCountedAt: 1 } } } };
    const selected = () => buildPresets({ ...noKeys, groq: true }, desktop, false, { cache }).find(p => p.id === 'cloud-balanced')!.target!.transcriptionDefaultModel;
    expect(selected()).toBe('groq/whisper-large-v3-turbo');
    cache.groq!.lastError = 'Temporary outage';
    expect(selected()).toBe('groq/whisper-large-v3');
  });
  it('ranks adequately sampled compatible alternatives without changing the current config', () => {
    const sample = (id: string, latency_ms: number): ModelPerformance => ({ id, task: 'transcription', latency_ms, samples: 3, failures: 0, updated_at_ms: Date.now() });
    const presets = buildPresets({ ...noKeys, groq: true, openai: true }, desktop, false, { performance: [sample('groq/whisper-large-v3', 300), sample('openai/gpt-4o-transcribe', 100)] });
    expect(presets.find(p => p.id === 'cloud-balanced')!.target!.transcriptionDefaultModel).toBe('openai/gpt-4o-transcribe');
  });
  it('recognizes cloud priorities independently of the chosen offline fallback', () => {
    const presets = buildPresets({ ...noKeys, groq: true }, desktop, true);
    const target = presets.find(p => p.id === 'cloud-balanced')!.target!;
    expect(matchActivePreset(presets, { ...target, cleanupDefaultModel: target.cleanupDefaultModel!, transcriptionFallbacks: ['local/moonshine-tiny'] })).toBe('cloud-balanced');
  });
});
