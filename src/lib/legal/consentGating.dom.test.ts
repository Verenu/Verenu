// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../tauri', async (original) => ({
  ...(await original<typeof import('../tauri')>()),
  invoke: (...a: unknown[]) => invoke(...a),
  isTauriRuntime: () => true,
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import ConsentFlowDialog from './ConsentFlowDialog.svelte';
import ConsentArea from './ConsentArea.svelte';
import GatedButton from './GatedButton.svelte';
import PrivacySection from '../components/settings/PrivacySection.svelte';
import AboutSection from '../components/settings/AboutSection.svelte';
import ApiKeysSection from '../components/settings/ApiKeysSection.svelte';
import IntegrationsSection from '../components/settings/IntegrationsSection.svelte';
import ProviderStep from '../setup/steps/ProviderStep.svelte';
import { downloadLocalModel } from '../localSttStore.svelte';
import { classifyIpcError } from '../errors';
import { consentFlow, consentStore, handleConsentError, loadConsent, requireConsent, resetConsentStoreForTests } from './consentStore.svelte';
import { isConsentRequiredError, type ConsentState, type ConsentStatus } from './consent';

function state(status: ConsentStatus): ConsentState {
  return { status, termsVersion: '2026-11-03', privacyVersion: '2026-11-03', acceptedAt: status === 'accepted' ? 1 : null, syncPending: false };
}

let current: ConsentState;
function backend(status: ConsentStatus) {
  current = state(status);
  invoke.mockImplementation(async (command: string, args?: { key?: string }) => {
    switch (command) {
      case 'get_consent_state': return current;
      case 'accept_legal': return (current = state('accepted'));
      case 'skip_legal': return (current = state('skipped'));
      case 'get_install_id': return 'inst-1';
      case 'get_all_settings': return { analytics_enabled: true, verenu_service_checks_enabled: true, beta_updates_enabled: false };
      case 'get_setting': return args?.key === 'beta_updates_enabled' ? false : null;
      case 'get_api_key_status': return {};
      case 'get_custom_providers': return [];
      default: return null;
    }
  });
}

const saved = (key: string, value: unknown) =>
  invoke.mock.calls.some(([c, a]) => c === 'save_setting' && a?.key === key && a?.value === value);
const called = (command: string) => invoke.mock.calls.some(([c]) => c === command);

beforeEach(() => {
  Element.prototype.animate = function () {
    return { cancel() {}, effect: null as unknown, set onfinish(fn: () => void) { queueMicrotask(fn); } } as unknown as Animation;
  };
  invoke.mockReset();
  resetConsentStoreForTests();
});
afterEach(cleanup);

type GatedCase = {
  feature: string;
  mount: () => void;
  control: () => HTMLElement;
  /** The hint copy that explains why the control is off. */
  hint: RegExp;
  /** True once the feature the user tapped is on. */
  enabled: () => boolean;
};

const CASES: GatedCase[] = [
  {
    feature: 'product analytics toggle',
    mount: () => render(PrivacySection),
    control: () => screen.getByRole('switch', { name: 'Share pseudonymous product analytics' }),
    hint: /Product analytics is off/,
    enabled: () => saved('analytics_enabled', true),
  },
  {
    feature: 'Verenu service checks toggle',
    mount: () => render(PrivacySection),
    control: () => screen.getByRole('switch', { name: 'Allow Verenu service checks' }),
    hint: /Service checks is off/,
    enabled: () => saved('verenu_service_checks_enabled', true),
  },
  {
    feature: 'Check for Updates button',
    mount: () => render(AboutSection, { appVersion: '1.0.0' }),
    control: () => screen.getByRole('button', { name: 'Check for Updates' }),
    hint: /Update checks is off/,
    enabled: () => called('check_for_update'),
  },
  {
    feature: 'cloud provider card in setup',
    mount: () => render(ProviderStep, { provider: 'local' as never }),
    control: () => screen.getByRole('button', { name: /Groq/ }),
    hint: /Cloud providers is off/,
    enabled: () => screen.getByRole('button', { name: /Groq/ }).getAttribute('aria-pressed') === 'true',
  },
  {
    feature: 'Add provider button (provider pickers and API keys)',
    mount: () => render(ApiKeysSection),
    control: () => screen.getByRole('button', { name: /Add provider/ }),
    hint: /Adding or changing cloud providers is off/,
    enabled: () => !!document.querySelector('[role="dialog"]:not([data-testid="consent-flow"]), .drawer, [aria-label="Back to providers"], [class*="drawer"]'),
  },
];

