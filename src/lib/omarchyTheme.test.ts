import { describe, expect, it } from 'vitest';
import { applyOmarchyPalette, effectiveAccent, resolvePalette, type OmarchyTheme } from './omarchyTheme';
import { customPalette, normalizeCustomTheme } from './customTheme';

function fakeRoot() {
  const values = new Map<string, string>();
  return {
    values,
    root: {
      style: {
        setProperty: (name: string, value: string) => void values.set(name, value),
        removeProperty: (name: string) => void values.delete(name),
      },
    } as unknown as HTMLElement,
  };
}

const solitude: OmarchyTheme = {
  name: 'solitude',
  mode: 'dark',
  colors: {
    accent: '#798186', background: '#101315', foreground: '#cacccc',
    red: '#565d60', bright_red: '#de6145', green: '#9fa5a9', bright_green: '#343d41',
    yellow: '#d9dbdc', bright_yellow: '#c9c2b4',
  },
};

describe('applyOmarchyPalette', () => {
  it('normalizes required colors and clears an invalid palette', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, { ...solitude, colors: { ...solitude.colors, background: ' #101315 ', foreground: ' #CACCCC ' } });
    expect(values.get('--paper')).toBe('#101315');
    expect(values.get('--danger')).toMatch(/^#[0-9a-f]{6}$/);
    applyOmarchyPalette(root, { ...solitude, colors: { background: 'bad', foreground: '#ffffff' } });
    expect(values.size).toBe(0);
  });
  it('themes surfaces, text, overlays, and shadows from the palette', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, solitude);
    expect(values.get('--paper')).toBe('#101315');
    expect(values.get('--ink')).toBe('#f4f4f4');
    expect(values.get('--overlay')).toContain('#000000');
    expect(values.get('--shadow-elev')).toContain('#000000');
  });

  it('takes status colors only from vivid palette entries', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, solitude);
    expect(values.get('--danger')).toBe('#de6145');
    expect(values.get('--danger-bg')).toContain('#de6145');
    // Grayscale greens and yellows stay on Verenu's own status colors.
    expect(values.has('--success')).toBe(false);
    expect(values.has('--warning')).toBe(false);
  });

  it('pulls a low-contrast status color toward the text color', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, {
      name: 'dim', mode: 'dark',
      colors: { background: '#101315', foreground: '#e0e0e0', bright_red: '#5a0000' },
    });
    expect(values.get('--danger')).not.toBe('#5a0000');
    expect(values.get('--danger')).toMatch(/^#[0-9a-f]{6}$/);
  });

  it('keeps Omarchy text neutral instead of tinting it with the theme', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, { ...solitude, colors: { ...solitude.colors, foreground: '#c8e6c9' } });
    expect(values.get('--ink')).toBe('#f4f4f4');
    expect(values.get('--pill-fg')).toBe('#f4f4f4');
    expect(values.get('--ink-mute')).not.toContain('#101315');
    const light = fakeRoot();
    applyOmarchyPalette(light.root, { name: 'latte', mode: 'light', colors: { background: '#f2efe9', foreground: '#5a4a2a' } });
    expect(light.values.get('--ink')).toBe('#141414');
  });

  it('uses the typed text color for Custom palettes', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, customPalette({ background: '#0B3D2E', foreground: '#E6F4EA' }));
    expect(values.get('--ink')).toBe('#e6f4ea');
  });

  it('clears everything when the palette is removed', () => {
    const { root, values } = fakeRoot();
    applyOmarchyPalette(root, solitude);
    applyOmarchyPalette(root, null);
    expect(values.size).toBe(0);
  });
});

describe('custom palettes', () => {
  const custom = { background: '#F2EFE9', foreground: '#2A2A2A', sidebar: null, surface: '#FFFFFF' };

  it('derives light or dark from the background', () => {
    expect(customPalette(custom)?.mode).toBe('light');
    expect(customPalette({ background: '#1E1E2E', foreground: '#CDD6F4' })?.mode).toBe('dark');
  });

  it('maps optional colors onto palette keys and skips unset ones', () => {
    const palette = customPalette(custom)!;
    expect(palette.colors.lighter_background).toBe('#ffffff');
    expect(palette.colors.dark_background).toBeUndefined();
  });

  it('rejects incomplete or malformed saved values', () => {
    expect(normalizeCustomTheme({ background: '#fff' })).toBeNull();
    expect(normalizeCustomTheme({ background: 'red', foreground: '#000000' })).toBeNull();
    expect(normalizeCustomTheme(null)).toBeNull();
  });

  it('only paints System, Omarchy, and Custom modes', () => {
    expect(resolvePalette('dark', solitude, normalizeCustomTheme(custom))).toBeNull();
    expect(resolvePalette('light', solitude, null)).toBeNull();
    expect(resolvePalette('system', solitude, normalizeCustomTheme(custom))).toBe(solitude);
    expect(resolvePalette('system', null, null)).toBeNull();
    expect(resolvePalette('omarchy', solitude, null)).toBe(solitude);
    expect(resolvePalette('custom', solitude, null)).toBeNull();
    expect(resolvePalette('custom', solitude, normalizeCustomTheme(custom))?.name).toBe('custom');
  });
});

describe('effectiveAccent', () => {
  it('prefers a picked accent, then the palette accent, then its text color', () => {
    expect(effectiveAccent('#4F7FD8', solitude)).toBe('#4F7FD8');
    expect(effectiveAccent('#000000', solitude)).toBe('#798186'.toUpperCase());
    expect(effectiveAccent(null, customPalette({ background: '#1E1E2E', foreground: '#CDD6F4' }))).toBe('#CDD6F4');
    expect(effectiveAccent(null, { ...solitude, colors: { background: '#101315', foreground: '#cacccc' } })).toBeNull();
    expect(effectiveAccent('#4F7FD8', null)).toBe('#4F7FD8');
    expect(effectiveAccent(null, null)).toBeNull();
  });
});
