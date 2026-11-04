import { formatIpcError } from '../errors';
import * as api from './consent';
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

export function resetConsentStoreForTests() {
  consentStore.state = null;
  consentStore.loaded = false;
  consentStore.busy = false;
  consentStore.error = '';
}
