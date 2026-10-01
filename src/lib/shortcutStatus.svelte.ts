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

export function desktopShortcut(id: ShortcutStatus['id']): ShortcutStatus | undefined {
  return shortcutStatus.items.find((item) => item.id === id);
}

export async function loadDesktopShortcuts(): Promise<void> {
  if (!isLinux || isAndroid) return;
  listening ??= listen<ShortcutStatus[]>('verenu:shortcuts-changed', (event) => {
    shortcutStatus.items = event.payload;
  }).then(() => undefined).catch(() => { listening = undefined; });
  await listening;
  try {
    shortcutStatus.items = await invoke<ShortcutStatus[]>('get_shortcut_status');
  } catch {
    // A browser preview has no native desktop bindings.
  }
}
