import { beforeEach, describe, expect, it, vi } from 'vitest';

const ipc = vi.hoisted(() => ({ invoke: vi.fn(), emit: vi.fn(), listen: vi.fn() }));
vi.mock('./tauri', () => ipc);

import { appStore } from './stores';
import { applyPalette, withAppearanceLock } from './appearanceActions';
import { ACCENT_CHANGE_EVENT } from './accentTheme';
import { CUSTOM_THEME_CHANGE_EVENT } from './customTheme';
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
  ipc.invoke.mockReset().mockImplementation(async (cmd: string, args: { key: string; values?: Record<string, unknown> }) => {
    if (cmd === 'get_setting') {
      return ({ appearance_mode: appStore.appearanceMode, accent_color: appStore.accentColor,
        custom_theme: appStore.customTheme, custom_themes: appStore.savedThemes } as Record<string, unknown>)[args.key];
    }
    if (cmd === 'save_model_settings_if_unchanged') {
      if (failKey && args.values && failKey in args.values) throw new Error('disk full');
      return;
    }
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
    expect(ipc.emit).toHaveBeenCalledWith(CUSTOM_THEME_CHANGE_EVENT, seed);
    expect(ipc.emit).toHaveBeenCalledWith(ACCENT_CHANGE_EVENT, null);
    expect(ipc.emit).toHaveBeenCalledWith('verenu:appearance-mode-changed', 'custom');
    ipc.emit.mockClear();
    updateThemeDraft({ palette: { ...seed, background: '#000000' }, accent: null });
    expect(appStore.customTheme?.background).toBe('#000000');
    expect(appStore.accentColor).toBeNull();
    expect(ipc.emit).toHaveBeenCalledWith(CUSTOM_THEME_CHANGE_EVENT, { ...seed, background: '#000000' });
    expect(ipc.emit).toHaveBeenCalledWith(ACCENT_CHANGE_EVENT, null);
    expect(ipc.emit).toHaveBeenCalledWith('verenu:appearance-mode-changed', 'custom');
    expect(isThemeEditorDirty()).toBe(true);
    ipc.emit.mockClear();
    cancelThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
    expect(appStore.customTheme).toBeNull();
    expect(appStore.accentColor).toBe('#D2637A');
    expect(ipc.emit).toHaveBeenCalledWith(CUSTOM_THEME_CHANGE_EVENT, null);
    expect(ipc.emit).toHaveBeenCalledWith(ACCENT_CHANGE_EVENT, '#D2637A');
    expect(ipc.emit).toHaveBeenCalledWith('verenu:appearance-mode-changed', 'light');
    expect(saves()).toEqual([]);
  });

  it('asks before discarding a dirty draft and only runs the action on discard', async () => {
    openThemeEditor({ seed, name: 'Mine' });
    updateThemeDraft({ name: 'Renamed' });
    const action = vi.fn();
    guardThemeEditor(action);
    expect(action).not.toHaveBeenCalled();
    expect(themeEditor.pendingAction).not.toBeNull();
    keepEditingTheme();
    expect(themeEditor.open).toBe(true);
    guardThemeEditor(action);
    await confirmDiscardThemeEditor();
    expect(action).toHaveBeenCalledOnce();
    expect(themeEditor.open).toBe(false);
    expect(appStore.appearanceMode).toBe('light');
  });

  it('closes a clean draft without asking', async () => {
    openThemeEditor({ theme: saved });
    await requestCloseThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(themeEditor.pendingAction).toBeNull();
  });

  it('does not open another editor from a stale conflicted draft', async () => {
    openThemeEditor({ seed, name: 'Mine' });
    updateThemeDraft({ name: 'Renamed' });
    const action = vi.fn();
    guardThemeEditor(action);
    themeEditor.externalChange = true;
    await confirmDiscardThemeEditor();
    expect(action).not.toHaveBeenCalled();
    expect(themeEditor.open).toBe(false);
  });

  it('waits for the baseline before switching away from a stale editor', async () => {
    let resolveMode: (value: string) => void = () => {};
    const mode = new Promise<string>(resolve => { resolveMode = resolve; });
    const invoke = ipc.invoke.getMockImplementation()!;
    ipc.invoke.mockImplementation((cmd, args) => cmd === 'get_setting' && args.key === 'appearance_mode' ? mode : invoke(cmd, args));
    openThemeEditor({ seed, name: 'Mine' });
    const action = vi.fn();
    const transition = guardThemeEditor(action);
    expect(action).not.toHaveBeenCalled();
    resolveMode('dark');
    await transition;
    expect(action).not.toHaveBeenCalled();
    expect(themeEditor.open).toBe(false);
    expect(window.dispatchEvent).toHaveBeenCalledWith(expect.objectContaining({ type: 'verenu:refresh-agent-appearance' }));
  });

  it('does not discard an edit made while a transition waits for the baseline', async () => {
    let resolveMode: (value: string) => void = () => {};
    const mode = new Promise<string>(resolve => { resolveMode = resolve; });
    const invoke = ipc.invoke.getMockImplementation()!;
    ipc.invoke.mockImplementation((cmd, args) => cmd === 'get_setting' && args.key === 'appearance_mode' ? mode : invoke(cmd, args));
    openThemeEditor({ seed, name: 'Mine' });
    const action = vi.fn();
    const transition = guardThemeEditor(action);
    updateThemeDraft({ name: 'New draft' });
    resolveMode('light');
    await transition;
    expect(themeEditor.open).toBe(true);
    expect(themeEditor.draft.name).toBe('New draft');
    expect(action).not.toHaveBeenCalled();
  });

  it('honors keep editing while discard is waiting for the baseline', async () => {
    let resolveMode: (value: string) => void = () => {};
    const mode = new Promise<string>(resolve => { resolveMode = resolve; });
    const invoke = ipc.invoke.getMockImplementation()!;
    ipc.invoke.mockImplementation((cmd, args) => cmd === 'get_setting' && args.key === 'appearance_mode' ? mode : invoke(cmd, args));
    openThemeEditor({ seed, name: 'Mine' });
    updateThemeDraft({ name: 'New draft' });
    const action = vi.fn();
    await guardThemeEditor(action);
    const transition = confirmDiscardThemeEditor();
    keepEditingTheme();
    resolveMode('light');
    await transition;
    expect(themeEditor.open).toBe(true);
    expect(action).not.toHaveBeenCalled();
  });

  it('saves the library then applies the palette and accent', async () => {
    openThemeEditor({ seed, name: 'Mine', accent: '#4F7FD8' });
    await saveThemeEditor();
    expect(themeEditor.open).toBe(false);
    expect(appStore.savedThemes).toHaveLength(1);
    const batch = ipc.invoke.mock.calls.find(([cmd]) => cmd === 'save_model_settings_if_unchanged');
    expect(batch?.[1].values).toMatchObject({ appearance_mode: 'custom', accent_color: '#4F7FD8', custom_theme: seed });
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

  it('keeps the library unchanged and the draft when the atomic save fails', async () => {
    failKey = 'appearance_mode';
    openThemeEditor({ seed, name: 'Mine' });
    await saveThemeEditor();
    expect(themeEditor.open).toBe(true);
    expect(appStore.savedThemes).toEqual([]);
    expect(themeEditor.error).toMatch(/save/i);
    expect(appStore.customTheme?.background).toBe('#FCFCFA'.toUpperCase());
    expect(saves()).toEqual([]);
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

describe('applyPalette atomic save', () => {
  const palette = { background: '#101010', foreground: '#EEEEEE', sidebar: null, surface: null };

  function persistedLook() {
    appStore.appearanceMode = 'light';
    appStore.customTheme = null;
    appStore.accentColor = '#D2637A';
    appStore.activeThemeId = 'prior';
    return { mode: 'light' as const, customTheme: null, accent: '#D2637A', themeId: 'prior' };
  }

  it('does not issue rollback writes when a concurrent change rejects the batch', async () => {
    const from = persistedLook();
    failKey = 'custom_theme';
    const ok = await applyPalette(palette, { accent: '#8B6FD6', themeId: 'new', from });
    expect(ok).toBe(false);
    expect(saves()).toEqual([]);
    expect(window.dispatchEvent).toHaveBeenCalledWith(expect.objectContaining({ type: 'verenu:refresh-agent-appearance' }));
  });

  it('commits the palette, mode and accent in one conditional batch', async () => {
    const from = persistedLook();
    expect(await applyPalette(palette, { accent: '#8B6FD6', themeId: 'new', from })).toBe(true);
    const batch = ipc.invoke.mock.calls.find(([cmd]) => cmd === 'save_model_settings_if_unchanged');
    expect(batch?.[1].values).toMatchObject({ appearance_mode: 'custom', custom_theme: palette, accent_color: '#8B6FD6' });
    expect(appStore.activeThemeId).toBe('new');
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
