import { describe, expect, it } from 'vitest';
import { visibleSettingsSections } from './settingsSections';

describe('settings navigation', () => {
  it('shows Sync without enabling developer features', () => {
    const sections = visibleSettingsSections({ isMac: false, devMode: false });
    expect(sections.flatMap((group) => group.items).map((item) => item.id)).toContain('sync');
  });
});
