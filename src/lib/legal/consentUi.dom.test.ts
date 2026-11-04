// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../tauri', () => ({ invoke: (...a: unknown[]) => invoke(...a), isTauriRuntime: () => true }));
const openShell = vi.fn();
vi.mock('@tauri-apps/plugin-shell', () => ({ open: (...a: unknown[]) => openShell(...a) }));

import LegalConsentScreen from './LegalConsentScreen.svelte';
import PrivacyConsentSettings from './PrivacyConsentSettings.svelte';
import ConsentHint from './ConsentHint.svelte';
import { consentStore, loadConsent, resetConsentStoreForTests } from './consentStore.svelte';
import { PRIVACY_URL, TERMS_URL, type ConsentState, type ConsentStatus } from './consent';

const INSTALL_ID = 'inst-1234';
function state(status: ConsentStatus, extra: Partial<ConsentState> = {}): ConsentState {
  return {
    status, termsVersion: '2026-11-03', privacyVersion: '2026-11-03',
    acceptedAt: status === 'accepted' ? Date.UTC(2026, 10, 3, 12) : null, syncPending: false, ...extra,
  };
}

/** Backend double: commands mutate one shared state, like the real store. */
function backend(initial: ConsentState) {
  let current = initial;
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'get_consent_state': return current;
      case 'accept_legal': return (current = state('accepted'));
      case 'skip_legal': return (current = state('skipped'));
      case 'delete_my_data': return (current = state('deletion_pending'));
      case 'get_install_id': return INSTALL_ID;
      default: throw new Error(`unexpected command ${command}`);
    }
  });
}

beforeEach(() => {
  // happy-dom's Web Animations reject when Svelte cancels a transition; finish them at once instead.
  Element.prototype.animate = function () {
    return { cancel() {}, effect: null as unknown, set onfinish(fn: () => void) { queueMicrotask(fn); } } as unknown as Animation;
  };
  invoke.mockReset(); openShell.mockReset(); resetConsentStoreForTests();
});
afterEach(cleanup);

describe('onboarding consent screen', () => {
  it('shows the version-pinned Terms and Privacy links and the skip copy', async () => {
    backend(state('unanswered'));
    render(LegalConsentScreen);
    const terms = screen.getByRole('link', { name: 'Terms of Service' });
    const privacy = screen.getByRole('link', { name: 'Privacy Policy' });
    expect(terms.getAttribute('href')).toBe(TERMS_URL);
    expect(privacy.getAttribute('href')).toBe(PRIVACY_URL);
    expect(TERMS_URL).toContain('2026-11-03');
    expect(document.body.textContent).toContain('Analytics and online features stay off');
    expect(document.body.textContent).toContain('Settings → Privacy');
    await fireEvent.click(terms);
    await waitFor(() => expect(openShell).toHaveBeenCalledWith(TERMS_URL));
  });

  it('accept calls accept_legal and stores the accepted state', async () => {
    backend(state('unanswered'));
    render(LegalConsentScreen);
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('accepted'));
    expect(invoke).toHaveBeenCalledWith('accept_legal');
  });

  it('skip calls skip_legal and stores the skipped state', async () => {
    backend(state('unanswered'));
    render(LegalConsentScreen);
    await fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('skipped'));
    expect(invoke).toHaveBeenCalledWith('skip_legal');
  });

  it('shows the error and keeps the screen actionable when saving fails', async () => {
    invoke.mockRejectedValue('disk full');
    render(LegalConsentScreen);
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Accept and continue' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('re-prompt mode explains that the terms changed', () => {
    render(LegalConsentScreen, { mode: 'update' });
    expect(screen.getByText('Our terms and privacy policy changed')).toBeTruthy();
    expect(document.querySelector('[data-mode="update"]')).toBeTruthy();
  });
});

describe('Settings > Privacy consent controls', () => {
  it('copies the install ID', async () => {
    backend(state('accepted'));
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    render(PrivacyConsentSettings);
    await waitFor(() => expect(screen.getByTestId('install-id').textContent).toBe(INSTALL_ID));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(INSTALL_ID));
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
  });

  it('offers Accept terms only when skipped, and accepting updates the status', async () => {
    backend(state('skipped'));
    render(PrivacyConsentSettings);
    await waitFor(() => expect(screen.getByTestId('consent-status').textContent).toBe('Skipped'));
    await fireEvent.click(screen.getByRole('button', { name: 'Accept terms' }));
    await waitFor(() => expect(screen.getByTestId('consent-status').textContent).toBe('Accepted'));
    expect(screen.queryByRole('button', { name: 'Accept terms' })).toBeNull();
  });

  it('writes the accepted date with a space after the status', async () => {
    backend(state('accepted'));
    const { container } = render(PrivacyConsentSettings);
    await waitFor(() => expect(screen.getByTestId('consent-status').textContent).toBe('Accepted'));
    expect(container.textContent).toMatch(/Accepted on \w+ \d+, 2026|Accepted on \d+ \w+ 2026/);
  });

  it('cancelling the delete confirm does not call delete_my_data', async () => {
    backend(state('accepted'));
    render(PrivacyConsentSettings);
    await waitFor(() => screen.getByTestId('install-id'));
    await fireEvent.click(screen.getByRole('button', { name: 'Delete my data' }));
    expect(screen.getByRole('dialog', { name: 'Delete your data?' })).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(invoke).not.toHaveBeenCalledWith('delete_my_data');
  });

  it('confirming deletion calls delete_my_data and shows the pending state', async () => {
    backend(state('accepted'));
    render(PrivacyConsentSettings);
    await waitFor(() => screen.getByTestId('install-id'));
    await fireEvent.click(screen.getByRole('button', { name: 'Delete my data' }));
    const dialog = screen.getByRole('dialog');
    const confirm = Array.from(dialog.querySelectorAll('button')).find((b) => b.textContent?.trim() === 'Delete my data')!;
    await fireEvent.click(confirm);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('delete_my_data'));
    expect(await screen.findByTestId('deletion-pending')).toBeTruthy();
    const trigger = screen.getByRole('button', { name: 'Deletion pending' }) as HTMLButtonElement;
    expect(trigger.disabled).toBe(true);
  });

  it('renders the pending state from a stored deletion_pending status', async () => {
    backend(state('deletion_pending'));
    render(PrivacyConsentSettings);
    expect(await screen.findByTestId('deletion-pending')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Deletion pending' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByRole('button', { name: 'Accept terms' })).toBeNull();
  });
});

describe('gated feature hints', () => {
  it('shows a hint with an accept action when skipped, and nothing when accepted', async () => {
    backend(state('skipped'));
    const onaccept = vi.fn();
    render(ConsentHint, { feature: 'Product analytics', thenEnable: onaccept });
    expect(screen.queryByTestId('consent-hint')).toBeNull();
    await loadConsent();
    expect(await screen.findByTestId('consent-hint')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Review and accept' }));
    expect(onaccept).not.toHaveBeenCalled();
    cleanup();
    backend(state('accepted'));
    await loadConsent();
    render(ConsentHint, { feature: 'Product analytics' });
    expect(screen.queryByTestId('consent-hint')).toBeNull();
  });
});
