import { describe, expect, it } from 'vitest';
import { SETTINGS_SECTION_ORDER, visibleSettingsSections } from './settingsSections';

const ids = (opts: Parameters<typeof visibleSettingsSections>[0]) =>
  visibleSettingsSections(opts).flatMap((group) => group.items).map((item) => item.id);

describe('settings navigation', () => {
  it('shows Sync without enabling developer features', () => {
    expect(ids({ isMac: false, devMode: false })).toContain('sync');
  });

  it('places Text beside General in the Dictation group', () => {
    const dictation = visibleSettingsSections({ isMac: false, devMode: false }).find((group) => group.group === 'Dictation');
    expect(dictation?.items.map((item) => item.id).slice(0, 3)).toEqual(['general', 'text', 'advanced']);
  });

  it('keeps Models next to Providers', () => {
    const order = ids({ isMac: true, devMode: true, legacyMode: true });
    expect(order.indexOf('models')).toBe(order.indexOf('keys') + 1);
  });

  it('gives every section a unique id and label', () => {
    const sections = visibleSettingsSections({ isMac: true, devMode: true, legacyMode: true }).flatMap((group) => group.items);
    expect(new Set(sections.map((item) => item.id)).size).toBe(sections.length);
    expect(new Set(sections.map((item) => item.label)).size).toBe(sections.length);
    expect(SETTINGS_SECTION_ORDER).toEqual(sections.map((item) => item.id));
  });
});
