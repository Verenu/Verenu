// Custom appearance mode: a palette the user types in as hex codes. It is
// converted into the same shape as an Omarchy palette, so one mapping
// (applyOmarchyPalette) themes the whole UI for both.
import { normalizeAccentColor, relativeLuminance } from './accentTheme';
import type { OmarchyTheme } from './omarchyTheme';

export const CUSTOM_THEME_CHANGE_EVENT = 'verenu:custom-theme-changed';

export type CustomTheme = {
  background: string;
  foreground: string;
  /** Sidebar and pill surface. Derived from the background when absent. */
  sidebar?: string | null;
  /** Raised surfaces such as cards, menus, and modals. Derived when absent. */
  surface?: string | null;
};

export type CustomThemeField = 'background' | 'foreground' | 'sidebar' | 'surface';

export const CUSTOM_THEME_FIELDS: { id: CustomThemeField; label: string; optional: boolean }[] = [
  { id: 'background', label: 'Background', optional: false },
  { id: 'foreground', label: 'Text', optional: false },
  { id: 'sidebar', label: 'Sidebar', optional: true },
  { id: 'surface', label: 'Surface', optional: true },
];

export const CUSTOM_THEME_PRESETS: { label: string; theme: CustomTheme }[] = [
  { label: 'Catppuccin Mocha', theme: { background: '#1E1E2E', foreground: '#CDD6F4', sidebar: '#181825', surface: '#313244' } },
  { label: 'Tokyo Night', theme: { background: '#1A1B26', foreground: '#C0CAF5', sidebar: '#16161E', surface: '#24283B' } },
  { label: 'Nord', theme: { background: '#2E3440', foreground: '#D8DEE9', sidebar: '#272C36', surface: '#3B4252' } },
  { label: 'Gruvbox Dark', theme: { background: '#282828', foreground: '#EBDBB2', sidebar: '#1D2021', surface: '#3C3836' } },
  { label: 'Catppuccin Latte', theme: { background: '#EFF1F5', foreground: '#4C4F69', sidebar: '#E6E9EF', surface: '#FFFFFF' } },
  { label: 'Solarized Light', theme: { background: '#FDF6E3', foreground: '#586E75', sidebar: '#EEE8D5', surface: '#FFFBF0' } },
];

export function normalizeCustomTheme(value: unknown): CustomTheme | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const background = normalizeAccentColor(raw.background);
  const foreground = normalizeAccentColor(raw.foreground);
  if (!background || !foreground) return null;
  return {
    background,
    foreground,
    sidebar: normalizeAccentColor(raw.sidebar),
    surface: normalizeAccentColor(raw.surface),
  };
}

export function isDarkColor(hex: string): boolean {
  return relativeLuminance(hex) <= 0.4;
}

/**
 * Starting point when Custom is first chosen (and on Reset): the background and
 * text of the preset matching the current light/dark look. Sidebar and surface
 * stay derived, so editing the background never leaves a stale surface behind.
 */
export function defaultCustomTheme(dark: boolean): CustomTheme {
  const { background, foreground } = (dark ? CUSTOM_THEME_PRESETS[0] : CUSTOM_THEME_PRESETS[4]).theme;
  return { background, foreground, sidebar: null, surface: null };
}

export function customPalette(custom: CustomTheme | null): OmarchyTheme | null {
  const theme = normalizeCustomTheme(custom);
  if (!theme) return null;
  const colors: Record<string, string> = {
    background: theme.background.toLowerCase(),
    foreground: theme.foreground.toLowerCase(),
  };
  if (theme.sidebar) colors.dark_background = theme.sidebar.toLowerCase();
  if (theme.surface) colors.lighter_background = theme.surface.toLowerCase();
  return { name: 'custom', mode: isDarkColor(theme.background) ? 'dark' : 'light', colors, customText: true };
}
