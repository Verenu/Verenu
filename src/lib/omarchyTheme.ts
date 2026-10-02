// Palette-driven appearance: maps a palette (the active Omarchy theme's
// colors.toml, read by the backend, or the user's Custom hex colors) onto
// Verenu's surface, ink, line, status, overlay, and pill tokens. Status colors
// come from the palette's red/green/yellow only when they are vivid enough to
// read as status; monochrome themes keep Verenu's own so errors stay visible.
import { normalizeAccentColor, isAdaptiveDefaultAccent, relativeLuminance } from './accentTheme';
import { customPalette, type CustomTheme } from './customTheme';

export const OMARCHY_THEME_EVENT = 'verenu:omarchy-theme-changed';

export type OmarchyTheme = {
  name: string;
  mode: 'dark' | 'light';
  colors: Record<string, string>;
  /** The user typed this text color, so it is used as-is. Theme palettes keep text neutral. */
  customText?: boolean;
};

const OMARCHY_PROPERTIES = [
  '--paper', '--paper-2', '--paper-3',
  '--amber-50', '--amber-100', '--amber-200',
  '--sidebar-bg', '--bg-elev',
  '--ink', '--ink-strong', '--ink-soft', '--ink-mute', '--ink-faint',
  '--arm-200', '--arm-300', '--arm-400', '--arm-500', '--arm-600',
  '--arm-700', '--arm-800', '--arm-900', '--arm-950',
  '--line', '--line-soft', '--line-strong',
  '--control-hover', '--control-active',
  '--accent-theme-default',
  '--danger', '--danger-bg', '--danger-line',
  '--success', '--success-bg', '--success-line',
  '--warning', '--warning-bg', '--warning-line',
  '--overlay', '--shadow-elev', '--shadow-popover',
  '--pill-bg', '--pill-fg', '--pill-bar', '--pill-muted', '--pill-muted-strong',
  '--pill-line', '--pill-hover', '--pill-spinner-track', '--pill-shine-clear',
  '--pill-shine-mid', '--pill-shine-strong',
] as const;

const STATUS_SOURCES = {
  danger: ['bright_red', 'red'],
  success: ['bright_green', 'green'],
  warning: ['bright_yellow', 'yellow', 'orange'],
} as const;

/** Palette colors flatter than this (0-255 channel spread) read as gray, not as a status. */
const MIN_STATUS_CHROMA = 48;
const MIN_STATUS_CONTRAST = 3.2;

