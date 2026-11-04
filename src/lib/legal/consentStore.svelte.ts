import { formatIpcError } from '../errors';
import * as api from './consent';
import { isConsentBlocked, isConsentRequiredError } from './consent';
import type { ConsentState } from './consent';

/** Shared reactive consent state for the gate, Settings, and gated-feature hints. */
export const consentStore = $state<{
  state: ConsentState | null;
  loaded: boolean;
  busy: boolean;
  error: string;
}>({ state: null, loaded: false, busy: false, error: '' });

export async function loadConsent(): Promise<void> {
  try {
    consentStore.state = await api.getConsentState();
    consentStore.error = '';
  } catch (err) {
    consentStore.error = formatIpcError(err);
  } finally {
    consentStore.loaded = true;
  }
}

async function run(action: () => Promise<ConsentState>): Promise<boolean> {
  if (consentStore.busy) return false;
  consentStore.busy = true;
  consentStore.error = '';
  try {
    consentStore.state = await action();
    return true;
  } catch (err) {
    consentStore.error = formatIpcError(err);
    return false;
  } finally {
    consentStore.busy = false;
  }
}

export const acceptConsent = () => run(api.accept);
export const skipConsent = () => run(api.skip);
export const requestDataDeletion = () => run(api.deleteMyData);

/** The accept dialog. `pending` runs after a successful Accept, to enable what the user tapped. */
export const consentFlow = $state<{ open: boolean; pending: (() => void | Promise<void>) | null }>({
  open: false,
  pending: null,
});

/** True while skipped, unanswered, outdated, or deleting. Gated controls read this. */
export function consentBlocked(): boolean {
  return consentStore.loaded && isConsentBlocked(consentStore.state);
}

/**
 * Run `thenEnable` now if the terms are accepted. Otherwise open the accept
 * dialog and run it only after the user accepts. Dismissing leaves it off.
 * Returns true when `thenEnable` ran immediately.
 */
export function requireConsent(thenEnable: () => void | Promise<void>): boolean {
  if (!consentBlocked()) {
    void thenEnable();
    return true;
  }
  consentStore.error = '';
  consentFlow.pending = thenEnable;
  consentFlow.open = true;
  return false;
}

/**
 * Call from a catch block. If `err` is the backend's `consent_required`, refresh
 * the state, open the accept dialog (retrying `retry` on Accept), and return true.
 */
export function handleConsentError(err: unknown, retry?: () => void | Promise<void>): boolean {
  if (!isConsentRequiredError(err)) return false;
  void loadConsent();
  consentStore.error = '';
  consentFlow.pending = retry ?? null;
  consentFlow.open = true;
  return true;
}

export async function acceptFromFlow(): Promise<void> {
  if (!(await acceptConsent())) return;
  const pending = consentFlow.pending;
  consentFlow.open = false;
  consentFlow.pending = null;
  await pending?.();
}

export function dismissConsentFlow() {
  consentFlow.open = false;
  consentFlow.pending = null;
  consentStore.error = '';
}

export function resetConsentStoreForTests() {
  consentFlow.open = false;
  consentFlow.pending = null;
  consentStore.state = null;
  consentStore.loaded = false;
  consentStore.busy = false;
  consentStore.error = '';
}
