import { beforeEach, describe, expect, it, vi } from 'vitest';

const ipc = vi.hoisted(() => ({ invoke: vi.fn(), emit: vi.fn(), listen: vi.fn() }));
vi.mock('./tauri', () => ipc);

import { appStore } from './stores';
import { applyPalette, withAppearanceLock } from './appearanceActions';
import {
  cancelThemeEditor,
  confirmDiscardThemeEditor,
  guardThemeEditor,
  isThemeEditorDirty,
  keepEditingTheme,
  openThemeEditor,
  requestCloseThemeEditor,
  saveThemeEditor,
  themeEditor,
  updateThemeDraft,
  deleteThemeFromEditor,
} from './themeEditor.svelte';

const seed = { background: '#FCFCFA', foreground: '#111110', sidebar: null, surface: null };
const saved = { id: 't-1', name: 'Dusk', palette: { background: '#1E1E2E', foreground: '#CDD6F4', sidebar: null, surface: null }, accent: '#4F7FD8' };
let failKey: string | null;
let failOnce: string | null;

beforeEach(() => {
  vi.stubGlobal('document', { documentElement: { dataset: {} } });
  vi.stubGlobal('window', { dispatchEvent: vi.fn() });
  failKey = null;
  failOnce = null;
  ipc.emit.mockReset().mockResolvedValue(undefined);
  ipc.invoke.mockReset().mockImplementation(async (_cmd: string, args: { key: string }) => {
    if (args.key === failKey) throw new Error('disk full');
    if (args.key === failOnce) {
      failOnce = null;
      throw new Error('disk full');
    }
  });
  if (themeEditor.open) cancelThemeEditor();
  appStore.appearanceMode = 'light';
  appStore.customTheme = null;
  appStore.accentColor = null;
  appStore.activeThemeId = null;
  appStore.savedThemes = [];
});

const saves = () => ipc.invoke.mock.calls.filter(([cmd]) => cmd === 'save_setting').map(([, a]) => [a.key, a.value]);

describe('theme editor', () => {
  it('previews edits live and restores the exact prior look on cancel', () => {
    appStore.accentColor = '#D2637A';
    openThemeEditor({ seed, name: 'Mine' });
    expect(appStore.appearanceMode).toBe('custom');
    updateThemeDraft({ palette: { ...seed, background: '#000000' }, accent: null });
    expect(appStore.customTheme?.background).toBe('#000000');
    expect(appStore.accentColor).toBeNull();
    expect(isThemeEditorDirty()).toBe(true);
    cancelThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
    expect(appStore.customTheme).toBeNull();
    expect(appStore.accentColor).toBe('#D2637A');
    expect(saves()).toEqual([]);
  });

  it('asks before discarding a dirty draft and only runs the action on discard', () => {
    openThemeEditor({ seed, name: 'Mine' });
    updateThemeDraft({ name: 'Renamed' });
    const action = vi.fn();
    guardThemeEditor(action);
    expect(action).not.toHaveBeenCalled();
    expect(themeEditor.pendingAction).not.toBeNull();
    keepEditingTheme();
    expect(themeEditor.open).toBe(true);
    guardThemeEditor(action);
    confirmDiscardThemeEditor();
    expect(action).toHaveBeenCalledOnce();
    expect(themeEditor.open).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
  });

  it('closes a clean draft without asking', () => {
    openThemeEditor({ theme: saved });
    requestCloseThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(themeEditor.pendingAction).toBeNull();
  });

  it('saves the library then applies the palette and accent', async () => {
    openThemeEditor({ seed, name: 'Mine', accent: '#4F7FD8' });
    await saveThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(appStore.savedThemes).toHaveLength(1);
    const keys = saves().map(([k]) => k);
    expect(keys).toEqual(expect.arrayContaining(['custom_themes', 'custom_theme', 'appearance_mode', 'accent_color']));
    expect(keys.indexOf('custom_themes')).toBeLessThan(keys.indexOf('custom_theme'));
    expect(appStore.appearanceMode).toBe('custom');
    expect(appStore.accentColor).toBe('#4F7FD8');
    expect(appStore.activeThemeId).toBe(appStore.savedThemes[0].id);
  });

  it('keeps the draft open and the library unchanged when the library save fails', async () => {
    failKey = 'custom_themes';
    openThemeEditor({ seed, name: 'Mine' });
    await saveThemeEditor();
    expect(themeEditor.open).toBe(true);
    expect(themeEditor.error).toMatch(/disk full|save/i);
    expect(appStore.savedThemes).toEqual([]);
    expect(appStore.appearanceMode).toBe('custom');
  });

  it('rolls back the library and keeps previewing when applying fails', async () => {
    failKey = 'appearance_mode';
    openThemeEditor({ seed, name: 'Mine' });
    await saveThemeEditor();
    expect(themeEditor.open).toBe(true);
    expect(appStore.savedThemes).toEqual([]);
    expect(themeEditor.error).toMatch(/nothing changed/i);
    expect(appStore.customTheme?.background).toBe('#FCFCFA'.toUpperCase());
    // The saved custom_theme and library were put back.
    const last = (key: string) => { const m = saves().filter(([k]) => k === key); return m[m.length - 1]?.[1]; };
    expect(last('custom_themes')).toEqual([]);
    expect(last('custom_theme')).toBeNull();
  });

  it('rejects duplicate and empty names without saving', async () => {
    appStore.savedThemes = [saved];
    openThemeEditor({ seed, name: 'Dusk' });
    updateThemeDraft({ name: 'dusk' });
    await saveThemeEditor();
    expect(themeEditor.error).toMatch(/exists/);
    updateThemeDraft({ name: ' ' });
    await saveThemeEditor();
    expect(themeEditor.error).toMatch(/name/i);
    expect(saves()).toEqual([]);
  });

  it('deletes a saved theme and restores the prior look', async () => {
    appStore.savedThemes = [saved];
    openThemeEditor({ theme: saved });
    await deleteThemeFromEditor();
    expect(appStore.savedThemes).toEqual([]);
    expect(themeEditor.open).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
  });

  it('keeps the theme when deleting fails', async () => {
    failKey = 'custom_themes';
    appStore.savedThemes = [saved];
    openThemeEditor({ theme: saved });
    await deleteThemeFromEditor();
    expect(appStore.savedThemes).toEqual([saved]);
    expect(themeEditor.open).toBe(true);
    expect(themeEditor.error).toMatch(/still saved/i);
  });
});