function channels(hex: string): number[] {
  return [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
}

function chroma(hex: string): number {
  const values = channels(hex);
  return Math.max(...values) - Math.min(...values);
}

function mixHex(from: string, to: string, amount: number): string {
  const a = channels(from);
  const b = channels(to);
  const out = a.map((value, index) => Math.round(value + (b[index] - value) * amount));
  return `#${out.map((value) => value.toString(16).padStart(2, '0')).join('')}`;
}

function contrast(a: string, b: string): number {
  const [light, dark] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

/** The most vivid palette color for a status, nudged toward the text color until it reads on the background. */
function statusColor(colors: Record<string, string>, names: readonly string[]): string | null {
  const candidates = names
    .map((name) => normalizeAccentColor(colors[name])?.toLowerCase())
    .filter((hex): hex is string => !!hex && chroma(hex) >= MIN_STATUS_CHROMA)
    .sort((a, b) => chroma(b) - chroma(a));
  const picked = candidates[0];
  if (!picked) return null;
  const background = normalizeAccentColor(colors.background);
  const foreground = normalizeAccentColor(colors.foreground);
  if (!background || !foreground) return null;
  let color = picked;
  for (let step = 1; step <= 8 && contrast(color, background) < MIN_STATUS_CONTRAST; step += 1) {
    color = mixHex(picked, foreground, step * 0.1);
  }
  return color;
}

export function isOmarchyTheme(value: unknown): value is OmarchyTheme {
  if (!value || typeof value !== 'object') return false;
  const theme = value as Partial<OmarchyTheme>;
  return (
    (theme.mode === 'dark' || theme.mode === 'light')
    && !!theme.colors
    && !!normalizeAccentColor(theme.colors.background)
    && !!normalizeAccentColor(theme.colors.foreground)
  );
}

/** Applies or clears the palette. Pass `null` to restore theme.css tokens. */
export function applyOmarchyPalette(root: HTMLElement, theme: OmarchyTheme | null): void {
  if (!isOmarchyTheme(theme)) {
    for (const property of OMARCHY_PROPERTIES) root.style.removeProperty(property);
    return;
  }
  const c = theme.colors;
  const bg = normalizeAccentColor(c.background)!.toLowerCase();
  const fg = normalizeAccentColor(c.foreground)!.toLowerCase();
  const edge = theme.mode === 'dark' ? '#ffffff' : '#000000';
  // Text stays neutral (white on dark, near-black on light) so a green or
  // brown theme never tints it. Only a color the user typed is used as given.
  const neutralInk = theme.mode === 'dark' ? '#f4f4f4' : '#141414';
  const ink = theme.customText ? fg : neutralInk;
  const mix = (amount: number, base = bg) => `color-mix(in srgb, ${ink} ${amount}%, ${base})`;
  const alpha = (amount: number) => `color-mix(in srgb, ${ink} ${amount}%, transparent)`;
  // Secondary text steps are gray rather than a blend into the themed background.
  const toneBase = theme.mode === 'dark' ? '#000000' : '#ffffff';
  const tone = (amount: number) =>
    theme.customText ? mix(amount) : `color-mix(in srgb, ${ink} ${amount}%, ${toneBase})`;
  const elevated = c.lighter_background && c.lighter_background !== bg ? c.lighter_background : mix(3);
  const sidebar = c.dark_background && c.dark_background !== bg ? c.dark_background : mix(2);

  // Overlays and shadows darken the page; on a light theme the text color is
  // the darkest tone the palette offers.
  const shade = theme.mode === 'dark' ? (c.darker_background ?? '#000000') : ink;

  const palette: Partial<Record<(typeof OMARCHY_PROPERTIES)[number], string>> = {
    '--paper': bg,
    '--paper-2': mix(4),
    '--paper-3': mix(8),
    '--amber-50': bg,
    '--amber-100': mix(4),
    '--amber-200': mix(8),
    '--sidebar-bg': sidebar,
    '--bg-elev': elevated,
    '--ink': ink,
    '--ink-strong': `color-mix(in srgb, ${ink} 85%, ${edge})`,
    '--ink-soft': tone(82),
    '--ink-mute': tone(60),
    '--ink-faint': tone(42),
    '--arm-200': mix(14),
    '--arm-300': mix(22),
    '--arm-400': tone(42),
    '--arm-500': tone(60),
    '--arm-600': tone(72),
    '--arm-700': tone(82),
    '--arm-800': ink,
    '--arm-900': ink,
    // Always-dark surfaces (the Home tile) use this; light themes darken the
    // foreground so white copy and accent keys keep their contrast.
    '--arm-950': theme.mode === 'dark' ? (c.darker_background ?? sidebar) : `color-mix(in srgb, ${ink} 45%, #000000)`,
    '--line': mix(14),
    '--line-soft': mix(9),
    '--line-strong': mix(22),
    '--control-hover': mix(7),
    '--control-active': mix(11),
    '--accent-theme-default': c.accent ?? ink,
    ...statusTokens(c, bg),
    '--overlay': `color-mix(in srgb, ${shade} 68%, transparent)`,
    '--shadow-elev': `0 20px 60px -12px color-mix(in srgb, ${shade} 50%, transparent)`,
    '--shadow-popover': `0 8px 24px color-mix(in srgb, ${shade} 36%, transparent)`,
    '--pill-bg': sidebar,
    '--pill-fg': ink,
    '--pill-bar': ink,
    '--pill-muted': alpha(45),
    '--pill-muted-strong': alpha(85),
    '--pill-line': alpha(10),
    '--pill-hover': alpha(14),
    '--pill-spinner-track': alpha(16),
    '--pill-shine-clear': alpha(0),
    '--pill-shine-mid': alpha(50),
    '--pill-shine-strong': ink,
  };
  for (const property of OMARCHY_PROPERTIES) {
    const value = palette[property];
    if (value) root.style.setProperty(property, value);
    else root.style.removeProperty(property);
  }
}

type StatusToken = '--danger' | '--danger-bg' | '--danger-line'
  | '--success' | '--success-bg' | '--success-line'
  | '--warning' | '--warning-bg' | '--warning-line';

function statusTokens(colors: Record<string, string>, background: string): Partial<Record<StatusToken, string>> {
  const tokens: Partial<Record<StatusToken, string>> = {};
  for (const status of ['danger', 'success', 'warning'] as const) {
    const color = statusColor(colors, STATUS_SOURCES[status]);
    if (!color) continue;
    tokens[`--${status}`] = color;
    tokens[`--${status}-bg`] = `color-mix(in srgb, ${color} 14%, ${background})`;
    tokens[`--${status}-line`] = `color-mix(in srgb, ${color} 42%, ${background})`;
  }
  return tokens;
}

/**
 * The palette the current appearance mode paints with, or `null` for the stock
 * Light/Dark look. System follows the desktop, so on Omarchy (the only place
 * `omarchy` is ever non-null) it means the active Omarchy theme.
 */
export function resolvePalette(
  mode: string,
  omarchy: OmarchyTheme | null,
  custom: CustomTheme | null,
): OmarchyTheme | null {
  if (mode === 'omarchy' || mode === 'system') return omarchy;
  if (mode === 'custom') return customPalette(custom);
  return null;
}

/**
 * Under a palette the palette's accent (or, for Custom, the typed text color
 * when it has none) is the default. A custom accent the user picked still wins; the black/white
 * "follow theme" swatches do not.
 */
export function effectiveAccent(accentColor: string | null, palette: OmarchyTheme | null): string | null {
  if (!palette) return accentColor;
  if (accentColor && !isAdaptiveDefaultAccent(accentColor)) return accentColor;
  const fallback = palette.customText ? palette.colors.foreground : undefined;
  return normalizeAccentColor(palette.colors.accent ?? fallback) ?? accentColor;
}
