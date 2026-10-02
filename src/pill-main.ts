import { mount } from 'svelte';
import PillApp from './PillApp.svelte';
import { invoke, listen } from './lib/tauri';
import { ACCENT_CHANGE_EVENT, applyAccentTheme, normalizeAccentColor } from './lib/accentTheme';
import { disableBrowserContextMenu } from './lib/disable-context-menu';
import { OMARCHY_THEME_EVENT, applyOmarchyPalette, effectiveAccent, isOmarchyTheme, resolvePalette, type OmarchyTheme } from './lib/omarchyTheme';
import { CUSTOM_THEME_CHANGE_EVENT, normalizeCustomTheme, type CustomTheme } from './lib/customTheme';
import './theme.css';

disableBrowserContextMenu(); // The pill webview lives until the process exits.

type AppearanceMode = 'system' | 'light' | 'dark' | 'omarchy' | 'custom';
const isAppearanceMode = (value: unknown): value is AppearanceMode =>
  value === 'system' || value === 'light' || value === 'dark' || value === 'omarchy' || value === 'custom';
type EffectiveTheme = 'light' | 'dark';
const APPEARANCE_CHANGE_EVENT = 'verenu:appearance-mode-changed';

function systemTheme(): EffectiveTheme {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

let currentMode: AppearanceMode = 'system';
let omarchyTheme: OmarchyTheme | null = null;
let customTheme: CustomTheme | null = null;
let accentSetting: string | null = null;

function activePalette() {
  return resolvePalette(currentMode, omarchyTheme, customTheme);
}

function applyAccent() {
  applyAccentTheme(document.documentElement, effectiveAccent(accentSetting, activePalette()), { animate: false });
}

function applyTheme(mode: AppearanceMode) {
  currentMode = mode;
  const palette = activePalette();
  document.documentElement.dataset.theme =
    mode === 'light' || mode === 'dark' ? mode : (palette?.mode ?? systemTheme());
  applyOmarchyPalette(document.documentElement, palette);
  applyAccent();
}

applyTheme('system');

(async () => {
  try {
    const [mode, accentColor, theme, custom] = await Promise.all([
      invoke<AppearanceMode | null>('get_setting', { key: 'appearance_mode' }),
      invoke<string | null>('get_setting', { key: 'accent_color' }),
      invoke<OmarchyTheme | null>('get_omarchy_theme').catch(() => null),
      invoke<unknown>('get_setting', { key: 'custom_theme' }).catch(() => null),
    ]);
    customTheme = normalizeCustomTheme(custom);
    omarchyTheme = isOmarchyTheme(theme) ? theme : null;
    accentSetting = normalizeAccentColor(accentColor);
    applyTheme(isAppearanceMode(mode) ? mode : currentMode);
  } catch {}
})();

void listen<string | null>(ACCENT_CHANGE_EVENT, (event) => {
  accentSetting = normalizeAccentColor(event.payload);
  applyAccent();
});

void listen<AppearanceMode>(APPEARANCE_CHANGE_EVENT, (event) => {
  if (isAppearanceMode(event.payload)) applyTheme(event.payload);
});

void listen<OmarchyTheme | null>(OMARCHY_THEME_EVENT, (event) => {
  omarchyTheme = isOmarchyTheme(event.payload) ? event.payload : null;
  applyTheme(currentMode);
});

void listen<unknown>(CUSTOM_THEME_CHANGE_EVENT, (event) => {
  customTheme = normalizeCustomTheme(event.payload);
  applyTheme(currentMode);
});

window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', () => {
  if (currentMode === 'system') applyTheme('system');
});

mount(PillApp, { target: document.getElementById('pill-root')! });
