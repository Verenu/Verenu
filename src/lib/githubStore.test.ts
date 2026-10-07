import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { githubState, refreshGithub, setGithubUsername, startGithubRefresh } from './githubStore.svelte';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn() }));
vi.mock('./tauri', () => ({ invoke: mocks.invoke }));
vi.mock('./settings', () => ({ saveSetting: mocks.save }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe('GitHub background refresh', () => {
  let username: string;
  const snapshot = { username: 'fixture-user', fetched_at: 1, daily: [], warning: null };
  beforeEach(() => {
    username = '';
    mocks.invoke.mockReset();
    mocks.save.mockImplementation(async (_key, value) => { username = value; });
    mocks.invoke.mockImplementation(async command => command === 'get_setting' ? username : snapshot);
    Object.assign(githubState, { username: '', snapshot: null, loading: false, ready: false, error: '' });
  });
  afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

  it('does not fetch commits before the user connects', async () => {
    await refreshGithub();
    expect(mocks.invoke).not.toHaveBeenCalledWith('get_github_commits', expect.anything());
    expect(githubState.snapshot).toBeNull();
  });

  it('saving the same account preserves its native cache', async () => {
    username = 'fixture-user';
    await refreshGithub();
    await setGithubUsername('fixture-user');
    expect(mocks.save).not.toHaveBeenCalled();
    expect(githubState.snapshot).toEqual(snapshot);
  });

  it('refreshes automatically across views and on returning to the app', async () => {
    vi.useFakeTimers();
    const doc = Object.assign(new EventTarget(), { hidden: false });
    vi.stubGlobal('document', doc);
    username = 'fixture-user';
    const stop = startGithubRefresh();
    await vi.advanceTimersByTimeAsync(0);
    expect(githubState.snapshot).toEqual(snapshot);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'get_github_commits')).toHaveLength(2);
    doc.hidden = true;
    doc.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(15 * 60_000);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'get_github_commits')).toHaveLength(3);
    doc.hidden = false;
    doc.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'get_github_commits')).toHaveLength(4);
    stop();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('disconnect does not restore a late response from the old account', async () => {
    username = 'fixture-user';
    let resolve!: (value: unknown) => void;
    const pending = new Promise(done => { resolve = done; });
    mocks.invoke.mockImplementation(async command => command === 'get_setting' ? username : pending);
    const refresh = refreshGithub();
    await Promise.resolve();
    const disconnect = setGithubUsername('');
    await Promise.resolve();
    resolve(snapshot);
    await Promise.all([refresh, disconnect]);
    expect(githubState.username).toBe('');
    expect(githubState.snapshot).toBeNull();
  });

  it('disconnect and account switch do not wait for or join an obsolete request', async () => {
    username = 'fixture-user';
    const oldResponse = deferred<unknown>();
    const newSnapshot = { ...snapshot, username: 'new-account', fetched_at: 2 };
    const newResponse = deferred<unknown>();
    const fetchedOwners: string[] = [];
    mocks.invoke.mockImplementation(async command => {
      if (command === 'get_setting') return username;
      fetchedOwners.push(username);
      return username === 'fixture-user' ? oldResponse.promise : newResponse.promise;
    });

    const oldRefresh = refreshGithub();
    await vi.waitFor(() => expect(fetchedOwners).toEqual(['fixture-user']));

    let disconnectFinished = false;
    const disconnect = setGithubUsername('').then(() => { disconnectFinished = true; });
    const completedBeforeOldResponse = await Promise.race([
      disconnect.then(() => true),
      new Promise<boolean>(resolve => setTimeout(() => resolve(false), 1_000)),
    ]);
    if (!completedBeforeOldResponse) {
      oldResponse.resolve(snapshot);
      await Promise.all([disconnect, oldRefresh]);
    }
    expect(completedBeforeOldResponse).toBe(true);

    const connect = setGithubUsername('new-account');
    await vi.waitFor(() => expect(fetchedOwners).toEqual(['fixture-user', 'new-account']));

    oldResponse.resolve(snapshot);
    await oldRefresh;
    expect(githubState.username).toBe('new-account');
    expect(githubState.snapshot).toBeNull();

    const coalescedRefresh = refreshGithub();
    expect(fetchedOwners).toEqual(['fixture-user', 'new-account']);
    newResponse.resolve(newSnapshot);
    await Promise.all([disconnect, connect, coalescedRefresh]);

    expect(githubState.username).toBe('new-account');
    expect(githubState.snapshot).toEqual(newSnapshot);
  });
});
