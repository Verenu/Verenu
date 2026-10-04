import { describe, expect, it } from 'vitest';
import {
  MAX_SAVED_THEMES,
  basicThemePalette,
  deriveForeground,
  hasPaletteOverrides,
  isDarkColor,
  findActiveSavedTheme,
  normalizeSavedThemes,
  sameThemeColors,
  uniqueThemeName,
  upsertSavedTheme,
  validateThemeName,
  type SavedTheme,
} from './customTheme';

const palette = { background: '#1e1e2e', foreground: '#cdd6f4', sidebar: null, surface: null };
const theme = (id: string, name: string, accent: string | null = null): SavedTheme => ({ id, name, palette, accent });

describe('saved themes', () => {
  it('normalizes entries and drops malformed, duplicate, or excess ones', () => {
    const many = Array.from({ length: MAX_SAVED_THEMES + 5 }, (_, i) => ({ id: `t${i}`, name: `T${i}`, palette, accent: null }));
    const result = normalizeSavedThemes([
      { id: 'a', name: ' Night ', palette, accent: '#4f7fd8' },
      { id: 'a', name: 'Dupe id', palette, accent: null },
      { id: 'bad id', name: 'Space in id', palette, accent: null },
      { id: 'b', name: '', palette, accent: null },
      { id: 'c', name: 'No palette' },
      { id: 'd', name: 'Bad accent', palette, accent: 'blue' },
      'junk',
      ...many,
    ]);
    expect(result.slice(0, 2)).toEqual([
      { id: 'a', name: 'Night', palette: { background: '#1E1E2E', foreground: '#CDD6F4', sidebar: null, surface: null }, accent: '#4F7FD8' },
      { id: 'd', name: 'Bad accent', palette: expect.anything(), accent: null },
    ]);
    expect(result).toHaveLength(MAX_SAVED_THEMES);
    expect(normalizeSavedThemes(null)).toEqual([]);
  });

  it('validates names for emptiness, length, and case-insensitive duplicates', () => {
    const list = [theme('a', 'Night')];
    expect(validateThemeName('  ', list, null)).toMatch(/name/i);
    expect(validateThemeName('x'.repeat(41), list, null)).toMatch(/40/);
    expect(validateThemeName('night', list, null)).toMatch(/exists/);
    expect(validateThemeName('night', list, 'a')).toBeNull();
    expect(validateThemeName('Dawn', list, null)).toBeNull();
  });

  it('suffixes unique names and upserts by id', () => {
    const list = [theme('a', 'Night'), theme('b', 'Night 2')];
    expect(uniqueThemeName('Night', list)).toBe('Night 3');
    expect(uniqueThemeName('Dawn', list)).toBe('Dawn');
    const renamed = upsertSavedTheme(list, theme('a', 'Evening'));
    expect(renamed.map((t) => t.name)).toEqual(['Evening', 'Night 2']);
    expect(upsertSavedTheme(list, theme('c', 'New'))).toHaveLength(3);
  });

  it('finds the active theme by colors, preferring the session hint', () => {
    const list = [theme('a', 'One'), theme('b', 'Two')];
    expect(findActiveSavedTheme(list, { ...palette }, null)?.id).toBe('a');
    expect(findActiveSavedTheme(list, { ...palette }, 'b')?.id).toBe('b');
    expect(findActiveSavedTheme(list, { ...palette, background: '#000000' }, null)).toBeNull();
    expect(sameThemeColors(palette, { ...palette, background: '#1E1E2E' })).toBe(true);
  });
});

describe('basic theme palette', () => {
  it('derives contrasting text from the background', () => {
    expect(deriveForeground('#101010')).toBe('#E8E8E8');
    expect(deriveForeground('#F5F5F5')).toBe('#1A1A1A');
  });

  it('builds an automatic palette per mode', () => {
    const dark = basicThemePalette(true);
    const light = basicThemePalette(false);
    expect(isDarkColor(dark.background)).toBe(true);
    expect(isDarkColor(light.background)).toBe(false);
    expect(dark.sidebar).toBeNull();
    expect(dark.surface).toBeNull();
    expect(hasPaletteOverrides(dark)).toBe(false);
    expect(hasPaletteOverrides(light)).toBe(false);
  });

  it('flags sidebar, surface, or non-derived text as overrides', () => {
    const base = basicThemePalette(true);
    expect(hasPaletteOverrides({ ...base, sidebar: '#181825' })).toBe(true);
    expect(hasPaletteOverrides({ ...base, surface: '#313244' })).toBe(true);
    expect(hasPaletteOverrides({ ...base, foreground: '#CDD6F4' })).toBe(true);
    expect(hasPaletteOverrides({ ...base, foreground: base.foreground.toLowerCase() })).toBe(false);
  });
});
