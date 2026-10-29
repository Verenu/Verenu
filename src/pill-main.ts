import { mount } from 'svelte';
import PillApp from './PillApp.svelte';
import { invoke, listen } from './lib/tauri';
import { initializeDevSession, isBrowserDevSession } from './lib/devSession';
import { ACCENT_CHANGE_EVENT, applyAccentTheme, normalizeAccentColor } from './lib/accentTheme';
import { disableBrowserContextMenu } from './lib/disable-context-menu';
import { OMARCHY_THEME_EVENT, applyOmarchyPalette, effectiveAccent, isOmarchyTheme, resolvePalette, type OmarchyTheme } from './lib/omarchyTheme';
import { CUSTOM_THEME_CHANGE_EVENT, normalizeCustomTheme, type CustomTheme } from './lib/customTheme';
import './theme.css';

disableBrowserContextMenu();

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
let appearanceVersion = 0;
let accentVersion = 0;
let customVersion = 0;
let omarchyVersion = 0;

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
    if (isBrowserDevSession()) await initializeDevSession();
    await Promise.all([
      listen<string | null>(ACCENT_CHANGE_EVENT, (event) => {
        accentVersion++;
        accentSetting = normalizeAccentColor(event.payload);
        applyAccent();
      }),
      listen<AppearanceMode>(APPEARANCE_CHANGE_EVENT, (event) => {
        appearanceVersion++;
        if (isAppearanceMode(event.payload)) applyTheme(event.payload);
      }),
      listen<OmarchyTheme | null>(OMARCHY_THEME_EVENT, (event) => {
        omarchyVersion++;
        omarchyTheme = isOmarchyTheme(event.payload) ? event.payload : null;
        applyTheme(currentMode);
      }),
      listen<unknown>(CUSTOM_THEME_CHANGE_EVENT, (event) => {
        customVersion++;
        customTheme = normalizeCustomTheme(event.payload);
        applyTheme(currentMode);
      }),
    ]);
    const versions = { appearance: appearanceVersion, accent: accentVersion, custom: customVersion, omarchy: omarchyVersion };
    const [mode, accentColor, theme, custom] = await Promise.all([
      invoke<AppearanceMode | null>('get_setting', { key: 'appearance_mode' }),
      invoke<string | null>('get_setting', { key: 'accent_color' }),
      invoke<OmarchyTheme | null>('get_omarchy_theme').catch(() => null),
      invoke<unknown>('get_setting', { key: 'custom_theme' }).catch(() => null),
    ]);
    if (versions.custom === customVersion) customTheme = normalizeCustomTheme(custom);
    if (versions.omarchy === omarchyVersion) omarchyTheme = isOmarchyTheme(theme) ? theme : null;
    if (versions.accent === accentVersion) accentSetting = normalizeAccentColor(accentColor);
    applyTheme(versions.appearance === appearanceVersion && isAppearanceMode(mode) ? mode : currentMode);
  } catch {}
})();

window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', () => {
  if (currentMode === 'system') applyTheme('system');
});

mount(PillApp, { target: document.getElementById('pill-root')! });
