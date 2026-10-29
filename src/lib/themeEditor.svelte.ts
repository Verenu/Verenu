import { appStore } from './stores';
import {
  broadcastAppearance,
  withAppearanceLock,
  snapshotAppearance,
  type AppearanceSnapshot,
} from './appearanceActions';
import {
  MAX_SAVED_THEMES,
  newThemeId,
  sameThemeColors,
  themeColors,
  uniqueThemeName,
  validateThemeName,
  upsertSavedTheme,
  normalizeCustomTheme,
  normalizeSavedThemes,
  type CustomTheme,
  type SavedTheme,
} from './customTheme';
import { formatIpcError } from './errors';
import { normalizeAccentColor } from './accentTheme';
import { invoke } from './tauri';

type Draft = { name: string; palette: CustomTheme; accent: string | null };
type AppearanceSettings = { appearance_mode: unknown; accent_color: unknown; custom_theme: unknown; custom_themes: unknown };
let settingsBaseline: Promise<AppearanceSettings | null> | null = null;
let transitionVersion = 0;

async function readAppearanceSettings(): Promise<AppearanceSettings> {
  const [appearance_mode, accent_color, custom_theme, custom_themes] = await Promise.all([
    invoke('get_setting', { key: 'appearance_mode' }),
    invoke('get_setting', { key: 'accent_color' }),
    invoke('get_setting', { key: 'custom_theme' }),
    invoke('get_setting', { key: 'custom_themes' }),
  ]);
  return { appearance_mode, accent_color, custom_theme, custom_themes };
}

export const themeEditor = $state({
  open: false,
  session: 0,
  editingId: null as string | null,
  draft: { name: '', palette: { background: '#FFFFFF', foreground: '#000000', sidebar: null, surface: null }, accent: null } as Draft,
  original: null as Draft | null,
  baseline: null as AppearanceSnapshot | null,
  saving: false,
  error: '',
  externalChange: false,
  confirmingDelete: false,
  pendingAction: null as (() => void) | null,
});

function copyDraft(draft: Draft): Draft {
  return { name: draft.name, palette: themeColors(draft.palette), accent: draft.accent };
}

export function isThemeEditorDirty(): boolean {
  const { original, draft } = themeEditor;
  if (!original) return false;
  return (
    draft.name.trim() !== original.name.trim()
    || draft.accent !== original.accent
    || !sameThemeColors(draft.palette, original.palette)
  );
}

function previewDraft() {
  appStore.appearanceMode = 'custom';
  appStore.customTheme = themeColors(themeEditor.draft.palette);
  appStore.accentColor = themeEditor.draft.accent;
  broadcastAppearance('custom', appStore.customTheme, appStore.accentColor);
}

function restoreBaseline() {
  const baseline = themeEditor.baseline;
  if (!baseline) return;
  appStore.appearanceMode = baseline.mode;
  appStore.customTheme = baseline.customTheme;
  appStore.accentColor = baseline.accent;
  appStore.activeThemeId = baseline.themeId;
  broadcastAppearance(baseline.mode, baseline.customTheme, baseline.accent);
}

function close() {
  themeEditor.open = false;
  themeEditor.editingId = null;
  themeEditor.original = null;
  themeEditor.baseline = null;
  themeEditor.error = '';
  themeEditor.confirmingDelete = false;
  themeEditor.pendingAction = null;
  themeEditor.saving = false;
  if (themeEditor.externalChange) {
    themeEditor.externalChange = false;
    window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
  }
}

export function openThemeEditor(source: { theme: SavedTheme } | { seed: CustomTheme; name: string; accent?: string | null }) {
  appStore.appearanceChangeVersion++;
  const baseline = snapshotAppearance();
  const saved = JSON.stringify(appStore.savedThemes);
  const session = themeEditor.session + 1;
  settingsBaseline = readAppearanceSettings().then(settings => {
    if ((settings.appearance_mode ?? 'system') !== baseline.mode
      || normalizeAccentColor(settings.accent_color) !== baseline.accent
      || JSON.stringify(normalizeCustomTheme(settings.custom_theme)) !== JSON.stringify(baseline.customTheme)
      || JSON.stringify(normalizeSavedThemes(settings.custom_themes)) !== saved) {
      if (themeEditor.session !== session) return settings;
      if (themeEditor.open) themeEditor.externalChange = true;
      else window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
    }
    return settings;
  }).catch(() => null);
  const keepBaseline = themeEditor.open ? themeEditor.baseline : null;
  const draft: Draft = 'theme' in source
    ? { name: source.theme.name, palette: themeColors(source.theme.palette), accent: source.theme.accent }
    : {
        name: uniqueThemeName(source.name, appStore.savedThemes),
        palette: themeColors(source.seed),
        accent: source.accent ?? null,
      };
  themeEditor.baseline = keepBaseline ?? snapshotAppearance();
  themeEditor.editingId = 'theme' in source ? source.theme.id : null;
  themeEditor.draft = copyDraft(draft);
  themeEditor.original = copyDraft(draft);
  themeEditor.error = '';
  themeEditor.externalChange = false;
  themeEditor.confirmingDelete = false;
  themeEditor.pendingAction = null;
  themeEditor.saving = false;
  themeEditor.session += 1;
  themeEditor.open = true;
  previewDraft();
}

