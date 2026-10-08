import { invoke, listen } from './tauri';
import { isAndroid, isLinux } from './platform';

export type ShortcutStatus = {
  id: 'dictation' | 'copy' | 'capture' | 'cancel' | 'handsfree';
  requested: string;
  active: string | null;
  codes: string[];
  note: string | null;
};

export const shortcutStatus = $state<{ items: ShortcutStatus[] }>({ items: [] });
let listening: Promise<void> | undefined;
let statusRevision = 0;
let loadSequence = 0;

export function desktopShortcut(id: ShortcutStatus['id']): ShortcutStatus | undefined {
  return shortcutStatus.items.find((item) => item.id === id);
}

export async function loadDesktopShortcuts(): Promise<void> {
  if (!isLinux || isAndroid) return;
  listening ??= listen<ShortcutStatus[]>('verenu:shortcuts-changed', (event) => {
    statusRevision += 1;
    shortcutStatus.items = Array.isArray(event.payload) ? event.payload : [];
  }).then(() => undefined).catch(() => { listening = undefined; });
  await listening;
  const sequence = ++loadSequence;
  const revision = statusRevision;
  try {
    const items = await invoke<ShortcutStatus[]>('get_shortcut_status');
    if (sequence !== loadSequence || revision !== statusRevision) return;
    shortcutStatus.items = Array.isArray(items) ? items : [];
  } catch {
    // A browser preview has no native desktop bindings.
  }
}
