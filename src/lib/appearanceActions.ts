import { tick } from 'svelte';
import { emit } from './tauri';
import { appStore } from './stores';
import { saveSetting, type AppearanceMode } from './settings';
import { ACCENT_CHANGE_EVENT, animateAccentChange, isAdaptiveDefaultAccent } from './accentTheme';
import {
  CUSTOM_THEME_CHANGE_EVENT,
  defaultCustomTheme,
  themeColors,
  type CustomTheme,
  type SavedTheme,
} from './customTheme';

const APPEARANCE_MODE_CHANGE_EVENT = 'verenu:appearance-mode-changed';

function broadcastCustomTheme(theme: CustomTheme | null) {
  void emit(CUSTOM_THEME_CHANGE_EVENT, theme).catch((err) => {
    console.warn('broadcast custom theme failed:', err);
  });
}

/** Keeps the separate dictation-pill window in sync with an unsaved preview or rollback. */
export function broadcastAppearance(mode: AppearanceMode, theme: CustomTheme | null, accent: string | null) {
  broadcastCustomTheme(theme);
  void emit(ACCENT_CHANGE_EVENT, accent).catch((err) => {
    console.warn('broadcast preview accent failed:', err);
  });
  void emit(APPEARANCE_MODE_CHANGE_EVENT, mode).catch((err) => {
    console.warn('broadcast preview appearance failed:', err);
  });
}

/** Saves the active custom palette. Throws (after restoring `previous`) on failure. */
export async function persistCustomTheme(next: CustomTheme | null, previous: CustomTheme | null = appStore.customTheme) {
  const colors = next ? themeColors(next) : null;
  appStore.customTheme = colors;
  broadcastCustomTheme(colors);
  try {
    await saveSetting('custom_theme', colors);
  } catch (err) {
    appStore.customTheme = previous;
    broadcastCustomTheme(previous);
    console.error('save custom_theme failed:', err);
    throw err;
  }
}

/** Saves the named theme list. Leaves the store untouched on failure. */
export async function persistSavedThemes(next: SavedTheme[]) {
  try {
    await saveSetting('custom_themes', next);
  } catch (err) {
    console.error('save custom_themes failed:', err);
    throw err;
  }
  appStore.savedThemes = next;
}

/**
 * Switches appearance mode. `from` is the last persisted mode when a live
 * preview has already moved the store, so a failure restores the real one.
 */
export async function setAppearanceMode(
  mode: AppearanceMode,
  from: { mode: AppearanceMode; customTheme: CustomTheme | null; accent: string | null } = {
    mode: appStore.appearanceMode,
    customTheme: appStore.customTheme,
    accent: appStore.accentColor,
  },
) {
  const previousAccent = from.accent;
  const resetAccentToThemeDefault = isAdaptiveDefaultAccent(previousAccent);
  let accentResetSaved = false;

  try {
    if (mode === 'custom' && !appStore.customTheme) {
      await persistCustomTheme(defaultCustomTheme(document.documentElement.dataset.theme === 'dark'), null);
    }
    if (resetAccentToThemeDefault) {
      await saveSetting('accent_color', null);
      accentResetSaved = true;
    }
    await saveSetting('appearance_mode', mode);

    await animateAccentChange(async () => {
      appStore.appearanceMode = mode;
      if (resetAccentToThemeDefault) appStore.accentColor = null;
      await tick();
    });
    if (resetAccentToThemeDefault) {
      void emit(ACCENT_CHANGE_EVENT, null).catch((err) => {
        console.warn('broadcast adaptive accent reset failed:', err);
      });
    }
  } catch (err) {
    if (accentResetSaved) {
      try {
        await saveSetting('accent_color', previousAccent);
      } catch (rollbackErr) {
        console.error('restore accent_color after appearance save failed:', rollbackErr);
      }
    }
    appStore.appearanceMode = from.mode;
    appStore.accentColor = previousAccent;
    console.error('save appearance_mode failed:', err);
    throw err;
  }
}

/** Saves the accent with animation; restores the previous accent and returns false on failure. */
export async function persistAccentColor(
  color: string | null,
  previous: string | null = appStore.accentColor,
): Promise<boolean> {
  const next = isAdaptiveDefaultAccent(color) ? null : color;
  await animateAccentChange(async () => {
    appStore.accentColor = next;
    await tick();
  });
  void emit(ACCENT_CHANGE_EVENT, next).catch((err) => {
    console.warn('broadcast accent color failed:', err);
  });
  try {
    await saveSetting('accent_color', next);
    return true;
  } catch (err) {
    await animateAccentChange(async () => {
      appStore.accentColor = previous;
      await tick();
    });
    void emit(ACCENT_CHANGE_EVENT, previous).catch((emitErr) => {
      console.warn('broadcast accent rollback failed:', emitErr);
    });
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

/**
 * Makes a palette (and optionally its accent) the active look. Everything
 * rolls back together if any save fails; returns whether it all stuck.
 */
export async function applyPalette(
  palette: CustomTheme,
  options: { accent?: string | null; themeId?: string | null; from?: AppearanceSnapshot } = {},
): Promise<boolean> {
  const before = options.from ?? snapshotAppearance();
  let wroteTheme = false;
  let wroteMode = false;
  let wroteAccent = false;
  try {
    await persistCustomTheme(palette, before.customTheme);
    wroteTheme = true;
    if (before.mode !== 'custom') {
      await setAppearanceMode('custom', before);
      wroteMode = true;
      wroteAccent = true;
    }
    if ('accent' in options) {
      wroteAccent = true;
      if (!(await persistAccentColor(options.accent ?? null, before.accent))) throw new Error('accent save failed');
    }
    appStore.activeThemeId = options.themeId ?? null;
    return true;
  } catch {
    await restoreAppearance(before, { theme: wroteTheme, mode: wroteMode, accent: wroteAccent });
    return false;
  }
}

async function restoreAppearance(
  before: AppearanceSnapshot,
  wrote: { theme: boolean; mode: boolean; accent: boolean },
) {
  const restore = async (key: 'custom_theme' | 'appearance_mode' | 'accent_color', value: unknown) => {
    try {
      await saveSetting(key, value as never);
    } catch (err) {
      console.error(`restore ${key} after appearance save failure failed:`, err);
    }
  };
  if (wrote.mode) await restore('appearance_mode', before.mode);
  if (wrote.accent) await restore('accent_color', before.accent);
  if (wrote.theme) await restore('custom_theme', before.customTheme ? themeColors(before.customTheme) : null);
  appStore.appearanceMode = before.mode;
  appStore.customTheme = before.customTheme;
  appStore.accentColor = before.accent;
  appStore.activeThemeId = before.themeId;
  broadcastAppearance(before.mode, before.customTheme, before.accent);
}

/**
 * Runs one appearance save at a time. A second request made while one is in
 * flight is ignored (resolves `undefined`) so saves never race each other.
 */
export async function withAppearanceLock<T>(task: () => Promise<T>): Promise<T | undefined> {
  if (appStore.appearanceSaving) return undefined;
  appStore.appearanceSaving = true;
  try {
    return await task();
  } finally {
    appStore.appearanceSaving = false;
  }
}