export function updateThemeDraft(patch: Partial<Draft>) {
  if (!themeEditor.open || themeEditor.saving) return;
  transitionVersion++;
  const appearanceChanged = patch.palette !== undefined || patch.accent !== undefined;
  themeEditor.draft = {
    ...themeEditor.draft,
    ...patch,
    palette: patch.palette ? themeColors(patch.palette) : themeEditor.draft.palette,
  };
  themeEditor.error = '';
  if (appearanceChanged) previewDraft();
}

export function cancelThemeEditor() {
  if (!themeEditor.open) return;
  restoreBaseline();
  close();
}

export async function requestCloseThemeEditor() {
  if (!themeEditor.open || themeEditor.saving) return;
  if (isThemeEditorDirty()) themeEditor.pendingAction = () => {};
  else await finishTransition();
}

async function finishTransition(action?: () => void, confirmed = false) {
  const session = themeEditor.session;
  const version = ++transitionVersion;
  const baseline = await settingsBaseline;
  if (!themeEditor.open || themeEditor.session !== session || version !== transitionVersion) return;
  if (!confirmed && isThemeEditorDirty()) {
    themeEditor.pendingAction = action ?? (() => {});
    return;
  }
  if (!baseline) themeEditor.externalChange = true;
  const conflicted = themeEditor.externalChange;
  cancelThemeEditor();
  if (!conflicted) action?.();
}

export async function guardThemeEditor(action: () => void) {
  if (!themeEditor.open) return action();
  if (themeEditor.saving) return;
  if (isThemeEditorDirty()) {
    themeEditor.pendingAction = action;
    return;
  }
  await finishTransition(action);
}

export async function confirmDiscardThemeEditor() {
  const action = themeEditor.pendingAction;
  if (action) await finishTransition(action, true);
}

export function keepEditingTheme() {
  transitionVersion++;
  themeEditor.pendingAction = null;
}

export function themeNameError(): string | null {
  return validateThemeName(themeEditor.draft.name, appStore.savedThemes, themeEditor.editingId);
}

export async function saveThemeEditor() {
  await withAppearanceLock(saveThemeEditorLocked);
}

async function saveThemeEditorLocked() {
  if (!themeEditor.open || themeEditor.saving) return;
  if (themeEditor.externalChange) {
    themeEditor.error = 'Appearance changed outside this editor. Discard this draft and reopen the editor to use the latest settings.';
    return;
  }
  const nameError = themeNameError();
  if (nameError) {
    themeEditor.error = nameError;
    return;
  }
  if (!themeEditor.editingId && appStore.savedThemes.length >= MAX_SAVED_THEMES) {
    themeEditor.error = `You can keep up to ${MAX_SAVED_THEMES} themes. Delete one first.`;
    return;
  }
  const previousList = appStore.savedThemes;
  const theme: SavedTheme = {
    id: themeEditor.editingId ?? newThemeId(),
    name: themeEditor.draft.name.trim(),
    palette: themeColors(themeEditor.draft.palette),
    accent: themeEditor.draft.accent,
  };
  themeEditor.saving = true;
  themeEditor.error = '';
  try {
    const expected = await settingsBaseline;
    if (!expected || themeEditor.externalChange) throw new Error('Appearance changed. Discard this draft and reopen the editor.');
    const values = { ...expected, custom_themes: upsertSavedTheme(previousList, theme), custom_theme: theme.palette, appearance_mode: 'custom', accent_color: theme.accent };
    await invoke('save_model_settings_if_unchanged', { expected, values });
    appStore.savedThemes = values.custom_themes;
    appStore.appearanceMode = 'custom';
    appStore.customTheme = theme.palette;
    appStore.accentColor = theme.accent;
    appStore.activeThemeId = theme.id;
    broadcastAppearance('custom', theme.palette, theme.accent);
  } catch (err) {
    themeEditor.saving = false;
    themeEditor.error = formatIpcError(err, 'Could not save this theme. Your changes are still here.');
    return;
  }
  close();
}

export async function deleteThemeFromEditor() {
  await withAppearanceLock(deleteLocked);
}

async function deleteLocked() {
  const id = themeEditor.editingId;
  if (!themeEditor.open || !id || themeEditor.saving) return;
  if (themeEditor.externalChange) {
    themeEditor.error = 'Appearance changed outside this editor. Discard this draft and reopen the editor to use the latest settings.';
    return;
  }
  themeEditor.saving = true;
  try {
    const expected = await settingsBaseline;
    if (!expected || themeEditor.externalChange) throw new Error('Appearance changed. Discard this draft and reopen the editor.');
    const next = appStore.savedThemes.filter((t) => t.id !== id);
    await invoke('save_model_settings_if_unchanged', { expected, values: { ...expected, custom_themes: next } });
    appStore.savedThemes = next;
  } catch (err) {
    themeEditor.saving = false;
    themeEditor.confirmingDelete = false;
    themeEditor.error = formatIpcError(err, 'Could not delete this theme. It is still saved.');
    return;
  }
  if (appStore.activeThemeId === id) appStore.activeThemeId = null;
  if (themeEditor.baseline) themeEditor.baseline.themeId = themeEditor.baseline.themeId === id ? null : themeEditor.baseline.themeId;
  cancelThemeEditor();
}
