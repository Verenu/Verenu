// State and actions for the theme editor dock. The dock lives at the app root
// so it survives navigation; this module owns the draft, the live preview, and
// the rules for leaving it (cancel restores exactly what was applied before).
import { appStore } from './stores';
import {
  applyPalette,
  withAppearanceLock,
  persistSavedThemes,
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
  type CustomTheme,
  type SavedTheme,
} from './customTheme';
import { formatIpcError } from './errors';

type Draft = { name: string; palette: CustomTheme; accent: string | null };

export const themeEditor = $state({
  open: false,
  /** Bumped on every open so the dock rebuilds its text fields. */
  session: 0,
  editingId: null as string | null,
  draft: { name: '', palette: { background: '#FFFFFF', foreground: '#000000', sidebar: null, surface: null }, accent: null } as Draft,
  original: null as Draft | null,
  baseline: null as AppearanceSnapshot | null,
  saving: false,
  error: '',
  confirmingDelete: false,
  /** Set while the user is asked whether to throw away unsaved changes. */
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

/** Paints the draft across the app without saving anything. */
function previewDraft() {
  appStore.appearanceMode = 'custom';
  appStore.customTheme = themeColors(themeEditor.draft.palette);
  appStore.accentColor = themeEditor.draft.accent;
}

function restoreBaseline() {
  const baseline = themeEditor.baseline;
  if (!baseline) return;
  appStore.appearanceMode = baseline.mode;
  appStore.customTheme = baseline.customTheme;
  appStore.accentColor = baseline.accent;
  appStore.activeThemeId = baseline.themeId;
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
}

/** Opens the editor on a saved theme, or starts a new one from `seed`. */
export function openThemeEditor(source: { theme: SavedTheme } | { seed: CustomTheme; name: string; accent?: string | null }) {
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
  themeEditor.confirmingDelete = false;
  themeEditor.pendingAction = null;
  themeEditor.saving = false;
  themeEditor.session += 1;
  themeEditor.open = true;
  // Editing an existing theme previews it right away; a new one starts from
  // its seed, which the user has usually already seen.
  previewDraft();
}

export function updateThemeDraft(patch: Partial<Draft>) {
  if (!themeEditor.open || themeEditor.saving) return;
  themeEditor.draft = {
    ...themeEditor.draft,
    ...patch,
    palette: patch.palette ? themeColors(patch.palette) : themeEditor.draft.palette,
  };
  themeEditor.error = '';
  previewDraft();
}

/** Closes without saving and puts back what was applied before editing. */
export function cancelThemeEditor() {
  if (!themeEditor.open) return;
  restoreBaseline();
  close();
}

/** Escape / Close: clean drafts close at once, dirty ones ask first. */
export function requestCloseThemeEditor() {
  if (!themeEditor.open || themeEditor.saving) return;
  if (isThemeEditorDirty()) themeEditor.pendingAction = () => {};
  else cancelThemeEditor();
}

/**
 * Runs `action` once it is safe to leave the editor. With unsaved changes the
 * dock asks first; the action runs only if the user discards.
 */
export function guardThemeEditor(action: () => void) {
  if (!themeEditor.open) return action();
  if (themeEditor.saving) return;
  if (isThemeEditorDirty()) {
    themeEditor.pendingAction = action;
    return;
  }
  cancelThemeEditor();
  action();
}

export function confirmDiscardThemeEditor() {
  const action = themeEditor.pendingAction;
  cancelThemeEditor();
  action?.();
}

export function keepEditingTheme() {
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
  const nameError = themeNameError();
  if (nameError) {
    themeEditor.error = nameError;
    return;
  }
  if (!themeEditor.editingId && appStore.savedThemes.length >= MAX_SAVED_THEMES) {
    themeEditor.error = `You can keep up to ${MAX_SAVED_THEMES} themes. Delete one first.`;
    return;
  }
  const baseline = themeEditor.baseline ?? snapshotAppearance();
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
    await persistSavedThemes(upsertSavedTheme(previousList, theme));
  } catch (err) {
    themeEditor.saving = false;
    themeEditor.error = formatIpcError(err, 'Could not save this theme. Your changes are still here.');
    return;
  }
  const applied = await applyPalette(theme.palette, { accent: theme.accent, themeId: theme.id, from: baseline });
  if (!applied) {
    try {
      await persistSavedThemes(previousList);
    } catch (err) {
      console.error('restore custom_themes after apply failure failed:', err);
      appStore.savedThemes = previousList;
    }
    themeEditor.saving = false;
    themeEditor.error = 'The theme was saved but could not be applied, so nothing changed. Try again.';
    previewDraft();
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
  themeEditor.saving = true;
  try {
    await persistSavedThemes(appStore.savedThemes.filter((t) => t.id !== id));
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
