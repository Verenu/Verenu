import { beforeEach, describe, expect, it, vi } from 'vitest';

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('./tauri', () => ipc);

import { connectT3, t3State, updateT3 } from './t3Store.svelte';

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void; reject: (error: unknown) => void };
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const status = (label: string | null) => ({
  minimumVersion: '0.46',
  skills: [],
  connection: label ? { id: 'c', label, version: '0.46.0', expiresAt: 4102444800, fetchedAt: 1, selectedCatalog: null, catalogs: [], error: null } : null,
});

describe('T3 store request ordering', () => {
  beforeEach(() => {
    ipc.invoke.mockReset();
    t3State.status = null;
    t3State.loading = false;
    t3State.connecting = false;
    t3State.error = '';
    t3State.notice = '';
  });

  it('keeps a successful connect when an older background read finishes later', async () => {
    const read = deferred<ReturnType<typeof status>>();
    const connect = deferred<ReturnType<typeof status>>();
    ipc.invoke.mockReturnValueOnce(read.promise).mockReturnValueOnce(connect.promise);
    const pending = updateT3('pull_t3_skills', { force: false }, { background: true });
    const paired = connectT3('https://synthetic.invalid/pair#token=synthetic');
    connect.resolve(status('Synthetic workstation'));
    await expect(paired).resolves.toBe(true);
    read.resolve(status(null));
    await pending;
    expect(t3State.status?.connection?.label).toBe('Synthetic workstation');
    expect(t3State.notice).toContain('Synthetic workstation');
  });

  it('applies a connect result even when a later poll finishes first', async () => {
    const connect = deferred<ReturnType<typeof status>>();
    const poll = deferred<ReturnType<typeof status>>();
    ipc.invoke.mockReturnValueOnce(connect.promise).mockReturnValueOnce(poll.promise);
    const paired = connectT3('https://synthetic.invalid/pair#token=synthetic');
    const pending = updateT3('pull_t3_skills', { force: false }, { background: true });
    poll.resolve(status(null));
    await pending;
    connect.resolve(status('Synthetic workstation'));
    await expect(paired).resolves.toBe(true);
    expect(t3State.status?.connection?.label).toBe('Synthetic workstation');
  });

  it('does not let an older read overwrite a newer applied result', async () => {
    const older = deferred<ReturnType<typeof status>>();
    const newer = deferred<ReturnType<typeof status>>();
    ipc.invoke.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const first = updateT3('get_t3_skills', undefined, { background: true });
    const second = updateT3('get_t3_skills', undefined, { background: true });
    newer.resolve(status('Newer'));
    await second;
    older.resolve(status('Older'));
    await first;
    expect(t3State.status?.connection?.label).toBe('Newer');
  });

  it('ignores a stale poll started during pairing even when it finishes after pairing', async () => {
    const connect = deferred<ReturnType<typeof status>>();
    const poll = deferred<ReturnType<typeof status>>();
    ipc.invoke.mockReturnValueOnce(connect.promise).mockReturnValueOnce(poll.promise);
    const paired = connectT3('https://synthetic.invalid/pair#token=synthetic');
    const pending = updateT3('get_t3_skills', undefined, { background: true });
    connect.resolve(status('New pairing'));
    await paired;
    poll.resolve(status(null));
    await pending;
    expect(t3State.status?.connection?.label).toBe('New pairing');
  });

  it('shows loading only for foreground work and clears it after overlapping calls', async () => {
    const a = deferred<ReturnType<typeof status>>();
    const b = deferred<ReturnType<typeof status>>();
    ipc.invoke.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const background = updateT3('get_t3_skills', undefined, { background: true });
    expect(t3State.loading).toBe(false);
    const foreground = updateT3('pull_t3_skills', { force: true });
    expect(t3State.loading).toBe(true);
    a.resolve(status('Synthetic'));
    await background;
    expect(t3State.loading).toBe(true);
    b.resolve(status('Synthetic'));
    await foreground;
    expect(t3State.loading).toBe(false);
  });

  it('reports a failed pairing as an error and clears the connecting state', async () => {
    ipc.invoke.mockRejectedValueOnce(new Error('synthetic failure'));
    await expect(connectT3('https://synthetic.invalid/pair#token=synthetic')).resolves.toBe(false);
    expect(t3State.connecting).toBe(false);
    expect(t3State.loading).toBe(false);
    expect(t3State.error).toBeTruthy();
    expect(t3State.notice).toBe('');
  });
});
