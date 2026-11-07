// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../tauri', async (original) => ({
  ...(await original<typeof import('../tauri')>()),
  invoke: (...a: unknown[]) => invoke(...a),
  isTauriRuntime: () => true,
}));

import ConsentFlowDialog from './ConsentFlowDialog.svelte';
import Insights from '../views/Insights.svelte';
import { devInvoke } from '../tauri.dev';
import { consentStore, loadConsent, resetConsentStoreForTests } from './consentStore.svelte';
import { PRIVACY_URL, PRIVACY_VERSION, TERMS_URL, TERMS_VERSION, type ConsentState, type ConsentStatus } from './consent';

function state(status: ConsentStatus): ConsentState {
  return { status, termsVersion: TERMS_VERSION, privacyVersion: PRIVACY_VERSION, acceptedAt: null, syncPending: false };
}

let current: ConsentState;
let pricingCalls = 0;
function backend(status: ConsentStatus) {
  current = state(status);
  pricingCalls = 0;
  invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === 'get_consent_state') return current;
    if (command === 'accept_legal') return (current = state('accepted'));
    if (command === 'get_insights_pricing') {
      pricingCalls += 1;
      if (current.status !== 'accepted') throw 'consent_required';
    }
    return devInvoke(command, args as never);
  });
}

beforeEach(() => {
  Element.prototype.animate = function () {
    return { cancel() {}, effect: null as unknown, set onfinish(fn: () => void) { queueMicrotask(fn); } } as unknown as Animation;
  };
  invoke.mockReset();
  resetConsentStoreForTests();
});
afterEach(cleanup);

describe('Insights pricing after Skip', () => {
  it('shows a hint instead of failing silently, then loads pricing after Accept; Not now keeps it off', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    backend('skipped');
    render(ConsentFlowDialog);
    render(Insights);
    await loadConsent();
    await waitFor(() => expect(screen.getAllByText(/Provider pricing for cost estimates is off/).length).toBe(1), { timeout: 20000 });
    expect(warn).not.toHaveBeenCalledWith('OpenRouter pricing refresh failed:', expect.anything());

    await fireEvent.click(screen.getByRole('button', { name: 'Review and accept' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(screen.queryByTestId('consent-flow')).toBeNull());
    expect(consentStore.state?.status).toBe('skipped');
    const callsBefore = pricingCalls;

    await fireEvent.click(screen.getByRole('button', { name: 'Review and accept' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(pricingCalls).toBeGreaterThan(callsBefore));
    await waitFor(() => expect(screen.queryByText(/Provider pricing for cost estimates is off/)).toBeNull());
    warn.mockRestore();
  }, 40000);
});

describe('legal document versions', () => {
  it('links Terms 2026-11-03 and Privacy 2026-11-03.1', () => {
    expect(TERMS_URL).toBe('https://verenu.com/legal/terms/2026-11-03');
    expect(PRIVACY_URL).toBe('https://verenu.com/legal/privacy/2026-11-03.1');
    expect(PRIVACY_VERSION).toBe('2026-11-03.1');
  });
});
