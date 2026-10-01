// Sub-apps: a place inside an app (a Discord server, a VS Code project)
// matched by a window-title rule. Matching mirrors the Rust resolver in
// src-tauri/src/data/db/sub_apps.rs so the sheet's live check agrees with
// what dictation will do.
import { isMac } from './platform';
import type { TitleMatchMode } from './stores';

export const SUB_APP_CAPTURED_EVENT = 'verenu:sub-app-captured';
export const SUB_APP_CAPTURE_FAILED_EVENT = 'verenu:sub-app-capture-failed';
export const SUB_APP_LABEL_LIMIT = 60;
export const SUB_APP_PATTERN_LIMIT = 200;

/** Platform default; mirrors `Chord::default_for_platform` in Rust. */
export const DEFAULT_SUB_APP_CAPTURE_HOTKEY = isMac ? 'Alt+Shift+Super+S' : 'Ctrl+Alt+Shift+S';

const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Super'] as const;
type Modifier = (typeof MODIFIER_ORDER)[number];

function modifierLabel(modifier: Modifier): string {
  if (isMac) return { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘' }[modifier];
  return modifier === 'Super' ? 'Super' : modifier;
}

/** Display keys for a stored chord, e.g. ["Ctrl", "Alt", "Shift", "S"]. */
export function subAppCaptureKeys(chord: string | null | undefined): string[] {
  const parts = (chord || DEFAULT_SUB_APP_CAPTURE_HOTKEY).split('+').map((part) => part.trim());
  const key = parts.find((part) => !MODIFIER_ORDER.includes(part as Modifier)) ?? 'S';
  const modifiers = MODIFIER_ORDER.filter((modifier) => parts.includes(modifier));
  return [...modifiers.map(modifierLabel), key];
}

/**
 * Canonical chord from a key press, or null while only modifiers are held or
 * the chord would hijack typing (needs Ctrl, Alt, or Super). Mirrors
 * `Chord::parse` in src-tauri/src/core/hotkey/chord.rs.
 */
export function chordFromKeyboardEvent(event: KeyboardEvent): string | null {
  let key: string | null = null;
  const letter = /^Key([A-Z])$/.exec(event.code);
  const digit = /^Digit([0-9])$/.exec(event.code);
  const fn = /^F([1-9]|1[0-2])$/.exec(event.code);
  if (letter) key = letter[1];
  else if (digit) key = digit[1];
  else if (fn) key = `F${fn[1]}`;
  if (!key) return null;
  if (!event.ctrlKey && !event.altKey && !event.metaKey) return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Super');
  parts.push(key);
  return parts.join('+');
}

/** What the capture hotkey saw. `window_title` is shown once and never stored. */
export type SubAppCapture = {
  executable: string;
  app_name: string;
  window_title: string;
  proposed_pattern: string;
};

export const MATCH_MODES: { id: TitleMatchMode; label: string }[] = [
  { id: 'contains', label: 'Contains' },
  { id: 'starts_with', label: 'Starts with' },
  { id: 'equals', label: 'Is exactly' },
];

export function normalizeTitle(value: string): string {
  return value.split(/\s+/).filter(Boolean).join(' ').toLowerCase();
}

export function titleRuleMatches(pattern: string, mode: TitleMatchMode, title: string): boolean {
  const p = normalizeTitle(pattern);
  if (!p) return false;
  const t = normalizeTitle(title);
  if (mode === 'equals') return t === p;
  if (mode === 'starts_with') return t.startsWith(p);
  return t.includes(p);
}

/** A readable default name: the first meaningful segment of the pattern. */
export function suggestLabel(pattern: string): string {
  const first = pattern.split(/\s[-—–|]\s/)[0]?.trim() ?? '';
  return (first || pattern.trim()).slice(0, SUB_APP_LABEL_LIMIT);
}
