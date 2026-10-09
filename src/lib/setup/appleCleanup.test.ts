import { describe, expect, it, vi } from 'vitest';
import { buildPresets, type Hardware } from '../components/settings/modelPresets';
import { appleCleanupOffer, appleCleanupPanel, appleCleanupReadiness, applyAppleCleanup, applyAppleCleanupDefaults, APPLE_CLEANUP_MODEL, missingLocalModels } from './appleCleanup';
import { getProviderLogo } from './ProviderLogos';
import { setupCleanupEnabled, setupDefaultModels, setupModelReadiness } from './setupModelReadiness';

vi.mock('../tauri', () => ({ invoke: vi.fn() }));
vi.mock('../platform', () => ({ isAndroid: false }));

const status = (state: string, available = false) => ({ state, available, message: `Message for ${state}` });
const noKeys = { groq: false, openai: false, google: false, assemblyai: false, openrouter: false, xai: false, local: false };
const hardware: Hardware = { totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: false };
const inventory = {
  speechModels: [], cleanupModels: [],
  transcriptionState: { is_downloading: false, downloading_model_id: null },
  cleanupState: { is_downloading: false, downloading_model_id: null },
  cleanupRuntime: { installed: false, is_downloading: false },
};

describe('Apple Intelligence onboarding offer', () => {
  it('is selectable only when available', () => {
    expect(appleCleanupOffer(status('available', true))).toEqual({ visible: true, selectable: true, reason: '' });
  });
  it.each(['intelligence-disabled', 'model-not-ready'])('shows %s with its reason but cannot be chosen', state => {
    expect(appleCleanupOffer(status(state))).toEqual({ visible: true, selectable: false, reason: `Message for ${state}` });
  });
  it.each(['checking', 'unknown', 'unavailable', 'unsupported-platform', 'unsupported-os', 'device-not-eligible', 'anything-else'])('hides the option for %s', state => {
    expect(appleCleanupOffer(status(state)).visible).toBe(false);
    expect(appleCleanupOffer(status(state, true)).visible).toBe(false);
  });
});

describe('Apple Intelligence onboarding panel', () => {
  it('offers a supported Mac, chosen or not', () => {
    expect(appleCleanupPanel(status('available', true), false)).toBe('offer');
    expect(appleCleanupPanel(status('model-not-ready'), true)).toBe('offer');
  });
  it.each(['checking', 'unknown', 'unsupported-platform'])('recovers a chosen cleanup that is %s instead of dropping it', state => {
    expect(appleCleanupPanel(status(state), true)).toBe('recovery');
  });
  it.each(['checking', 'unsupported-platform'])('hides the card for %s when nothing was chosen', state => {
    expect(appleCleanupPanel(status(state), false)).toBe('hidden');
  });
  it('hides the card again once the explicit choice is cleared', () => {
    expect(appleCleanupPanel(status('unsupported-platform'), false)).toBe('hidden');
  });
  it('keeps blocking a chosen cleanup while recovering, until it is cleared', () => {
    expect(appleCleanupReadiness(true, true, status('checking')).ready).toBe(false);
    expect(appleCleanupReadiness(false, true, status('checking')).ready).toBe(true);
  });
});

describe('Apple Intelligence onboarding target', () => {
  const local = buildPresets(noKeys, hardware, true, { includeTranscriptionOnly: true, localOnly: true });

  it('keeps speech selections and moves cleanup to Apple without fallbacks or downloads', () => {
    const balanced = local.find(p => p.id === 'local-balanced')!.target!;
    const next = applyAppleCleanup(balanced, true)!;
    expect(next.transcriptionDefaultModel).toBe(balanced.transcriptionDefaultModel);
    expect(next.transcriptionFallbacks).toEqual(balanced.transcriptionFallbacks);
    expect(next.dualTranscription).toBe(balanced.dualTranscription);
    expect(next.cleanupEnabled).toBe(true);
    expect(next.cleanupDefaultModel).toBe(APPLE_CLEANUP_MODEL);
    expect(next.cleanupFallbacks).toEqual([]);
    expect(next.requiredLocalModels.every(m => m.task === 'transcription')).toBe(true);
  });
  it('turns cleanup on for a speech-only preset only through the explicit opt-in', () => {
    const only = local.find(p => p.id === 'local-transcription-only')!.target!;
    expect(applyAppleCleanup(only, false)).toBe(only);
    expect(applyAppleCleanup(only, true)!.cleanupEnabled).toBe(true);
  });
  it('is a no-op without a preset and keeps intensity none off', () => {
    expect(applyAppleCleanup(null, true)).toBeNull();
    const target = applyAppleCleanup(local[1].target, true);
    expect(setupCleanupEnabled('none', target)).toBe(false);
    expect(setupCleanupEnabled('medium', target)).toBe(true);
  });
  it('replaces only the cleanup default when no preset was chosen', () => {
    const defaults = setupDefaultModels('groq');
    expect(applyAppleCleanupDefaults(defaults, false)).toBe(defaults);
    expect(applyAppleCleanupDefaults(defaults, true)).toEqual({ ...defaults, cleanupDefaultModel: APPLE_CLEANUP_MODEL });
  });
  it('needs no cleanup model or engine in the readiness check', () => {
    const target = applyAppleCleanup(local.find(p => p.id === 'local-balanced')!.target, true)!;
    const missing = setupModelReadiness(target, inventory, true);
    expect(missing.message).not.toMatch(/cleanup/);
    const ready = setupModelReadiness(target, { ...inventory, speechModels: [{ id: target.requiredLocalModels[0].id, is_downloaded: true, is_downloading: false }] }, true);
    expect(ready.ready).toBe(true);
  });
});

