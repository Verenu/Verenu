import { expect, it, vi } from 'vitest';

vi.mock('./platform', () => ({ isLinux: true, isAndroid: false }));
vi.mock('./tauri', () => ({
  invoke: vi.fn(async () => null),
  listen: vi.fn(async () => () => {}),
}));

it('keeps Settings usable when shortcut status is unavailable', async () => {
  const { desktopShortcut, loadDesktopShortcuts } = await import('./shortcutStatus.svelte');
  await loadDesktopShortcuts();
  expect(desktopShortcut('copy')).toBeUndefined();
});
