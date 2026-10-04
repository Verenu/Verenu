import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '../../tauri';
import { buildPresets, getHardware, type Hardware } from './modelPresets';

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
