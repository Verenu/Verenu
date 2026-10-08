import { expect, it, vi } from 'vitest';
import { invoke, listen } from './tauri';

const harness = vi.hoisted(() => ({ handlers: new Map<string, unknown>() }));

vi.mock('./platform', () => ({ isLinux: true, isAndroid: false }));
vi.mock('./tauri', () => ({
  invoke: vi.fn(async () => null),
  listen: vi.fn(async (event: string, handler: unknown) => {
    harness.handlers.set(event, handler);
    return () => {};
  }),
}));

it('keeps Settings usable when shortcut status is unavailable', async () => {
  const { desktopShortcut, loadDesktopShortcuts } = await import('./shortcutStatus.svelte');
  await loadDesktopShortcuts();
  expect(desktopShortcut('copy')).toBeUndefined();
});

it('loads persistent shortcut failures and clears them after native recovery', async () => {
  const { desktopShortcut, loadDesktopShortcuts } = await import('./shortcutStatus.svelte');
  const failure = {
    id: 'dictation', requested: 'Ctrl + Super', active: null, codes: [],
    note: 'Creating shortcut session: An app id is required',
  };
  vi.mocked(invoke).mockResolvedValueOnce([failure]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')).toEqual(failure);
  // Opening Settings or reloading must retain the backend's failure reason.
  vi.mocked(invoke).mockResolvedValueOnce([failure]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')?.note).toContain('An app id is required');
  const recovered = { ...failure, active: 'Ctrl + Super', codes: ['ControlLeft', 'MetaLeft'], note: null };
  vi.mocked(invoke).mockResolvedValueOnce([recovered]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')).toEqual(recovered);
});

it('keeps a newer shortcut event when the status snapshot returns late', async () => {
  const { desktopShortcut, loadDesktopShortcuts } = await import('./shortcutStatus.svelte');
  const failure = {
    id: 'dictation' as const, requested: 'Ctrl + Super', active: null, codes: [],
    note: 'Creating shortcut session: An app id is required',
  };
  const staleSnapshot = { ...failure, active: 'Ctrl + Super', codes: ['ControlLeft', 'MetaLeft'], note: null };
  let resolveSnapshot!: (items: typeof staleSnapshot[]) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise((resolve) => { resolveSnapshot = resolve; }));

  const loading = loadDesktopShortcuts();
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_shortcut_status'));
  const onShortcutStatus = harness.handlers.get('verenu:shortcuts-changed') as
    ((event: { payload: typeof failure[] }) => void) | undefined;
  expect(onShortcutStatus).toBeDefined();
  onShortcutStatus?.({ payload: [failure] });
  resolveSnapshot([staleSnapshot]);
  await loading;

  expect(desktopShortcut('dictation')).toEqual(failure);
});
