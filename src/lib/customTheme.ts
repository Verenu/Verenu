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

/** A readable text color for a background, used while the palette is automatic. */
export function deriveForeground(background: string): string {
  return isDarkColor(background) ? '#E8E8E8' : '#1A1A1A';
}

/** True when the palette sets anything the basic editor would have chosen itself. */
export function hasPaletteOverrides(theme: CustomTheme): boolean {
  const background = normalizeAccentColor(theme.background);
  const foreground = normalizeAccentColor(theme.foreground);
  if (theme.sidebar || theme.surface) return true;
  return !background || !foreground || foreground.toLowerCase() !== deriveForeground(background).toLowerCase();
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

/** The editor's automatic palette for a mode: sidebar and surface derived, text chosen for contrast. */
export function basicThemePalette(dark: boolean): CustomTheme {
  const { background } = (dark ? CUSTOM_THEME_PRESETS[0] : CUSTOM_THEME_PRESETS[4]).theme;
  return { background, foreground: deriveForeground(background), sidebar: null, surface: null };
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

// ── Saved theme library ──────────────────────────────────────────────────
// The active palette stays in `custom_theme` (colors only) and the accent in
// `accent_color`. Named themes the user creates live in the separate
// device-local `custom_themes` list, so revisiting or editing one never loses
// it and the active-palette shape stays unchanged.

export type SavedTheme = {
  id: string;
  name: string;
  palette: CustomTheme;
  /** Accent the theme applies; null keeps the theme's own default accent. */
  accent: string | null;
};

export const MAX_SAVED_THEMES = 24;
export const THEME_NAME_MAX = 40;
const THEME_ID = /^[A-Za-z0-9_-]{1,64}$/;

export function newThemeId(): string {
  const random = globalThis.crypto?.randomUUID?.().replace(/-/g, '').slice(0, 12)
    ?? Math.random().toString(36).slice(2, 14);
  return `t-${random}`;
}

export function normalizeSavedThemes(value: unknown): SavedTheme[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const themes: SavedTheme[] = [];
  for (const item of value) {
    const raw = (item && typeof item === 'object' ? item : {}) as Record<string, unknown>;
    const palette = normalizeCustomTheme(raw.palette);
    const id = typeof raw.id === 'string' ? raw.id : '';
    const name = typeof raw.name === 'string' ? raw.name.trim().slice(0, THEME_NAME_MAX) : '';
    if (!palette || !THEME_ID.test(id) || !name || seen.has(id)) continue;
    seen.add(id);
    themes.push({ id, name, palette, accent: normalizeAccentColor(raw.accent) });
    if (themes.length >= MAX_SAVED_THEMES) break;
  }
  return themes;
}

/** Only the colors, the shape stored in `custom_theme`. */
export function themeColors(theme: CustomTheme): CustomTheme {
  return {
    background: theme.background,
    foreground: theme.foreground,
    sidebar: theme.sidebar ?? null,
    surface: theme.surface ?? null,
  };
}

export function sameThemeColors(a: CustomTheme | null | undefined, b: CustomTheme | null | undefined): boolean {
  if (!a || !b) return false;
  return CUSTOM_THEME_FIELDS.every(({ id }) =>
    (normalizeAccentColor(a[id])?.toLowerCase() ?? null) === (normalizeAccentColor(b[id])?.toLowerCase() ?? null));
}

/** Returns an error message, or null when the name can be saved. */
export function validateThemeName(name: string, themes: SavedTheme[], selfId: string | null): string | null {
  const trimmed = name.trim();
  if (!trimmed) return 'Give the theme a name.';
  if (trimmed.length > THEME_NAME_MAX) return `Use ${THEME_NAME_MAX} characters or fewer.`;
  const taken = themes.some((t) => t.id !== selfId && t.name.toLowerCase() === trimmed.toLowerCase());
  return taken ? 'A theme with that name already exists.' : null;
}

export function upsertSavedTheme(themes: SavedTheme[], theme: SavedTheme): SavedTheme[] {
  return themes.some((t) => t.id === theme.id)
    ? themes.map((t) => (t.id === theme.id ? theme : t))
    : [...themes, theme];
}

/** First "Name", then "Name 2", "Name 3" … that no saved theme uses. */
export function uniqueThemeName(base: string, themes: SavedTheme[]): string {
  const root = base.trim().slice(0, THEME_NAME_MAX - 3) || 'Custom theme';
  const used = new Set(themes.map((t) => t.name.toLowerCase()));
  if (!used.has(root.toLowerCase())) return root;
  for (let n = 2; n < 100; n += 1) {
    const candidate = `${root} ${n}`;
    if (!used.has(candidate.toLowerCase())) return candidate;
  }
  return root;
}

/**
 * The saved theme the active palette corresponds to. `hintId` (the theme last
 * applied this session) breaks ties between saved themes with identical colors.
 */
export function findActiveSavedTheme(
  themes: SavedTheme[],
  active: CustomTheme | null,
  hintId: string | null,
): SavedTheme | null {
  if (!active) return null;
  const matches = themes.filter((t) => sameThemeColors(t.palette, active));
  return matches.find((t) => t.id === hintId) ?? matches[0] ?? null;
}

/** Concrete colors for a miniature preview, derived like applyOmarchyPalette does. */
export function previewColors(theme: CustomTheme): { background: string; foreground: string; sidebar: string; surface: string } {
  const background = normalizeAccentColor(theme.background) ?? '#FFFFFF';
  const foreground = normalizeAccentColor(theme.foreground) ?? '#000000';
  return {
    background,
    foreground,
    sidebar: normalizeAccentColor(theme.sidebar) ?? `color-mix(in srgb, ${foreground} 2%, ${background})`,
    surface: normalizeAccentColor(theme.surface) ?? `color-mix(in srgb, ${foreground} 5%, ${background})`,
  };
}

/** Stock Light and Dark colors, only for drawing preview cards. */
export const STOCK_PREVIEWS = {
  light: { background: '#FCFCFA', foreground: '#111110', sidebar: '#FBFBF9', surface: '#FFFFFE' },
  dark: { background: '#161514', foreground: '#F5F4F0', sidebar: '#10100F', surface: '#1C1B1A' },
} as const;