describe('applyPalette rollback', () => {
  const palette = { background: '#101010', foreground: '#EEEEEE', sidebar: null, surface: null };

  function persistedLook() {
    appStore.appearanceMode = 'light';
    appStore.customTheme = null;
    appStore.accentColor = '#D2637A';
    appStore.activeThemeId = 'prior';
    return { mode: 'light' as const, customTheme: null, accent: '#D2637A', themeId: 'prior' };
  }

  async function failAt(key: string) {
    const from = persistedLook();
    // The store holds an editor preview that differs from what is persisted.
    appStore.appearanceMode = 'custom';
    appStore.customTheme = seed;
    appStore.accentColor = '#4F7FD8';
    failOnce = key;
    const ok = await applyPalette(palette, { accent: '#8B6FD6', themeId: 'new', from });
    expect(ok).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
    expect(appStore.customTheme).toBeNull();
    expect(appStore.accentColor).toBe('#D2637A');
    expect(appStore.activeThemeId).toBe('prior');
    return (k: string) => {
      const m = saves().filter(([name]) => name === k);
      return m.length ? m[m.length - 1][1] : undefined;
    };
  }

  it('restores everything to the persisted look when the palette save fails', async () => {
    const last = await failAt('custom_theme');
    expect(last('appearance_mode')).toBeUndefined();
    expect(last('accent_color')).toBeUndefined();
  });

  it('restores disk and store when the mode save fails after the palette was written', async () => {
    const last = await failAt('appearance_mode');
    expect(last('custom_theme')).toBeNull();
  });

  it('restores disk and store when the accent save fails after palette and mode were written', async () => {
    const last = await failAt('accent_color');
    expect(last('custom_theme')).toBeNull();
    expect(last('appearance_mode')).toBe('light');
    expect(last('accent_color')).toBe('#D2637A');
  });

  it('ignores a second appearance save while one is running', async () => {
    let release!: () => void;
    const first = withAppearanceLock(() => new Promise<string>((resolve) => { release = () => resolve('done'); }));
    expect(appStore.appearanceSaving).toBe(true);
    expect(await withAppearanceLock(async () => 'second')).toBeUndefined();
    release();
    expect(await first).toBe('done');
    expect(appStore.appearanceSaving).toBe(false);
  });
});