describe('Apple Intelligence picker downloads', () => {
  const local = buildPresets(noKeys, hardware, true, { includeTranscriptionOnly: true, localOnly: true });
  const nothingInstalled = { transcription: [], cleanup: [] };
  const original = (id: string) => local.find(p => p.id === id)!.target!;
  const ids = (models: { id: string }[]) => models.map(m => m.id);

  it('starts no cleanup download when Apple is opted in before choosing a preset', () => {
    const balanced = original('local-balanced');
    expect(balanced.requiredLocalModels.some(m => m.task === 'cleanup')).toBe(true);
    const downloads = missingLocalModels(balanced, nothingInstalled, true);
    expect(downloads.some(m => m.task === 'cleanup')).toBe(false);
    expect(ids(downloads)).toEqual(ids(balanced.requiredLocalModels.filter(m => m.task === 'transcription')));
  });
  it('keeps the original cleanup download when Apple is off', () => {
    const balanced = original('local-balanced');
    expect(missingLocalModels(balanced, nothingInstalled, false)).toEqual(balanced.requiredLocalModels);
  });
  it('keeps speech downloads for the speech-only preset and skips installed speech', () => {
    const only = original('local-transcription-only');
    expect(missingLocalModels(only, nothingInstalled, true)).toEqual(only.requiredLocalModels);
    const installed = { transcription: only.requiredLocalModels.map(m => m.id), cleanup: [] };
    expect(missingLocalModels(only, installed, true)).toEqual([]);
  });
  it('treats a preset with no target as needing nothing', () => {
    expect(missingLocalModels(null, nothingInstalled, true)).toEqual([]);
  });
});

describe('Apple Intelligence readiness after opt-in', () => {
  it('is not a concern until chosen', () => {
    expect(appleCleanupReadiness(false, true, status('model-not-ready'))).toEqual({ ready: true, message: '' });
  });
  it('is not a concern when cleanup will not run, so Off keeps the original behaviour', () => {
    expect(appleCleanupReadiness(true, false, status('model-not-ready'))).toEqual({ ready: true, message: '' });
    expect(appleCleanupReadiness(true, false, status('checking')).ready).toBe(true);
  });
  it('stays ready while available', () => {
    expect(appleCleanupReadiness(true, true, status('available', true)).ready).toBe(true);
  });
  it.each(['intelligence-disabled', 'model-not-ready', 'checking', 'unavailable', 'unsupported-platform'])('blocks a chosen Apple cleanup that is %s instead of falling back', state => {
    const result = appleCleanupReadiness(true, true, status(state));
    expect(result.ready).toBe(false);
    expect(result.message).toContain('Apple Intelligence');
    expect(result.message).toMatch(/Models/);
  });
  it('keeps the Apple target and its speech requirements when it becomes unready', () => {
    const local = buildPresets(noKeys, hardware, true, { includeTranscriptionOnly: true, localOnly: true });
    const target = local.find(p => p.id === 'local-balanced')!.target!;
    const chosen = applyAppleCleanup(target, true)!;
    expect(chosen.cleanupDefaultModel).toBe(APPLE_CLEANUP_MODEL);
    expect(chosen.requiredLocalModels.filter(m => m.task === 'transcription'))
      .toEqual(target.requiredLocalModels.filter(m => m.task === 'transcription'));
  });
});

describe('Apple Intelligence logo', () => {
  it('renders the bundled official PNG unchanged, with no remote request', () => {
    const svg = getProviderLogo('apple-intelligence');
    expect(svg).toContain('<image');
    expect(svg).not.toMatch(/https?:\/\/(?!www\.w3\.org)/);
    expect(svg).not.toContain('currentColor');
    expect(svg).toMatch(/apple-intelligence[^"]*\.png/);
  });
});
