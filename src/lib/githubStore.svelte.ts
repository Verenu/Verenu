import { invoke } from './tauri';
import { saveSetting } from './settings';
import { formatIpcError } from './errors';
import { startPolling } from './polling';
import type { GithubSnapshot } from './views/insights/github';

export const githubState = $state({
  username: '', snapshot: null as GithubSnapshot | null, loading: false, ready: false, error: '',
});
let generation = 0;
let inFlight: { generation: number; promise: Promise<void> } | null = null;
let pendingSaveGeneration: number | null = null;

export function refreshGithub(refresh = false): Promise<void> {
  if (pendingSaveGeneration !== null) return Promise.resolve();
  const token = generation;
  if (inFlight?.generation === token) return inFlight.promise;
  const request = { generation: token, promise: Promise.resolve() };
  request.promise = (async () => {
    try {
      const username = await invoke<string | null>('get_setting', { key: 'github_username' }) ?? '';
      if (token !== generation) return;
      if (username !== githubState.username) githubState.snapshot = null;
      githubState.username = username;
      githubState.ready = true;
      if (!username) { githubState.snapshot = null; githubState.error = ''; return; }
      githubState.loading = true;
      const snapshot = await invoke<GithubSnapshot | null>('get_github_commits', { refresh });
      const current = await invoke<string | null>('get_setting', { key: 'github_username' }) ?? '';
      if (token !== generation || current !== username) return;
      githubState.snapshot = snapshot;
      githubState.error = '';
    } catch (error) {
      if (token === generation) githubState.error = formatIpcError(error, 'Could not refresh GitHub commits.');
    } finally {
      if (token === generation) { githubState.loading = false; githubState.ready = true; }
    }
  })().finally(() => { if (inFlight === request) inFlight = null; });
  inFlight = request;
  return request.promise;
}

export async function setGithubUsername(username: string): Promise<void> {
  const isSameAccount = username && username.toLowerCase() === githubState.username.toLowerCase();
  if (pendingSaveGeneration === null && isSameAccount) return;

  // Detach previous-account requests immediately. A slow native fetch must not
  // hold up disconnect or a new account's first refresh.
  const saveGeneration = ++generation;
  pendingSaveGeneration = saveGeneration;
  githubState.loading = false;
  try {
    await saveSetting('github_username', username);
  } catch (error) {
    if (generation === saveGeneration && pendingSaveGeneration === saveGeneration) {
      pendingSaveGeneration = null;
      generation++;
      githubState.loading = false;
      void refreshGithub();
    }
    throw error;
  }
  if (generation !== saveGeneration) return;

  // Polling is held during persistence so it cannot consume the new account's
  // first native refresh attempt.
  pendingSaveGeneration = null;
  generation++;
  githubState.username = username;
  githubState.snapshot = null;
  githubState.error = '';
  githubState.loading = false;
  if (username) await refreshGithub();
}

export function startGithubRefresh(): () => void {
  // The native cache limits network refreshes to every 15 minutes. Reconcile
  // settings and date rollover while visible, and refresh when the app returns.
  const poll = startPolling(() => refreshGithub(), 60_000, { hiddenIntervalMs: 15 * 60_000 });
  return poll.stop;
}
