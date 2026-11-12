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

  it('falls back to the older model and provider fields the way the backend does', () => {
    expect(usageLabel(providerUse('groq', {}))).toBe('Runs transcription and clean-up');
    const legacy = { transcription_model: 'openai/whisper-1', cleanup_model: 'gpt-5-mini', cleanup_provider: 'openai' };
    // An unset older model is a Groq model in the backend, whatever provider is saved beside it.
    expect(providerUse('openai', { transcription_provider: 'openai' }).transcription).toBeNull();
    expect(providerUse('openai', { transcription_default_model: ' OpenAI/whisper-1' }).transcription).toBe('main');
    expect(providerUse('openai', legacy)).toEqual({ transcription: 'main', cleanup: 'main' });
    expect(usageLabel(providerUse('groq', legacy))).toBe('Not in use');
  });

  it('counts the first fallback as running transcription when two models are compared', () => {
    const dual = { ...settings, transcription_fallback_models: ['openai/gpt-4o-mini-transcribe', 'xai/grok-stt'], dual_transcription_enabled: true };
    expect(providerUse('openai', dual).transcription).toBe('main');
    expect(providerUse('xai', dual).transcription).toBe('backup');
    expect(providerUse('openai', { ...dual, dual_transcription_enabled: false }).transcription).toBe('backup');
  });

  it('treats every cloud provider as running when an automatic mode orders models by speed', () => {
    const auto = { ...settings, transcription_fallback_models: ['openai/gpt-4o-mini-transcribe', 'local/parakeet-v3', 'xai/grok-stt'], model_selection_mode: 'balanced' };
    expect(providerUse('openai', auto).transcription).toBe('main');
    expect(providerUse('xai', auto).transcription).toBe('main');
    expect(providerUse('local', auto).transcription).toBe('backup');
    // Quality keeps its first two speech models in order; the second runs only when two are compared.
    const quality = { ...auto, model_selection_mode: 'quality', dual_transcription_enabled: true };
    expect(providerUse('openai', quality).transcription).toBe('main');
    expect(providerUse('openai', { ...quality, dual_transcription_enabled: false }).transcription).toBe('backup');
    expect(providerUse('xai', quality).transcription).toBe('backup');
    expect(providerUse('custom:12345678-1234-1234-8234-1234567890ab', quality).cleanup).toBe('main');
  });
});
