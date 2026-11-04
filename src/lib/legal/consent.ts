import { invoke } from '../tauri';

/**
 * Typed adapter over the legal consent commands described in
 * docs/legal-consent-api.md. Everything in the UI goes through these five
 * functions, so a contract change touches only this file.
 */
export const TERMS_VERSION = '2026-11-03';
export const PRIVACY_VERSION = '2026-11-03';
export const TERMS_URL = `https://verenu.com/legal/terms/${TERMS_VERSION}`;
export const PRIVACY_URL = `https://verenu.com/legal/privacy/${PRIVACY_VERSION}`;

export type ConsentStatus =
  | 'unanswered'
  | 'skipped'
  | 'accepted'
  | 'update_required'
  | 'deletion_pending';

export interface ConsentState {
  status: ConsentStatus;
  termsVersion: string;
  privacyVersion: string;
  /** Unix milliseconds. */
  acceptedAt: number | null;
  syncPending: boolean;
}

export const getConsentState = () => invoke<ConsentState>('get_consent_state');
export const accept = () => invoke<ConsentState>('accept_legal');
export const skip = () => invoke<ConsentState>('skip_legal');
export const deleteMyData = () => invoke<ConsentState>('delete_my_data');
export const getInstallId = () => invoke<string>('get_install_id');

/** Analytics and online features run only after the current terms are accepted. */
export function isConsentGranted(state: ConsentState | null): boolean {
  return state?.status === 'accepted';
}

/** True when the user must answer before the app is usable. */
export function needsConsentPrompt(state: ConsentState | null): boolean {
  return state?.status === 'unanswered' || state?.status === 'update_required';
}

export const CONSENT_STATUS_LABEL: Record<ConsentStatus, string> = {
  unanswered: 'Not answered',
  skipped: 'Skipped',
  accepted: 'Accepted',
  update_required: 'New terms to review',
  deletion_pending: 'Deletion pending',
};

/**
 * True when online features must stay off. An unknown state (backend not
 * answering) is not treated as blocked, so the UI never hides features the
 * backend would allow. The backend still enforces consent on every call.
 */
export function isConsentBlocked(state: ConsentState | null): boolean {
  return state !== null && state.status !== 'accepted';
}

/** The backend's `consent_required` error, including its current plain-text message. */
export function isConsentRequiredError(err: unknown): boolean {
  const text = typeof err === 'string' ? err
    : err instanceof Error ? err.message
    : typeof err === 'object' && err !== null ? JSON.stringify(err) : '';
  return /consent_required|accept the (current )?terms and privacy policy/i.test(text);
}
