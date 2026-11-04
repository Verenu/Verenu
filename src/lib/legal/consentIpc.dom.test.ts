// @vitest-environment happy-dom
// Runs the UI through the real `invoke` in src/lib/tauri.ts and Tauri's IPC layer. Only the Rust side is
// replaced, by a fake that follows docs/legal-consent-api.md (command names, argument shapes, statuses).
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import ConsentFlowDialog from './ConsentFlowDialog.svelte';
import LegalConsentScreen from './LegalConsentScreen.svelte';
import DeletionPendingScreen from './DeletionPendingScreen.svelte';
import PrivacySection from '../components/settings/PrivacySection.svelte';
import { consentStore, loadConsent, resetConsentStoreForTests } from './consentStore.svelte';
import { PRIVACY_VERSION, TERMS_VERSION, type ConsentState, type ConsentStatus } from './consent';

type Call = { cmd: string; args: Record<string, unknown> | undefined };
let calls: Call[] = [];
let current: ConsentState;
let serverVersions = { terms: TERMS_VERSION, privacy: PRIVACY_VERSION };
let acceptRejects: string | null = null;
let acceptReturns: ConsentStatus | null = null;
let deleteMode: 'pending' | 'error' = 'pending';
const INSTALL_ID = 'a1b2c3d4-install';

function st(status: ConsentStatus, over: Partial<ConsentState> = {}): ConsentState {
  return {
    status, termsVersion: serverVersions.terms, privacyVersion: serverVersions.privacy,
    acceptedAt: status === 'accepted' ? Date.UTC(2026, 10, 3, 12) : null, syncPending: false, ...over,
  };
}

function rust(initial: ConsentStatus) {
  current = st(initial);
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: args as Record<string, unknown> | undefined });
    switch (cmd) {
      // The service reports its current document versions on every read.
      case 'get_consent_state': return { ...current, termsVersion: serverVersions.terms, privacyVersion: serverVersions.privacy };
      case 'get_install_id': return INSTALL_ID;
      case 'skip_legal': return (current = st('skipped'));
      case 'accept_legal': {
        if (acceptRejects) return Promise.reject(acceptRejects);
        const shown = args as { termsVersion?: string; privacyVersion?: string };
        if (shown.termsVersion !== serverVersions.terms || shown.privacyVersion !== serverVersions.privacy) {
          return Promise.reject('consent_version_conflict: HTTP 409');
        }
        return (current = st(acceptReturns ?? 'accepted', { syncPending: true }));
      }
      case 'delete_my_data':
        return deleteMode === 'error' ? Promise.reject('Data deletion is not available yet') : (current = st('deletion_pending'));
      case 'get_all_settings': return { analytics_enabled: true, verenu_service_checks_enabled: true };
      case 'save_setting': return null;
      default: return null;
    }
  });
}

const ran = (cmd: string) => calls.filter((c) => c.cmd === cmd);
const savedAnalytics = () => calls.some((c) => c.cmd === 'save_setting' && c.args?.key === 'analytics_enabled' && c.args?.value === true);

beforeEach(() => {
  Element.prototype.animate = function () {
    return { cancel() {}, effect: null as unknown, set onfinish(fn: () => void) { queueMicrotask(fn); } } as unknown as Animation;
  };
  localStorage.clear();
  calls = []; acceptRejects = null; acceptReturns = null; deleteMode = 'pending';
  serverVersions = { terms: TERMS_VERSION, privacy: PRIVACY_VERSION };
  resetConsentStoreForTests();
});
afterEach(() => { cleanup(); clearMocks(); });

describe('consent commands over IPC', () => {
  it('accept sends the shown Terms 2026-11-03 and Privacy 2026-11-03.1 versions', async () => {
    rust('unanswered');
    render(LegalConsentScreen);
    await loadConsent();
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('accepted'));
    expect(ran('accept_legal')[0].args).toEqual({ termsVersion: '2026-11-03', privacyVersion: '2026-11-03.1' });
    expect(screen.getByRole('link', { name: 'Privacy Policy' }).getAttribute('href')).toBe('https://verenu.com/legal/privacy/2026-11-03.1');
    expect(screen.getByRole('link', { name: 'Terms of Service' }).getAttribute('href')).toBe('https://verenu.com/legal/terms/2026-11-03');
  });

  it('skip calls skip_legal and leaves features off', async () => {
    rust('unanswered');
    render(LegalConsentScreen);
    await loadConsent();
    await fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('skipped'));
    expect(ran('skip_legal')).toHaveLength(1);
    expect(ran('accept_legal')).toHaveLength(0);
  });
});

