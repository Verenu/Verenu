import { expect, it, vi } from 'vitest';
import { invoke, listen } from './tauri';
import { desktopShortcut, loadDesktopShortcuts } from './shortcutStatus.svelte';

const harness = vi.hoisted(() => ({ handlers: new Map<string, unknown>() }));

vi.mock('./platform', () => ({ isLinuxDesktop: true }));
vi.mock('./tauri', () => ({
  invoke: vi.fn(async () => null),
  listen: vi.fn(async (event: string, handler: unknown) => {
    harness.handlers.set(event, handler);
    return () => {};
  }),
}));

it('keeps Settings usable when shortcut status is unavailable', async () => {
  await loadDesktopShortcuts();
  expect(desktopShortcut('copy')).toBeUndefined();
});

it('loads persistent shortcut failures and clears them after native recovery', async () => {
  const failure = {
    id: 'dictation', requested: 'Ctrl + Super', active: null, codes: [],
    note: 'Creating shortcut session: An app id is required',
  };
  vi.mocked(invoke).mockResolvedValueOnce([failure]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')).toEqual(failure);
  vi.mocked(invoke).mockResolvedValueOnce([failure]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')?.note).toContain('An app id is required');
  const recovered = { ...failure, active: 'Ctrl + Super', codes: ['ControlLeft', 'MetaLeft'], note: null };
  vi.mocked(invoke).mockResolvedValueOnce([recovered]);
  await loadDesktopShortcuts();
  expect(desktopShortcut('dictation')).toEqual(recovered);
});

it('keeps a newer shortcut event when the status snapshot returns late', async () => {
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

it('keeps an event received while the shortcut listener is being registered', async () => {
  vi.resetModules();
  let finishListening!: () => void;
  let onStatus!: (event: { payload: unknown[] }) => void;
  vi.mocked(listen).mockImplementationOnce((_event, handler) => {
    onStatus = handler as typeof onStatus;
    return new Promise((resolve) => { finishListening = () => resolve(() => {}); });
  });
  vi.mocked(invoke).mockResolvedValueOnce([]);
  const { loadDesktopShortcuts: load, desktopShortcut: shortcut } = await import('./shortcutStatus.svelte');
  const failure = { id: 'dictation', requested: 'Ctrl + Super', active: null, codes: [], note: 'unavailable' };
  const loading = load();
  onStatus({ payload: [failure] });
  finishListening();
  await loading;
  expect(shortcut('dictation')).toEqual(failure);
});
