import { describe, expect, it } from 'vitest';
import type { PresetTarget } from '../components/settings/modelPresets';
import { setupCleanupEnabled, setupDefaultModels, setupModelReadiness, type SetupModelInventory } from './setupModelReadiness';

const inventory: SetupModelInventory = {
  speechModels: [],
  cleanupModels: [],
  transcriptionState: { is_downloading: false, downloading_model_id: null },
  cleanupState: { is_downloading: false, downloading_model_id: null },
  cleanupRuntime: { installed: false, is_downloading: false },
};

const transcriptionOnly: PresetTarget = {
  transcriptionDefaultModel: 'local/parakeet-v3',
  cleanupEnabled: false,
  cleanupDefaultModel: null,
  dualTranscription: false,
  transcriptionFallbacks: [],
  cleanupFallbacks: [],
  requiredLocalModels: [{ task: 'transcription', id: 'parakeet-v3', sizeMb: 1 }],
};

const localWithCleanup: PresetTarget = {
  ...transcriptionOnly,
  cleanupEnabled: true,
  cleanupDefaultModel: 'local/qwen2.5-3b-instruct',
  requiredLocalModels: [
    ...transcriptionOnly.requiredLocalModels,
    { task: 'cleanup', id: 'qwen2.5-3b-instruct', sizeMb: 1 },
  ],
};

describe('setup model readiness', () => {
  it('validates the defaults saved by Finish when Android skips the preset step', () => {
    const defaults = setupDefaultModels('local');
    expect(setupModelReadiness(null, inventory, true, defaults)).toMatchObject({
      ready: false,
      message: expect.stringContaining('speech model, cleanup model, and cleanup engine'),
    });
    const speechInstalled = {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: true, is_downloading: false }],
    };
    expect(setupModelReadiness(null, speechInstalled, false, defaults).ready).toBe(true);
    expect(setupModelReadiness(null, speechInstalled, true, defaults)).toMatchObject({
      ready: false, message: expect.stringContaining('cleanup model and cleanup engine'),
    });
    expect(setupModelReadiness(null, {
      ...speechInstalled,
      cleanupModels: [{ id: 'qwen2.5-3b-instruct', is_downloaded: true, is_downloading: false }],
      cleanupRuntime: { installed: true, is_downloading: false },
    }, true, defaults).ready).toBe(true);
    expect(setupModelReadiness(null, {
      ...inventory,
      transcriptionState: { is_downloading: true, downloading_model_id: 'parakeet-v3' },
    }, false, defaults)).toMatchObject({ ready: false, pending: true });
    for (const provider of ['groq', 'openai', 'google']) {
      expect(setupModelReadiness(null, inventory, true, setupDefaultModels(provider)).ready).toBe(true);
    }
  });

  it('requires speech for a local transcription-only preset without requiring optional cleanup', () => {
    expect(setupModelReadiness(transcriptionOnly, inventory)).toMatchObject({
      ready: false,
      pending: false,
      message: expect.stringContaining('speech model'),
    });
    expect(setupModelReadiness({
      ...transcriptionOnly,
      requiredLocalModels: [
        ...transcriptionOnly.requiredLocalModels,
        { task: 'cleanup', id: 'qwen2.5-3b-instruct', sizeMb: 1 },
      ],
    }, {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: true, is_downloading: false }],
    })).toMatchObject({ ready: true, pending: false, message: '' });
  });

  it('reports a pending speech download separately from API-key status', () => {
    const pending = setupModelReadiness(transcriptionOnly, {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: false, is_downloading: true }],
      transcriptionState: { is_downloading: true, downloading_model_id: 'parakeet-v3' },
    });
    expect(pending).toMatchObject({ ready: false, pending: true, message: expect.stringContaining('still downloading') });
  });

  it.each([
    ['failed', { ...inventory, speechModels: [{ id: 'parakeet-v3', is_downloaded: false, is_downloading: false }] }],
    ['cancelled', { ...inventory, speechModels: [{ id: 'parakeet-v3', is_downloaded: false, is_downloading: false }] }],
    ['missing', inventory],
  ] as Array<[string, SetupModelInventory]>)('reports a %s speech model as unavailable without calling it an API-key problem', (_state, state) => {
    expect(setupModelReadiness(transcriptionOnly, state)).toMatchObject({
      ready: false,
      pending: false,
      message: expect.stringContaining("isn't installed yet"),
    });
  });

  it('requires an enabled cleanup model and its runtime, then clears when both are installed', () => {
    const modelsInstalled: SetupModelInventory = {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: true, is_downloading: false }],
      cleanupModels: [{ id: 'qwen2.5-3b-instruct', is_downloaded: true, is_downloading: false }],
    };
    expect(setupModelReadiness(localWithCleanup, modelsInstalled)).toMatchObject({
      ready: false,
      pending: false,
      message: expect.stringContaining('cleanup engine'),
    });
    expect(setupModelReadiness(localWithCleanup, {
      ...modelsInstalled,
      cleanupRuntime: { installed: false, is_downloading: true },
    })).toMatchObject({ ready: false, pending: true });
    expect(setupModelReadiness(localWithCleanup, {
      ...modelsInstalled,
      cleanupRuntime: { installed: true, is_downloading: false },
    })).toMatchObject({ ready: true, pending: false, message: '' });
  });

  it('ignores optional cleanup requirements when Writing Style is Off', () => {
    const speechInstalled: SetupModelInventory = {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: true, is_downloading: false }],
    };
    const off = setupCleanupEnabled('none', localWithCleanup);
    const on = setupCleanupEnabled('medium', localWithCleanup);

    expect(off).toBe(false);
    expect(setupModelReadiness(localWithCleanup, speechInstalled, off)).toEqual({ ready: true, pending: false, message: '' });
    expect(on).toBe(true);
    expect(setupModelReadiness(localWithCleanup, speechInstalled, on)).toMatchObject({
      ready: false,
      pending: false,
      message: expect.stringContaining('cleanup model and cleanup engine'),
    });
  });

  it('distinguishes a pending download from other missing local requirements', () => {
    const readiness = setupModelReadiness(localWithCleanup, {
      ...inventory,
      speechModels: [{ id: 'parakeet-v3', is_downloaded: false, is_downloading: true }],
      transcriptionState: { is_downloading: true, downloading_model_id: 'parakeet-v3' },
    });
    expect(readiness).toMatchObject({ ready: false, pending: true });
    expect(readiness.message).toContain('speech model is still downloading');
    expect(readiness.message).toContain("cleanup model and cleanup engine aren't installed yet");

    const staleDownloadState = setupModelReadiness(transcriptionOnly, {
      ...inventory,
      transcriptionState: { is_downloading: false, downloading_model_id: 'parakeet-v3' },
    });
    expect(staleDownloadState).toMatchObject({ ready: false, pending: false });
    expect(staleDownloadState.message).toContain("speech model isn't installed yet");
  });

  it('does not require any local files for a cloud preset', () => {
    expect(setupModelReadiness({
      ...transcriptionOnly,
      transcriptionDefaultModel: 'groq/whisper-large-v3-turbo',
      cleanupEnabled: true,
      cleanupDefaultModel: 'openai/gpt-4o-mini',
      requiredLocalModels: [],
    }, inventory)).toMatchObject({ ready: true, pending: false, message: '' });
  });
});
