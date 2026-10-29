import { tick } from 'svelte';
import { emit, invoke } from './tauri';
import { appStore } from './stores';
import { type AppearanceMode } from './settings';
import { ACCENT_CHANGE_EVENT, animateAccentChange, isAdaptiveDefaultAccent, normalizeAccentColor } from './accentTheme';
import {
  CUSTOM_THEME_CHANGE_EVENT,
  defaultCustomTheme,
  themeColors,
  normalizeCustomTheme,
  type CustomTheme,
} from './customTheme';

const APPEARANCE_MODE_CHANGE_EVENT = 'verenu:appearance-mode-changed';

function broadcastCustomTheme(theme: CustomTheme | null) {
  void emit(CUSTOM_THEME_CHANGE_EVENT, theme).catch((err) => {
    console.warn('broadcast custom theme failed:', err);
  });
}

export function broadcastAppearance(mode: AppearanceMode, theme: CustomTheme | null, accent: string | null) {
  broadcastCustomTheme(theme);
  void emit(ACCENT_CHANGE_EVENT, accent).catch((err) => {
    console.warn('broadcast preview accent failed:', err);
  });
  void emit(APPEARANCE_MODE_CHANGE_EVENT, mode).catch((err) => {
    console.warn('broadcast preview appearance failed:', err);
  });
}

export async function setAppearanceMode(
  mode: AppearanceMode,
  from: { mode: AppearanceMode; customTheme: CustomTheme | null; accent: string | null } = {
    mode: appStore.appearanceMode,
    customTheme: appStore.customTheme,
    accent: appStore.accentColor,
  },
) {
  const version = appStore.appearanceChangeVersion;
  try {
    const expected = await readAppearanceSnapshot(from, version);
    const accent = isAdaptiveDefaultAccent(from.accent) ? null : from.accent;
    const theme = mode === 'custom' && !from.customTheme
      ? defaultCustomTheme(document.documentElement.dataset.theme === 'dark') : from.customTheme;
    await invoke('save_model_settings_if_unchanged', { expected, values: { ...expected, appearance_mode: mode, accent_color: accent, custom_theme: theme } });
    if (version !== appStore.appearanceChangeVersion) {
      window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
      return;
    }
    await animateAccentChange(async () => {
      if (version !== appStore.appearanceChangeVersion) return;
      appStore.appearanceMode = mode;
      appStore.accentColor = accent;
      appStore.customTheme = theme;
      await tick();
    });
    if (version !== appStore.appearanceChangeVersion) {
      window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
      return;
    }
    broadcastAppearance(mode, theme, accent);
  } catch (err) {
    window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
    console.error('save appearance_mode failed:', err);
    throw err;
  }
}

export async function persistAccentColor(
  color: string | null,
  previous: string | null = appStore.accentColor,
): Promise<boolean> {
  const next = isAdaptiveDefaultAccent(color) ? null : color;
  const version = appStore.appearanceChangeVersion;
  const before = { ...snapshotAppearance(), accent: previous };
  try {
    const expected = await readAppearanceSnapshot(before, version);
    await invoke('save_model_settings_if_unchanged', { expected, values: { ...expected, accent_color: next } });
    if (version !== appStore.appearanceChangeVersion) {
      window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
      return true;
    }
    await animateAccentChange(async () => {
      if (version !== appStore.appearanceChangeVersion) return;
      appStore.accentColor = next;
      await tick();
    });
    if (version !== appStore.appearanceChangeVersion) {
      window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
      return true;
    }
    void emit(ACCENT_CHANGE_EVENT, next).catch((err) => {
      console.warn('broadcast accent color failed:', err);
    });
    return true;
  } catch (err) {
    window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
    console.error('save accent_color failed:', err);
    return false;
  }
}

export type AppearanceSnapshot = {
  mode: AppearanceMode;
  customTheme: CustomTheme | null;
  accent: string | null;
  themeId: string | null;
};

export function snapshotAppearance(): AppearanceSnapshot {
  return {
    mode: appStore.appearanceMode,
    customTheme: appStore.customTheme,
    accent: appStore.accentColor,
    themeId: appStore.activeThemeId,
  };
}

async function readAppearanceSnapshot(before: { mode: AppearanceMode; customTheme: CustomTheme | null; accent: string | null }, version: number) {
  const [appearance_mode, accent_color, custom_theme, custom_themes] = await Promise.all([
    invoke('get_setting', { key: 'appearance_mode' }),
    invoke('get_setting', { key: 'accent_color' }),
    invoke('get_setting', { key: 'custom_theme' }),
    invoke('get_setting', { key: 'custom_themes' }),
  ]);
  if ((appearance_mode ?? 'system') !== before.mode
    || normalizeAccentColor(accent_color) !== before.accent
    || JSON.stringify(normalizeCustomTheme(custom_theme)) !== JSON.stringify(before.customTheme)
    || version !== appStore.appearanceChangeVersion) throw new Error('Appearance changed while editing');
  return { appearance_mode, accent_color, custom_theme, custom_themes };
}

export async function applyPalette(
  palette: CustomTheme,
  options: { accent?: string | null; themeId?: string | null; from?: AppearanceSnapshot } = {},
): Promise<boolean> {
  const before = options.from ?? snapshotAppearance();
  const version = appStore.appearanceChangeVersion;
  try {
    const expected = await readAppearanceSnapshot(before, version);
    const accent = 'accent' in options ? options.accent ?? null : (isAdaptiveDefaultAccent(before.accent) ? null : before.accent);
    const values = { ...expected, appearance_mode: 'custom', custom_theme: themeColors(palette), accent_color: accent };
    await invoke('save_model_settings_if_unchanged', { expected, values });
    if (version !== appStore.appearanceChangeVersion) {
      window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
      return true;
    }
    appStore.appearanceMode = 'custom';
    appStore.customTheme = values.custom_theme;
    appStore.accentColor = accent;
    appStore.activeThemeId = options.themeId ?? null;
    broadcastAppearance('custom', values.custom_theme, accent);
    return true;
  } catch {
    window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
    return false;
  }
}

export async function withAppearanceLock<T>(task: () => Promise<T>): Promise<T | undefined> {
  if (appStore.appearanceSaving) return undefined;
  appStore.appearanceChangeVersion++;
  appStore.appearanceSaving = true;
  try {
    return await task();
  } finally {
    appStore.appearanceSaving = false;
  }
}
