import { invoke } from './tauri';
import { saveSetting } from './settings';
import { formatIpcError } from './errors';
import { startPolling } from './polling';
import type { GithubSnapshot } from './views/insights/github';

export const githubState = $state({
  username: '', snapshot: null as GithubSnapshot | null, loading: false, ready: false, error: '',
});
let generation = 0;
let inFlight: Promise<void> | null = null;

export function refreshGithub(refresh = false): Promise<void> {
  if (inFlight) return inFlight;
  const token = generation;
  inFlight = (async () => {
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
  })().finally(() => { inFlight = null; });
  return inFlight;
}

export async function setGithubUsername(username: string): Promise<void> {
  if (username && username === githubState.username) {
    await refreshGithub();
    return;
  }
  await saveSetting('github_username', username);
  generation++;
  githubState.username = username;
  githubState.snapshot = null;
  githubState.error = '';
  githubState.loading = false;
  // Let the previous request finish without restoring the old account.
  if (inFlight) await inFlight;
  if (username) await refreshGithub();
}

export function startGithubRefresh(): () => void {
  // The native cache limits network refreshes to every 15 minutes. Reconcile
  // settings and date rollover while visible, and refresh when the app returns.
  const poll = startPolling(() => refreshGithub(), 60_000, { hiddenIntervalMs: 15 * 60_000 });
  return poll.stop;
}
