import { describe, expect, it, vi } from 'vitest';
import { buildPresets, type Hardware } from '../components/settings/modelPresets';

vi.mock('../tauri', () => ({ invoke: vi.fn() }));

const noKeys = {
  groq: false,
  openai: false,
  google: false,
  assemblyai: false,
  openrouter: false,
  xai: false,
  local: false,
};
const hardware: Hardware = { totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: false };
const onboardingOptions = { includeTranscriptionOnly: true, localOnly: true };

describe('local onboarding presets', () => {
  it.each([false, true])('offers an explicit speech-only choice when hardware is unknown=%s', unknown => {
    const presets = buildPresets(noKeys, { ...hardware, unknown }, true, onboardingOptions);
    const speechOnly = presets.find(preset => preset.id === 'local-transcription-only');
    expect(speechOnly?.target).toMatchObject({
      cleanupEnabled: false,
      cleanupDefaultModel: null,
      cleanupFallbacks: [],
      dualTranscription: false,
    });
    expect(speechOnly?.target?.requiredLocalModels.map(model => [model.task, model.id])).toEqual([
      ['transcription', expect.any(String)],
    ]);
    expect(presets.some(preset => preset.target?.cleanupEnabled)).toBe(true);
  });

  it('keeps the local onboarding choice local when cloud keys already exist', () => {
    const presets = buildPresets({ ...noKeys, groq: true }, hardware, true, onboardingOptions);
    expect(presets.every(preset => preset.offline)).toBe(true);
    expect(presets[0].id).toBe('local-transcription-only');
  });

  it('does not duplicate the speech-only floor on small hardware', () => {
    const presets = buildPresets(noKeys, { ...hardware, totalRamMb: 2048 }, true, onboardingOptions);
    expect(presets.filter(preset => preset.id === 'local-transcription-only')).toHaveLength(1);
  });

  it('does not offer local inference when the platform does not support it', () => {
    expect(buildPresets(noKeys, hardware, false, onboardingOptions).map(preset => preset.kind)).toEqual(['add-key']);
  });

  it('does not add the speech-only option to the normal Settings picker', () => {
    const presets = buildPresets(noKeys, hardware, true);
    expect(presets.some(preset => preset.id === 'local-transcription-only')).toBe(false);
  });
});