describe.each(CASES)('$feature after Skip', (c) => {
  async function mountSkipped() {
    backend('skipped');
    render(ConsentFlowDialog);
    c.mount();
    await loadConsent();
    await waitFor(() => expect(consentStore.state?.status).toBe('skipped'));
  }

  it('is disabled and explains why', async () => {
    await mountSkipped();
    await waitFor(() => expect(screen.getAllByText(c.hint).length).toBeGreaterThan(0));
    const control = c.control();
    expect(control.getAttribute('aria-disabled') === 'true' || control.classList.contains('consent-gated') || control.classList.contains('consent-dim')).toBe(true);
    expect(c.enabled()).toBe(false);
  });

  it('shows the Terms and Privacy dialog when tapped, and stays off until Accept', async () => {
    await mountSkipped();
    await fireEvent.click(c.control());
    expect(await screen.findByTestId('consent-flow')).toBeTruthy();
    expect(within(screen.getByTestId('consent-flow')).getByRole('link', { name: 'Terms of Service' })).toBeTruthy();
    expect(c.enabled()).toBe(false);
  });

  it('enables the tapped feature after Accept', async () => {
    await mountSkipped();
    await fireEvent.click(c.control());
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('accepted'));
    await waitFor(() => expect(screen.queryByTestId('consent-flow')).toBeNull());
    await waitFor(() => expect(c.enabled()).toBe(true));
  });

  it('leaves the feature off when the dialog is dismissed', async () => {
    await mountSkipped();
    await fireEvent.click(c.control());
    await fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(screen.queryByTestId('consent-flow')).toBeNull());
    expect(consentStore.state?.status).toBe('skipped');
    expect(called('accept_legal')).toBe(false);
    expect(c.enabled()).toBe(false);
  });
});

describe('IntegrationsSection (GitHub and T3 Code)', () => {
  it('marks every connect action as needing consent and shows the hint', async () => {
    backend('skipped');
    render(IntegrationsSection);
    await loadConsent();
    await waitFor(() => expect(screen.getAllByText(/Integrations is off/).length).toBe(1));
    const connectButtons = screen.getAllByRole('button', { name: /^Connect (GitHub|T3 Code)$/ });
    expect(connectButtons.length).toBe(2);
    await waitFor(() => connectButtons.forEach((b) => expect(b.getAttribute('aria-disabled')).toBe('true')));
  });
});

describe('ConsentArea', () => {
  it('re-clicks the tapped control after Accept and ignores unmarked controls', async () => {
    backend('skipped');
    const action = vi.fn();
    const { getByText } = render(ConsentFlowDialog);
    void getByText;
    const harness = document.createElement('div');
    document.body.append(harness);
    render(ConsentArea, {
      target: harness,
      props: {
        children: ((anchor: Comment) => {
          const area = document.createElement('div');
          area.innerHTML = '<button data-needs-consent id="gated">Sync now</button><button id="free">Rename</button>';
          anchor.before(area);
          area.querySelector('#gated')!.addEventListener('click', action);
        }) as never,
      },
    });
    await loadConsent();
    await fireEvent.click(document.getElementById('free')!);
    expect(consentFlow.open).toBe(false);
    await fireEvent.click(document.getElementById('gated')!);
    expect(consentFlow.open).toBe(true);
    expect(action).not.toHaveBeenCalled();
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(action).toHaveBeenCalledTimes(1));
  });
});

describe('GatedButton', () => {
  it('runs immediately when accepted', async () => {
    backend('accepted');
    const onclick = vi.fn();
    render(GatedButton, { onclick });
    await loadConsent();
    await fireEvent.click(screen.getByRole('button'));
    expect(onclick).toHaveBeenCalledTimes(1);
  });
});

describe('local model downloads', () => {
  it('do not start after Skip, and start after Accept', async () => {
    backend('skipped');
    render(ConsentFlowDialog);
    await loadConsent();
    expect(await downloadLocalModel('tiny')).toBe(false);
    expect(called('download_local_stt_model')).toBe(false);
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(called('download_local_stt_model')).toBe(true));
  });
});

describe('consent_required error', () => {
  it('is recognised from the backend code and the current message', () => {
    expect(isConsentRequiredError('consent_required')).toBe(true);
    expect(isConsentRequiredError(new Error('Accept the current Terms and Privacy Policy to use online features'))).toBe(true);
    expect(isConsentRequiredError('network timeout')).toBe(false);
  });

  it('classifies as consent-required so the toast offers Review terms', () => {
    expect(classifyIpcError('consent_required').kind).toBe('consent-required');
    expect(classifyIpcError('Accept the current Terms and Privacy Policy to use online features').kind).toBe('consent-required');
  });

  it('opens the accept dialog and retries the action after Accept', async () => {
    backend('skipped');
    render(ConsentFlowDialog);
    await loadConsent();
    const retry = vi.fn();
    expect(handleConsentError('consent_required', retry)).toBe(true);
    expect(await screen.findByTestId('consent-flow')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(retry).toHaveBeenCalledTimes(1));
    expect(handleConsentError('boom')).toBe(false);
  });

  it('requireConsent runs at once when accepted', async () => {
    backend('accepted');
    await loadConsent();
    const run = vi.fn();
    expect(requireConsent(run)).toBe(true);
    expect(run).toHaveBeenCalled();
  });
});
