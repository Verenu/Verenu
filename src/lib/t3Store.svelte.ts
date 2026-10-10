import { invoke } from './tauri';
import { formatIpcError } from './errors';
import { startPolling } from './polling';

export type T3Skill = { name: string; displayName?: string; description?: string };
export type T3Catalog = { id: string; label: string; providerInstanceId: string; workspaceId: string; revision: string; skills: T3Skill[] };
export type T3Connection = {
  id: string; label: string; version: string; expiresAt: number; fetchedAt: number;
  selectedCatalog: string | null; catalogs: T3Catalog[]; error: string | null;
};
export type T3Status = { minimumVersion: string; connection: T3Connection | null; skills: T3Skill[] };
export const t3State = $state({
  status: null as T3Status | null,
  // Any user-visible operation in flight, including the daily/manual pulls.
  loading: false,
  // A pairing link is being exchanged. Set only by connectT3, never by background polls.
  connecting: false,
  error: '',
  notice: '',
});

// Requests are numbered when issued. Pairing, selection, and disconnect results
// take precedence over reads. Reads must start outside a mutation and remain
// within the same mutation epoch, so an in-flight poll cannot restore old data.
let sequence = 0;
let appliedSequence = 0;
let pending = 0;
let mutationEpoch = 0;
let pendingMutations = 0;
let latestMutation = 0;
const MUTATIONS = new Set(['connect_t3', 'select_t3_catalog', 'disconnect_t3']);

export async function updateT3(command = 'get_t3_skills', args?: Record<string, unknown>, options: { background?: boolean } = {}): Promise<boolean> {
  const request = ++sequence;
  const mutation = MUTATIONS.has(command);
  if (mutation) {
    mutationEpoch++;
    pendingMutations++;
    latestMutation = request;
  }
  const epoch = mutationEpoch;
  const readDuringMutation = !mutation && pendingMutations > 0;
  const canApply = () => mutation
    ? request === latestMutation
    : !readDuringMutation && pendingMutations === 0 && epoch === mutationEpoch && request > appliedSequence;
  if (!options.background) {
    pending++;
    t3State.loading = true;
    t3State.error = '';
    // connectT3 owns the pairing notice; other actions replace it.
    if (command !== 'connect_t3') t3State.notice = '';
  }
  try {
    const result = await invoke<T3Status>(command, args);
    if (canApply()) {
      t3State.status = result;
      appliedSequence = Math.max(appliedSequence, request);
      t3State.error = '';
    }
    return true;
  } catch (error) {
    if (canApply()) {
      t3State.error = formatIpcError(error, 'Could not update T3 skills.');
      appliedSequence = Math.max(appliedSequence, request);
    }
    return false;
  } finally {
    if (mutation) pendingMutations--;
    if (!options.background) {
      pending--;
      t3State.loading = pending > 0;
    }
  }
}

export async function connectT3(pairingLink: string): Promise<boolean> {
  t3State.connecting = true;
  t3State.notice = 'Connecting to T3 Code…';
  try {
    const ok = await updateT3('connect_t3', { pairingLink });
    const label = t3State.status?.connection?.label;
    t3State.notice = ok && label
      ? `Connected to ${label}.${t3State.status?.skills.length ? ' Skills are ready to use in T3 Code.' : ' No skills were reported yet.'}`
      : ok ? 'Connected to T3 Code.' : '';
    return ok;
  } finally {
    t3State.connecting = false;
  }
}

export function startT3Refresh(): () => void {
  // The backend enforces a daily network pull and bounded failure retries.
  return startPolling(() => updateT3('pull_t3_skills', { force: false }, { background: true }), 60_000, { hiddenIntervalMs: 15 * 60_000 }).stop;
}
