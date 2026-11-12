import { describe, expect, it } from 'vitest';
import { providerUse, usageLabel, usageLevel } from './providerUsage';

const settings = {
  transcription_default_model: 'groq/whisper-large-v3-turbo',
  transcription_fallback_models: ['openai/gpt-4o-mini-transcribe'],
  cleanup_default_model: 'groq/qwen/qwen3.8-27b',
  cleanup_fallback_models: ['custom:12345678-1234-1234-8234-1234567890ab/llama3.2'],
  cleanup_enabled: true,
};

describe('providerUse', () => {
  it('reports the provider that runs both tasks', () => {
    const use = providerUse('groq', settings);
    expect(use).toEqual({ transcription: 'main', cleanup: 'main' });
    expect(usageLabel(use)).toBe('Runs transcription and clean-up');
    expect(usageLevel(use)).toBe('main');
  });

  it('reports a provider that only appears in a fallback chain', () => {
    const use = providerUse('openai', settings);
    expect(usageLabel(use)).toBe('Backup for transcription');
    expect(usageLevel(use)).toBe('backup');
    expect(usageLabel(providerUse('custom:12345678-1234-1234-8234-1234567890ab', settings))).toBe('Backup for clean-up');
  });

  it('reports a provider that runs one task and backs up the other', () => {
    const use = providerUse('groq', { ...settings, cleanup_default_model: 'openai/gpt-5-mini', cleanup_fallback_models: ['groq/qwen/qwen3.8-27b'] });
    expect(usageLabel(use)).toBe('Runs transcription, backup for clean-up');
  });

  it('ignores clean-up models while clean-up is turned off', () => {
    const use = providerUse('groq', { ...settings, transcription_default_model: 'openai/whisper-1', cleanup_enabled: false });
    expect(use).toEqual({ transcription: null, cleanup: null });
    expect(usageLabel(use)).toBe('Not in use');
    expect(usageLevel(use)).toBe('idle');
  });

  it('treats missing settings as not in use', () => {
    expect(usageLabel(providerUse('xai', {}))).toBe('Not in use');
  });
});