describe('re-enabling after Skip requires Accept', () => {
  it('does not save the setting until the Terms dialog is accepted', async () => {
    rust('skipped');
    render(ConsentFlowDialog);
    render(PrivacySection);
    await loadConsent();
    const toggle = await screen.findByRole('switch', { name: 'Share pseudonymous product analytics' });
    await waitFor(() => expect(toggle.getAttribute('aria-disabled')).toBe('true'));
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    await fireEvent.click(toggle);
    expect(await screen.findByTestId('consent-flow')).toBeTruthy();
    expect(savedAnalytics()).toBe(false);
    expect(ran('accept_legal')).toHaveLength(0);
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(savedAnalytics()).toBe(true));
    expect(ran('accept_legal')).toHaveLength(1);
  });

  it('keeps the feature off and sends nothing when the dialog is dismissed', async () => {
    rust('skipped');
    render(ConsentFlowDialog);
    render(PrivacySection);
    await loadConsent();
    await fireEvent.click(await screen.findByRole('switch', { name: 'Share pseudonymous product analytics' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    expect(savedAnalytics()).toBe(false);
    expect(ran('accept_legal')).toHaveLength(0);
    expect(consentStore.state?.status).toBe('skipped');
  });
});

describe('HTTP 409 on acceptance (obsolete versions)', () => {
  it('shows a clear message, refetches the current versions, re-prompts, and unlocks nothing', async () => {
    rust('skipped');
    render(ConsentFlowDialog);
    render(PrivacySection);
    await loadConsent();
    // The service moves to newer documents after the dialog was prepared with the old ones.
    serverVersions = { terms: '2026-11-03', privacy: '2026-11-04' };
    await fireEvent.click(await screen.findByRole('switch', { name: 'Share pseudonymous product analytics' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));

    const flow = await screen.findByTestId('consent-flow');
    await waitFor(() => expect(within(flow).getByTestId('consent-notice').textContent).toMatch(/changed while this was open.*Nothing was accepted/));
    await waitFor(() => expect(within(flow).getByRole('link', { name: 'Privacy Policy' }).getAttribute('href')).toBe('https://verenu.com/legal/privacy/2026-11-04'));
    expect(ran('get_consent_state').length).toBeGreaterThanOrEqual(2); // refetched after the 409
    expect(consentStore.state?.privacyVersion).toBe('2026-11-04');
    expect(consentStore.state?.status).toBe('skipped'); // never unlocked locally
    expect(savedAnalytics()).toBe(false);
    expect(screen.getByTestId('consent-flow')).toBeTruthy(); // still asking

    // Accepting the refreshed versions works and only then enables the feature.
    await fireEvent.click(within(flow).getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(savedAnalytics()).toBe(true));
    expect(ran('accept_legal')[ran('accept_legal').length - 1]?.args).toEqual({ termsVersion: '2026-11-03', privacyVersion: '2026-11-04' });
  });

  it('does not unlock when the reply is not an accepted state', async () => {
    rust('skipped');
    acceptReturns = 'update_required';
    render(ConsentFlowDialog);
    render(PrivacySection);
    await loadConsent();
    await fireEvent.click(await screen.findByRole('switch', { name: 'Share pseudonymous product analytics' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    const flow = await screen.findByTestId('consent-flow');
    await waitFor(() => expect(within(flow).getByTestId('consent-notice')).toBeTruthy());
    expect(savedAnalytics()).toBe(false);
    expect(consentStore.state?.status).toBe('update_required');
  });

  it('re-prompts on the first-run screen with the new versions', async () => {
    rust('unanswered');
    render(LegalConsentScreen);
    await loadConsent();
    serverVersions = { terms: '2026-11-03', privacy: '2026-11-04' };
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(screen.getByTestId('consent-notice').textContent).toMatch(/Nothing was accepted/));
    await waitFor(() => expect(screen.getByRole('link', { name: 'Privacy Policy' }).getAttribute('href')).toBe('https://verenu.com/legal/privacy/2026-11-04'));
    expect(consentStore.state?.status).toBe('unanswered');
  });
});

describe('Settings > Privacy over IPC', () => {
  async function openSettings(status: ConsentStatus) {
    rust(status);
    render(ConsentFlowDialog);
    render(PrivacySection);
    await loadConsent();
    await waitFor(() => expect(screen.getByTestId('install-id').textContent).toBe(INSTALL_ID));
  }

  it('copies the install ID from get_install_id', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    await openSettings('accepted');
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(INSTALL_ID));
  });

  it('delete asks for confirmation first, then calls delete_my_data and shows Deletion pending', async () => {
    await openSettings('accepted');
    await fireEvent.click(screen.getByRole('button', { name: 'Delete my data' }));
    expect(ran('delete_my_data')).toHaveLength(0);
    const dialog = screen.getByRole('dialog', { name: 'Delete your data?' });
    // The backend wipes local data and restarts the app, so the dialog must say so before it runs.
    expect(dialog.textContent).toMatch(/history, settings, dictionary, and saved API keys on this device/);
    expect(dialog.textContent).toMatch(/restarts/);
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Delete my data' }));
    await waitFor(() => expect(ran('delete_my_data')).toHaveLength(1));
    expect(await screen.findByTestId('deletion-pending')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Deletion pending' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows the backend error and keeps the dialog open when deletion fails', async () => {
    await openSettings('accepted');
    deleteMode = 'error';
    await fireEvent.click(screen.getByRole('button', { name: 'Delete my data' }));
    const dialog = screen.getByRole('dialog', { name: 'Delete your data?' });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Delete my data' }));
    await waitFor(() => expect(within(dialog).getByRole('alert').textContent).toMatch(/not available yet/));
    expect(consentStore.state?.status).toBe('accepted');
  });

  it('renders the deletion pending state from get_consent_state', async () => {
    await openSettings('deletion_pending');
    expect(await screen.findByTestId('deletion-pending')).toBeTruthy();
  });
});

describe('deletion pending screen', () => {
  it('shows Deletion pending with the install ID while the server has not confirmed', async () => {
    rust('deletion_pending');
    render(DeletionPendingScreen);
    await loadConsent();
    expect(await screen.findByTestId('deletion-pending')).toBeTruthy();
    expect(screen.getByText('Deleting your data')).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('install-id').textContent).toBe(INSTALL_ID));
    expect(consentStore.deletionConfirmed).toBe(false);
  });

  it('polls get_consent_state and flags the deletion as confirmed when the status leaves deletion_pending', async () => {
    rust('deletion_pending');
    render(DeletionPendingScreen);
    await loadConsent();
    const before = ran('get_consent_state').length;
    current = st('unanswered'); // the server confirmed, the backend cleared its marker
    await fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
    await waitFor(() => expect(ran('get_consent_state').length).toBeGreaterThan(before));
    await waitFor(() => expect(consentStore.state?.status).toBe('unanswered'));
    expect(consentStore.deletionConfirmed).toBe(true);
  });

  it('tells the user on the next consent screen that deletion was confirmed', async () => {
    rust('deletion_pending');
    await loadConsent();
    // The server confirms, the backend restarts Verenu: a new process, the same webview storage.
    resetConsentStoreForTests();
    current = st('unanswered');
    await loadConsent();
    render(LegalConsentScreen);
    expect(screen.getByTestId('deletion-confirmed').textContent).toMatch(/Deletion confirmed/);
    // Shown once: a later launch does not repeat it.
    resetConsentStoreForTests();
    await loadConsent();
    expect(consentStore.deletionConfirmed).toBe(false);
  });

  it('keeps polling on a timer', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    try {
      rust('deletion_pending');
      render(DeletionPendingScreen);
      await loadConsent();
      const before = ran('get_consent_state').length;
      await vi.advanceTimersByTimeAsync(10_000);
      expect(ran('get_consent_state').length).toBeGreaterThan(before);
    } finally {
      vi.useRealTimers();
    }
  });
});
