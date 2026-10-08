import { beforeEach, describe, expect, it, vi } from 'vitest';
import { searchSettings } from './settingsSearch.svelte';

const platform = vi.hoisted(() => ({ android: false }));
vi.mock('./platform', async (importOriginal) => ({
  ...await importOriginal<typeof import('./platform')>(),
  get isAndroid() { return platform.android; },
}));
beforeEach(() => { platform.android = false; });

const visibleSections = ['general', 'keys', 'models', 'privacy', 'advanced', 'about'] as const;

describe('settings search', () => {
  it('keeps the offline pill setting out of desktop search', () => {
    expect(searchSettings('hide pill', visibleSections)).toEqual([]);
  });

  it('finds the offline pill setting on Android', () => {
    platform.android = true;
    expect(searchSettings('hide pill', visibleSections)[0]).toMatchObject({
      target: 'general-hide-pill-offline',
    });
  });
  it('finds setting content instead of only section names', () => {
    expect(searchSettings('hotkey', visibleSections)[0]).toMatchObject({
      section: 'general',
      target: 'general-hotkey',
    });
  });

  it('finds a specific model and routes it to the relevant model task', () => {
    expect(searchSettings('gpt 4o mini transcribe', visibleSections)[0]).toMatchObject({
      label: 'GPT-4o mini Transcribe',
      section: 'models',
      target: 'models-transcription',
    });
  });

  it('routes color searches to the accent picker', () => {
    expect(searchSettings('orange', visibleSections)[0]).toMatchObject({
      section: 'general',
      target: 'general-accent',
    });
  });

  it('does not return results from hidden settings sections', () => {
    expect(searchSettings('notification test', visibleSections)).toEqual([]);
  });

  it('routes microphone mute-button dictation to audio settings', () => {
    expect(searchSettings('mute button dictation', visibleSections)[0]).toMatchObject({
      section: 'advanced',
      target: 'audio-mic-mute-button',
    });
  });

  it('finds synchronous muting by its name and routes it to sync', () => {
    expect(searchSettings('synchronous muting', ['sync'])[0]).toMatchObject({
      label: 'Synchronous muting',
      section: 'sync',
      target: 'sync-muting',
    });
  });

  it('finds the Android pill settings only on Android', () => {
    expect(searchSettings('pill', visibleSections, 24, true).map((entry) => entry.id)).toEqual(
      expect.arrayContaining(['general-pill-position', 'general-cover-keyboard-mic']),
    );
    expect(searchSettings('pill', visibleSections, 24, false)).toEqual([]);
  });
});
