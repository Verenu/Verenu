import { describe, expect, it } from 'vitest';
import { dictationReadiness, hasCloudSpeechCandidate, hasReadyLocalSpeech, readinessModel, type ReadinessInput } from './dictationReadiness';

const cloud: ReadinessInput = {
  transcriptionModel: 'groq/whisper-large-v3-turbo',
  cleanupModel: 'openai/gpt-4o-mini',
  cleanupEnabled: true,
  keys: {},
  speechModels: [],
  cleanupModels: [],
  cleanupEngineInstalled: false,
};

const downloadedSpeech = [{ id: 'parakeet-v3', is_downloaded: true }];
const downloadedCleanup = [{ id: 'qwen2.5-3b-instruct', is_downloaded: true }];

describe('dictation configuration readiness', () => {
  it('resolves qualified, legacy, and current cloud provider defaults', () => {
    expect(readinessModel('transcription', null, null, 'local')).toBe('local/parakeet-v3');
    expect(readinessModel('cleanup', null, 'qwen/qwen3.8-27b', 'groq')).toBe('groq/qwen/qwen3.8-27b');
    expect(readinessModel('transcription', 'openai/gpt-4o-transcribe', null, 'groq')).toBe('openai/gpt-4o-transcribe');
    expect(readinessModel('transcription', null, 'local/parakeet-v3', 'groq')).toBe('local/parakeet-v3');
    expect(readinessModel('transcription', null, null, 'openrouter')).toBe('openrouter/openai/whisper-large-v3');
    expect(readinessModel('cleanup', null, null, 'xai')).toBe('xai/grok-4-fast-non-reasoning');
  });

  it('warns when selected cloud speech and cleanup providers have no saved keys', () => {
    expect(dictationReadiness(cloud).map(issue => [issue.task, issue.section])).toEqual([
      ['transcription', 'keys'],
      ['cleanup', 'keys'],
    ]);
  });

  it('checks the selected providers and recognizes OpenRouter and xAI keys', () => {
    expect(dictationReadiness({ ...cloud, keys: { google: true } })).toHaveLength(2);
    expect(dictationReadiness({ ...cloud, keys: { groq: true, openai: true } })).toEqual([]);
    expect(dictationReadiness({
      ...cloud,
      transcriptionModel: 'openrouter/openai/whisper-large-v3',
      cleanupModel: 'xai/grok-4-fast-non-reasoning',
      keys: { openrouter: true, xai: true },
    })).toEqual([]);
  });

  it('accepts a custom endpoint with no key when it supports the configured tasks', () => {
    const custom = {
      id: 'custom:11111111-1111-4111-8111-111111111111',
      name: 'Local endpoint',
      requires_key: false,
      supports_transcription: true,
      supports_cleanup: true,
    };
    expect(dictationReadiness({
      ...cloud,
      transcriptionModel: custom.id + '/speech-model',
      cleanupModel: custom.id + '/cleanup-model',
      customProviders: [custom],
    })).toEqual([]);
  });

  it('does not classify a ready custom endpoint as cloud speech for offline warnings', () => {
    const custom = {
      id: 'custom:33333333-3333-4333-8333-333333333333',
      name: 'Local endpoint',
      requires_key: false,
      supports_transcription: true,
      supports_cleanup: false,
    };
    const input = {
      ...cloud,
      transcriptionModel: custom.id + '/speech-model',
      cleanupEnabled: false,
      customProviders: [custom],
    };
    expect(dictationReadiness(input)).toEqual([]);
    expect(hasCloudSpeechCandidate(input)).toBe(false);
  });

  it('requires a custom provider key only when configured and reports unsupported tasks', () => {
    const custom = {
      id: 'custom:22222222-2222-4222-8222-222222222222',
      name: 'Custom speech',
      requires_key: true,
      supports_transcription: true,
      supports_cleanup: false,
    };
    const input = {
      ...cloud,
      transcriptionModel: custom.id + '/speech-model',
      cleanupEnabled: false,
      customProviders: [custom],
    };
    expect(dictationReadiness(input)).toMatchObject([{ task: 'transcription', section: 'keys' }]);
    expect(dictationReadiness({ ...input, keys: { [custom.id]: true } })).toEqual([]);
    expect(dictationReadiness({
      ...input,
      transcriptionModel: cloud.transcriptionModel,
      cleanupEnabled: true,
      cleanupModel: custom.id + '/cleanup-model',
      keys: { groq: true },
    })).toMatchObject([{ task: 'cleanup', section: 'models' }]);
  });

  it('does not require cleanup keys or engines when cleanup is off', () => {
    expect(dictationReadiness({ ...cloud, keys: { groq: true }, cleanupEnabled: false })).toEqual([]);
  });

  it('offers model setup for missing local speech without requiring a key', () => {
    const issues = dictationReadiness({ ...cloud, transcriptionModel: 'local/parakeet-v3', cleanupEnabled: false });
    expect(issues).toMatchObject([{ task: 'transcription', section: 'models' }]);
    expect(dictationReadiness({
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      cleanupEnabled: false,
      speechModels: downloadedSpeech,
    })).toEqual([]);
  });

  it('distinguishes missing local cleanup weights from a missing engine', () => {
    const input = { ...cloud, keys: { groq: true }, cleanupModel: 'local/qwen2.5-3b-instruct' };
    expect(dictationReadiness(input)[0].message).toContain('not installed');
    const installed = { ...input, cleanupModels: downloadedCleanup };
    expect(dictationReadiness(installed)[0].message).toContain('local engine');
    expect(dictationReadiness({ ...installed, cleanupEngineInstalled: true })).toEqual([]);
  });

  it('accepts an installed speech fallback when the cloud primary key is missing', () => {
    const input = {
      ...cloud,
      transcriptionFallbacks: ['local/parakeet-v3'],
      speechModels: downloadedSpeech,
      cleanupEnabled: false,
    };
    expect(dictationReadiness(input)).toEqual([]);
    expect(hasReadyLocalSpeech(input)).toBe(true);
    expect(hasCloudSpeechCandidate(input)).toBe(true);
  });

  it('accepts an installed cleanup fallback when the cloud cleanup key is missing', () => {
    const input = {
      ...cloud,
      keys: { groq: true },
      cleanupFallbacks: ['local/qwen2.5-3b-instruct'],
      cleanupModels: downloadedCleanup,
      cleanupEngineInstalled: true,
    };
    expect(dictationReadiness(input)).toEqual([]);
  });

  it('does not count a local fallback until its model and cleanup engine are installed', () => {
    const input = { ...cloud, transcriptionFallbacks: ['local/parakeet-v3'] };
    expect(dictationReadiness(input).find(issue => issue.task === 'transcription')).toMatchObject({ section: 'keys' });
    expect(hasReadyLocalSpeech(input)).toBe(false);
  });

  it('routes malformed or unsupported selections to model settings', () => {
    for (const transcriptionModel of ['', 'local/', 'removed/model']) {
      expect(dictationReadiness({ ...cloud, transcriptionModel, cleanupEnabled: false })[0].section).toBe('models');
    }
  });
});
