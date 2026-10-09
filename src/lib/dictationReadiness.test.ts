import { describe, expect, it } from 'vitest';
import { cleanupMayBeUsed, dictationReadiness, hasCleanupIntensityOverride, hasCloudSpeechCandidate, hasReachableOffCleanupOverride, hasReadyOfflineSpeech, readinessModel, type ReadinessInput } from './dictationReadiness';

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
  it('Basic needs no cleanup provider or dual fusion while AI context overrides still do', () => {
    const input = { ...cloud, keys: { groq: true }, cleanupIntensity: 'rules', dualTranscriptionEnabled: true,
      transcriptionFallbacks: ['local/parakeet-v3'], speechModels: downloadedSpeech };
    expect(cleanupMayBeUsed(input)).toBe(false);
    expect(dictationReadiness(input)).toEqual([]);
    expect(hasCleanupIntensityOverride([{ id: 1, is_everywhere: true, cleanup_intensity: 'rules' }])).toBe(false);
    expect(cleanupMayBeUsed({ ...input, cleanupIntensityOverrideMayBeUsed: true })).toBe(true);
  });
  it('excludes built-in cloud speech from offline fusion but restores it online', () => {
    const input: ReadinessInput = {
      ...cloud, transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['groq/whisper-large-v3-turbo'],
      speechModels: downloadedSpeech, keys: { groq: true },
      dualTranscriptionEnabled: true, cleanupIntensity: 'none', isOnline: false,
    };
    expect(cleanupMayBeUsed(input)).toBe(false);
    expect(dictationReadiness(input)).toEqual([]);
    expect(cleanupMayBeUsed({ ...input, isOnline: true })).toBe(true);
    expect(dictationReadiness({ ...input, isOnline: true }).some(issue => issue.task === 'cleanup')).toBe(true);
    expect(cleanupMayBeUsed({ ...input, cleanupIntensityOverrideMayBeUsed: true })).toBe(true);
  });

  it('preserves custom speech endpoints in offline fusion', () => {
    const custom = { id: 'custom:11111111-1111-4111-8111-111111111111', name: 'LAN endpoint', requires_key: false, supports_transcription: true, supports_cleanup: false };
    expect(cleanupMayBeUsed({
      ...cloud, transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: [`${custom.id}/speech`], speechModels: downloadedSpeech,
      customProviders: [custom], dualTranscriptionEnabled: true, cleanupIntensity: 'none', isOnline: false,
    })).toBe(true);
  });

  it('resolves backend provider defaults when both model settings are invalid', () => {
    const backendDefaults = [
      ['transcription', 'groq', 'groq/whisper-large-v3-turbo'],
      ['transcription', 'openai', 'openai/gpt-4o-transcribe'],
      ['transcription', 'google', 'google/gemini-3.5-transcribe'],
      ['transcription', 'assemblyai', 'assemblyai/universal-3-5-pro'],
      ['transcription', 'openrouter', 'openrouter/openai/whisper-large-v3'],
      ['transcription', 'xai', 'xai/grok-voice-transcribe-2.0'],
      ['transcription', 'local', 'local/parakeet-v3'],
      ['cleanup', 'groq', 'groq/qwen/qwen3.8-27b'],
      ['cleanup', 'openai', 'openai/gpt-4o-mini'],
      ['cleanup', 'google', 'google/gemini-3.5-flash-lite'],
      ['cleanup', 'assemblyai', 'assemblyai/qwen/qwen3.8-27b'],
      ['cleanup', 'openrouter', 'openrouter/openai/gpt-4o-mini'],
      ['cleanup', 'xai', 'xai/grok-4-fast-non-reasoning'],
      ['cleanup', 'local', 'local/gemma-4-e2b'],
    ] as const;
    for (const [task, provider, expected] of backendDefaults) {
      expect(readinessModel(task, null, 'invalid/model', provider)).toBe(expected);
    }
  });

  it('uses a valid new model first, then a valid legacy model without prefixing malformed IDs', () => {
    expect(readinessModel('transcription', ' OpenAI/gpt-4o-transcribe ', 'groq/whisper-large-v3-turbo', 'groq'))
      .toBe('openai/gpt-4o-transcribe');
    expect(readinessModel('transcription', 'not-a-model-id', 'Groq/whisper-large-v3-turbo', 'openai'))
      .toBe('groq/whisper-large-v3-turbo');
    expect(readinessModel('transcription', 'not-a-model-id', 'whisper-large-v3-turbo', 'openai'))
      .toBe('openai/gpt-4o-transcribe');
    expect(readinessModel('transcription', 'not-a-model-id', '  ', 'openai'))
      .toBe('openai/gpt-4o-transcribe');
    expect(readinessModel('cleanup', 'not-a-model-id', null, 'openai'))
      .toBe('groq/qwen/qwen3.8-27b');

    const invalidNewWithGroqLegacy = dictationReadiness({
      ...cloud,
      transcriptionModel: readinessModel('transcription', 'not-a-model-id', 'groq/whisper-large-v3-turbo', 'openai'),
      cleanupEnabled: false,
      keys: { openai: true },
    }).find(issue => issue.task === 'transcription');
    expect(invalidNewWithGroqLegacy).toMatchObject({ section: 'keys' });
    expect(invalidNewWithGroqLegacy?.message).toContain('groq');
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

  it('treats only ready offline-capable candidates as a way around a cloud speech warning', () => {
    const custom = {
      id: 'custom:33333333-3333-4333-8333-333333333333',
      name: 'Local endpoint',
      requires_key: false,
      supports_transcription: true,
      supports_cleanup: false,
    };
    const customPrimary = {
      ...cloud,
      transcriptionModel: custom.id + '/speech-model',
      transcriptionFallbacks: ['groq/whisper-large-v3-turbo'],
      cleanupEnabled: false,
      customProviders: [custom],
    };
    expect(dictationReadiness(customPrimary)).toEqual([]);
    expect(hasCloudSpeechCandidate(customPrimary)).toBe(true);
    expect(hasReadyOfflineSpeech(customPrimary)).toBe(true);

    const customFallback = {
      ...customPrimary,
      transcriptionModel: 'groq/whisper-large-v3-turbo',
      transcriptionFallbacks: [custom.id + '/speech-model'],
      keys: { groq: true },
    };
    expect(dictationReadiness(customFallback)).toEqual([]);
    expect(hasCloudSpeechCandidate(customFallback)).toBe(true);
    expect(hasReadyOfflineSpeech(customFallback)).toBe(true);

    expect(hasReadyOfflineSpeech({
      ...customPrimary,
      customProviders: [{ ...custom, requires_key: true }],
    })).toBe(false);
    expect(hasCloudSpeechCandidate({
      ...customPrimary,
      customProviders: [{ ...custom, requires_key: true }],
    })).toBe(true);
    expect(hasReadyOfflineSpeech({
      ...customPrimary,
      customProviders: [{ ...custom, supports_transcription: false }],
    })).toBe(false);
    expect(hasReadyOfflineSpeech({
      ...customPrimary,
      transcriptionModel: 'custom:44444444-4444-4444-8444-444444444444/speech-model',
    })).toBe(false);
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

  it('checks cleanup for potential transcript fusion only when dual transcription can use it', () => {
    const singleTranscription = {
      ...cloud,
      transcriptionFallbacks: [],
      keys: { groq: true, openai: true },
    };
    const distinctTranscriptionFallback = { transcriptionFallbacks: ['openai/gpt-4o-transcribe'] };
    expect(cleanupMayBeUsed({ ...singleTranscription, cleanupEnabled: false, cleanupIntensity: 'none', dualTranscriptionEnabled: true })).toBe(false);
    expect(cleanupMayBeUsed({ ...singleTranscription, cleanupEnabled: true, cleanupIntensity: 'none', dualTranscriptionEnabled: false })).toBe(false);
    expect(cleanupMayBeUsed({ ...singleTranscription, cleanupEnabled: true, cleanupIntensity: 'none', dualTranscriptionEnabled: true })).toBe(false);
    expect(cleanupMayBeUsed({ ...singleTranscription, transcriptionFallbacks: [' GROQ/ whisper-large-v3-turbo '], cleanupEnabled: true, cleanupIntensity: 'none', dualTranscriptionEnabled: true })).toBe(false);
    expect(cleanupMayBeUsed({ ...singleTranscription, transcriptionModel: 'groq/llama-3.1-8b-instant', transcriptionFallbacks: ['groq/qwen/qwen3.8-27b'], cleanupEnabled: true, cleanupIntensity: 'none', dualTranscriptionEnabled: true })).toBe(false);
    expect(cleanupMayBeUsed({ ...singleTranscription, ...distinctTranscriptionFallback, cleanupEnabled: true, cleanupIntensity: 'none', dualTranscriptionEnabled: true })).toBe(true);
    expect(cleanupMayBeUsed({ ...singleTranscription, cleanupEnabled: true, cleanupIntensity: null, dualTranscriptionEnabled: false })).toBe(true);

    const localSpeechWithUnkeyedCloudFallback = dictationReadiness({
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['openai/gpt-4o-transcribe'],
      speechModels: downloadedSpeech,
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
      keys: {},
    });
    expect(localSpeechWithUnkeyedCloudFallback).toEqual([]);

    const missingLocalAndUnkeyedCloud = dictationReadiness({
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['openai/gpt-4o-transcribe'],
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
      keys: {},
    });
    expect(missingLocalAndUnkeyedCloud.map(issue => issue.task)).toEqual(['transcription']);

    const unsupportedCustomSpeechFallback = dictationReadiness({
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['custom:33333333-3333-4333-8333-333333333333/speech-model'],
      speechModels: downloadedSpeech,
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
      keys: {},
      customProviders: [{
        id: 'custom:33333333-3333-4333-8333-333333333333',
        name: 'Cleanup endpoint',
        requires_key: false,
        supports_transcription: false,
        supports_cleanup: true,
      }],
    });
    expect(unsupportedCustomSpeechFallback).toEqual([]);

    const twoReadySpeechModels = dictationReadiness({
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['openai/gpt-4o-transcribe'],
      speechModels: downloadedSpeech,
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
      keys: { openai: true },
    });
    expect(twoReadySpeechModels).toMatchObject([{
      task: 'cleanup',
      message: expect.stringContaining('Transcript comparison may use cleanup'),
    }]);

    const migratedDuplicateCandidates = {
      ...singleTranscription,
      transcriptionModel: 'google/gemini-2.5-pro',
      transcriptionFallbacks: ['google/gemini-3.5-flash-lite'],
      keys: { google: true },
      cleanupEnabled: true,
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
    };
    expect(cleanupMayBeUsed(migratedDuplicateCandidates)).toBe(false);

    const offWithoutFusion = dictationReadiness({
      ...cloud,
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: false,
      cleanupEnabled: true,
      keys: { groq: true },
    });
    expect(offWithoutFusion.some(issue => issue.task === 'cleanup')).toBe(false);

    const fusionMissingKey = dictationReadiness({
      ...cloud,
      cleanupIntensity: 'none',
      transcriptionFallbacks: ['openai/gpt-4o-transcribe'],
      dualTranscriptionEnabled: true,
      cleanupEnabled: true,
      cleanupModel: 'xai/grok-4-fast-non-reasoning',
      keys: { groq: true, openai: true },
    }).find(issue => issue.task === 'cleanup');
    expect(fusionMissingKey).toMatchObject({ section: 'keys' });
    expect(fusionMissingKey?.message).toContain('Transcript comparison may use cleanup');

    const fusionMissingModelWithReadyFallback = dictationReadiness({
      ...cloud,
      cleanupIntensity: 'none',
      dualTranscriptionEnabled: true,
      cleanupEnabled: true,
      cleanupModel: 'local/qwen2.5-3b-instruct',
      cleanupFallbacks: ['groq/qwen/qwen3.8-27b'],
      keys: { groq: true },
    });
    expect(fusionMissingModelWithReadyFallback.some(issue => issue.task === 'cleanup')).toBe(false);
  });

  it('checks cleanup when a Context or legacy app mapping overrides the global Off intensity', () => {
    expect(hasCleanupIntensityOverride([{ id: 1, is_everywhere: true, cleanup_intensity: 'light' }])).toBe(true);
    expect(hasCleanupIntensityOverride([], [{ cleanup_intensity: 'high' }])).toBe(true);
    expect(hasCleanupIntensityOverride([
      { id: 1, is_everywhere: true, cleanup_intensity: 'none' },
      { id: 2, is_everywhere: false, cleanup_intensity: null },
      { id: 3, is_everywhere: false },
    ], [{ cleanup_intensity: '  NONE ' }])).toBe(false);

    const input: ReadinessInput = {
      ...cloud,
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'none',
      transcriptionFallbacks: ['openai/gpt-4o-transcribe'],
      dualTranscriptionEnabled: true,
      cleanupEnabled: true,
      keys: { groq: true },
    };
    expect(cleanupMayBeUsed(input)).toBe(false);
    expect(dictationReadiness(input).some(issue => issue.task === 'cleanup')).toBe(false);

    const overridden = { ...input, cleanupIntensityOverrideMayBeUsed: true };
    expect(cleanupMayBeUsed(overridden)).toBe(true);
    expect(dictationReadiness(overridden)).toMatchObject([
      { task: 'cleanup', section: 'models', action: 'Choose models' },
    ]);
    expect(dictationReadiness(overridden)[0]?.message).toContain('A Context or app mapping may use cleanup.');
  });

  it('ignores unreachable Context overrides until an app, website, or sub-app is assigned', () => {
    const contexts = [{ id: 2, is_everywhere: false, cleanup_intensity: 'light' }];
    expect(hasCleanupIntensityOverride(contexts)).toBe(false);
    expect(hasCleanupIntensityOverride(contexts, [], [{ context_id: null }, { context_id: 3 }])).toBe(false);
    for (const assignment of [
      { context_id: 2, executable: 'fixture-app' },
      { context_id: 2, domain: 'fixture.example' },
      { context_id: 2, title_pattern: 'Fixture' },
    ]) {
      expect(hasCleanupIntensityOverride(contexts, [], [assignment])).toBe(true);
    }
    expect(hasCleanupIntensityOverride(contexts, [], [])).toBe(false);
  });

  it('warns for dual speech under Basic when a reachable Off Context reconciles with cleanup', () => {
    const contexts = [{ id: 4, is_everywhere: false, cleanup_intensity: 'none' }];
    const assignments = [{ context_id: 4, executable: 'fixture-app' }];
    expect(hasReachableOffCleanupOverride(contexts)).toBe(false);
    expect(hasReachableOffCleanupOverride(contexts, [], [{ context_id: null }])).toBe(false);
    expect(hasReachableOffCleanupOverride(contexts, [], assignments)).toBe(true);
    expect(hasReachableOffCleanupOverride([{ id: 5, is_everywhere: true, cleanup_intensity: 'rules' }])).toBe(false);
    expect(hasReachableOffCleanupOverride([], [{ cleanup_intensity: '  NONE ' }])).toBe(true);
    expect(hasCleanupIntensityOverride(contexts, [], assignments)).toBe(false);

    const basicDualSpeech: ReadinessInput = {
      ...cloud,
      transcriptionModel: 'local/parakeet-v3',
      transcriptionFallbacks: ['groq/whisper-large-v3-turbo'],
      speechModels: downloadedSpeech,
      cleanupModel: 'local/missing-cleanup-model',
      cleanupIntensity: 'rules',
      dualTranscriptionEnabled: true,
      keys: { groq: true },
      cleanupOffContextMayBeUsed: hasReachableOffCleanupOverride(contexts, [], assignments),
    };
    expect(cleanupMayBeUsed(basicDualSpeech)).toBe(true);
    expect(dictationReadiness(basicDualSpeech)).toMatchObject([{
      task: 'cleanup',
      section: 'models',
      message: expect.stringContaining('A Context set to Off may use cleanup to compare both speech results.'),
    }]);

    // Negations: no reachable Off Context, unassigned Off Context, one or duplicate
    // eligible candidates, unavailable or offline fallback, cleanup off, ready cleanup.
    expect(cleanupMayBeUsed({ ...basicDualSpeech, cleanupOffContextMayBeUsed: false })).toBe(false);
    expect(dictationReadiness({ ...basicDualSpeech, cleanupOffContextMayBeUsed: false })).toEqual([]);
    expect(dictationReadiness({ ...basicDualSpeech, cleanupOffContextMayBeUsed: hasReachableOffCleanupOverride(contexts) })).toEqual([]);
    expect(dictationReadiness({ ...basicDualSpeech, transcriptionFallbacks: [], cleanupOffContextMayBeUsed: true })).toEqual([]);
    expect(dictationReadiness({
      ...basicDualSpeech,
      transcriptionModel: 'groq/whisper-large-v3-turbo',
      transcriptionFallbacks: [' GROQ/ whisper-large-v3-turbo '],
      speechModels: [],
      cleanupOffContextMayBeUsed: true,
    })).toEqual([]);
    expect(dictationReadiness({ ...basicDualSpeech, transcriptionFallbacks: ['openai/gpt-4o-transcribe'], cleanupOffContextMayBeUsed: true })).toEqual([]);
    expect(dictationReadiness({ ...basicDualSpeech, isOnline: false, cleanupOffContextMayBeUsed: true })).toEqual([]);
    expect(dictationReadiness({ ...basicDualSpeech, cleanupEnabled: false, cleanupOffContextMayBeUsed: true })).toEqual([]);
    expect(dictationReadiness({
      ...basicDualSpeech,
      cleanupModel: 'local/qwen2.5-3b-instruct',
      cleanupModels: downloadedCleanup,
      cleanupEngineInstalled: true,
      cleanupOffContextMayBeUsed: true,
    })).toEqual([]);
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
    expect(hasReadyOfflineSpeech(input)).toBe(true);
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
    expect(hasReadyOfflineSpeech(input)).toBe(false);
  });

  it('rejects built-in cleanup models the backend skips and accepts a ready fallback', () => {
    for (const [provider, model] of [
      ['openai', 'o3'],
      ['groq', 'qwen/qwen3-32b'],
      ['google', 'gemini-3.7-pro'],
      ['openrouter', 'openai/o3-mini'],
      ['xai', 'grok-4-fast-reasoning'],
      ['assemblyai', 'universal-2'],
    ]) {
      const input = {
        ...cloud,
        cleanupModel: `${provider}/${model}`,
        keys: { groq: true, [provider]: true },
      };
      expect(dictationReadiness(input).find(issue => issue.task === 'cleanup')).toMatchObject({ section: 'models' });
    }

    for (const [provider, model] of [
      ['openai', 'gpt-4o-mini'],
      ['openai', 'gpt-5.1'],
      ['groq', 'qwen/qwen3.8-27b'],
      ['google', 'gemini-3.5-flash'],
      ['openrouter', 'meta-llama/llama-3.3-70b-instruct'],
      ['xai', 'grok-4-fast-non-reasoning'],
    ]) {
      expect(dictationReadiness({
        ...cloud,
        cleanupModel: `${provider}/${model}`,
        keys: { groq: true, [provider]: true },
      }).some(issue => issue.task === 'cleanup')).toBe(false);
    }

    const fallback = dictationReadiness({
      ...cloud,
      cleanupModel: 'openai/o3',
      cleanupFallbacks: ['openai/gpt-4o-mini'],
      keys: { groq: true, openai: true },
    });
    expect(fallback.some(issue => issue.task === 'cleanup')).toBe(false);
  });

  it('applies the backend model migrations to qualified primary and fallback candidates', () => {
    const migratedPrimary = dictationReadiness({
      ...cloud,
      cleanupModel: 'google/gemini-2.5-pro',
      cleanupFallbacks: ['google/gemini-3.7-flash'],
      keys: { groq: true, google: true },
    });
    expect(migratedPrimary).toEqual([]);

    const migratedFallback = dictationReadiness({
      ...cloud,
      cleanupModel: 'openai/o3',
      cleanupFallbacks: ['groq/openai/gpt-oss-120b'],
      keys: { groq: true, openai: true },
    });
    expect(migratedFallback).toEqual([]);
  });

  it('uses a valid qualified legacy model after an unqualified new model', () => {
    const legacyGoogleModel = readinessModel('cleanup', 'gemini-2.5-pro', 'google/gemini-2.5-pro', 'google');
    expect(legacyGoogleModel).toBe('google/gemini-2.5-pro');
    expect(dictationReadiness({
      ...cloud,
      cleanupModel: legacyGoogleModel,
      keys: { groq: true, google: true },
    })).toEqual([]);

    const invalid = dictationReadiness({
      ...cloud,
      cleanupModel: 'openai/o3',
      cleanupFallbacks: ['google/gemini-4-unknown'],
      keys: { groq: true, openai: true, google: true },
    }).find(issue => issue.task === 'cleanup');
    expect(invalid?.section).toBe('models');
    expect(invalid?.message).toContain('gemini-4-unknown');
  });

  it('does not check an unsupported cleanup selection when optional cleanup is off', () => {
    const issues = dictationReadiness({
      ...cloud,
      cleanupModel: 'assemblyai/universal-2',
      cleanupEnabled: false,
      keys: { groq: true },
    });
    expect(issues.some(issue => issue.task === 'cleanup')).toBe(false);
  });

  it('routes malformed or unsupported selections to model settings', () => {
    for (const transcriptionModel of ['', 'local/', 'removed/model']) {
      expect(dictationReadiness({ ...cloud, transcriptionModel, cleanupEnabled: false })[0].section).toBe('models');
    }
  });
});
