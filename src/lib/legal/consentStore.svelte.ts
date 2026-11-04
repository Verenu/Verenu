import { formatIpcError } from '../errors';
import * as api from './consent';
import { isConsentBlocked, isConsentRequiredError, isVersionConflictError, NOT_ACCEPTED_NOTICE, PRIVACY_VERSION, TERMS_VERSION, VERSION_CONFLICT_NOTICE } from './consent';
import type { ConsentState } from './consent';

/** Shared reactive consent state for the gate, Settings, and gated-feature hints. */
export const consentStore = $state<{
  state: ConsentState | null;
  loaded: boolean;
  busy: boolean;
  error: string;
  /** Explains a rejected acceptance. Cleared on the next attempt. */
  notice: string;
  /** True once a pending deletion has left `deletion_pending`, meaning the server confirmed it. */
  deletionConfirmed: boolean;
}>({ state: null, loaded: false, busy: false, error: '', notice: '', deletionConfirmed: false });

/** The document versions to show and send: the backend's current ones, else the built-in pair. */
export function currentVersions(): { termsVersion: string; privacyVersion: string } {
  return {
    termsVersion: consentStore.state?.termsVersion ?? TERMS_VERSION,
    privacyVersion: consentStore.state?.privacyVersion ?? PRIVACY_VERSION,
  };
}

export async function loadConsent(): Promise<void> {
  try {
    const next = await api.getConsentState();
    if (consentStore.state?.status === 'deletion_pending' && next.status !== 'deletion_pending') {
      consentStore.deletionConfirmed = true;
    }
    consentStore.state = next;
    consentStore.error = '';
  } catch (err) {
    consentStore.error = formatIpcError(err);
  } finally {
    consentStore.loaded = true;
  }
}

async function run(action: () => Promise<ConsentState>, expect?: ConsentState['status']): Promise<boolean> {
  if (consentStore.busy) return false;
  consentStore.busy = true;
  consentStore.error = '';
  consentStore.notice = '';
  try {
    const next = await action();
    // Only the backend's answer moves the state. A reply that is not the expected status
    // (for example update_required) is a refusal, so nothing unlocks.
    consentStore.state = next;
    if (expect && next.status !== expect) {
      consentStore.notice = next.status === 'update_required' ? VERSION_CONFLICT_NOTICE : NOT_ACCEPTED_NOTICE;
      return false;
    }
    return true;
  } catch (err) {
    if (isVersionConflictError(err)) {
      consentStore.notice = VERSION_CONFLICT_NOTICE;
      await loadConsent();
    } else {
      consentStore.error = formatIpcError(err);
    }
    return false;
  } finally {
    consentStore.busy = false;
  }
}

export const acceptConsent = () => run(() => api.accept(currentVersions()), 'accepted');
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
  consentStore.notice = '';
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
  consentStore.notice = '';
  consentStore.deletionConfirmed = false;
}

export function resetConsentStoreForTests() {
  consentFlow.open = false;
  consentFlow.pending = null;
  consentStore.state = null;
  consentStore.loaded = false;
  consentStore.busy = false;
  consentStore.error = '';
  consentStore.notice = '';
  consentStore.deletionConfirmed = false;
}
