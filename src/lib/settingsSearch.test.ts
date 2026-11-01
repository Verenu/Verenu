import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SETTINGS_SEARCH_ENTRIES, searchSettings } from './settingsSearch.svelte';
import { SETTINGS_SECTION_ORDER } from './settingsSections';

const platform = vi.hoisted(() => ({ android: false }));
vi.mock('./platform', async (importOriginal) => ({
  ...await importOriginal<typeof import('./platform')>(),
  get isAndroid() { return platform.android; },
}));
beforeEach(() => { platform.android = false; });

const visibleSections = ['general', 'text', 'keys', 'models', 'privacy', 'advanced', 'about'] as const;

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

  it('routes voice command searches to the Text voice commands row', () => {
    for (const query of ['voice commands', 'spoken punctuation', 'scratch that', 'line break']) {
      expect(searchSettings(query, visibleSections)[0]).toMatchObject({
        section: 'text',
        target: 'text-voice-commands',
      });
    }
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

  it('routes LAN device sync search to paired devices', () => {
    expect(searchSettings('LAN device sync', ['sync'])[0]).toMatchObject({
      label: 'LAN device sync',
      section: 'sync',
      target: 'sync-paired',
    });
  });

  it('finds the Android pill settings only on Android', () => {
    expect(searchSettings('pill', visibleSections, 24, true).map((entry) => entry.id)).toEqual(
      expect.arrayContaining(['general-pill-position', 'general-cover-keyboard-mic']),
    );
    expect(searchSettings('pill', visibleSections, 24, false)).toEqual([]);
  });
});

const sources = Object.values(
  import.meta.glob('./components/**/*.svelte', { query: '?raw', import: 'default', eager: true }),
).join('\n');

describe('settings search targets', () => {
  // The Models page builds its targets from the task id and belongs to a separate redesign.
  const entries = SETTINGS_SEARCH_ENTRIES.filter((entry) => entry.section !== 'models');

  it('points every entry at a known section', () => {
    for (const entry of SETTINGS_SEARCH_ENTRIES) {
      expect(SETTINGS_SECTION_ORDER, entry.id).toContain(entry.section);
    }
  });

  it('points every entry at a data-setting-target rendered by a component', () => {
    const missing = entries.filter((entry) => {
      if (entry.target.startsWith('api-key-')) return !sources.includes('api-key-${');
      // A row component can receive the target as a prop instead of rendering the attribute itself.
      return !sources.includes(`data-setting-target="${entry.target}"`) && !sources.includes(`target="${entry.target}"`);
    });
    expect(missing.map((entry) => `${entry.id} -> ${entry.target}`)).toEqual([]);
  });
});
