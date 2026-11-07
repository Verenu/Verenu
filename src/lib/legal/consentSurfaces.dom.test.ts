// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../tauri', async (original) => ({
  ...(await original<typeof import('../tauri')>()),
  invoke: (...a: unknown[]) => invoke(...a),
  isTauriRuntime: () => true,
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import ConsentFlowDialog from './ConsentFlowDialog.svelte';
import SyncSection from '../components/settings/SyncSection.svelte';
import ModelsSection from '../components/settings/ModelsSection.svelte';
import AudioSection from '../components/settings/AudioSection.svelte';
import CleanupPromptModal from '../components/settings/CleanupPromptModal.svelte';
import HistoryList from '../views/home/HistoryList.svelte';
import { refreshSyncStatus } from '../syncStore.svelte';
import { cleanupPromptEditor } from '../stores.svelte';
import { consentStore, loadConsent, resetConsentStoreForTests } from './consentStore.svelte';
import type { ConsentState, ConsentStatus } from './consent';

function state(status: ConsentStatus): ConsentState {
  return { status, termsVersion: '2026-11-03', privacyVersion: '2026-11-03.1', acceptedAt: status === 'accepted' ? 1 : null, syncPending: false };
}

const SYNC_STATUS = {
  this_device: { uuid: 'me', name: 'This PC', port: 1234, tailscale_ips: [] },
  listener_active: true,
  pairing: null,
  discovered: [{ uuid: 'near', name: 'Nearby PC', addresses: ['10.0.0.2'], port: 1, paired: false, last_seen_ms: 1 }],
  peers: [{ uuid: 'peer', name: 'Laptop', added_at: null, last_sync_at: null, state: 'idle', error: null, online: true }],
  last_error_hint: null,
};

let current: ConsentState;
function backend(status: ConsentStatus) {
  current = state(status);
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'get_consent_state': return current;
      case 'accept_legal': return (current = state('accepted'));
      case 'get_install_id': return 'inst-1';
      case 'sync_get_status': return SYNC_STATUS;
      case 'get_api_key_status': return { groq: true, openai: true };
      case 'get_all_settings': return { advanced_model_ui: true };
      case 'nvidia_reuse_status': return { state: 'not_installed', message: '', has_files: false };
      case 'get_hardware_capabilities': return { nvidia_gpu: true };
      case 'local_models_supported_on_this_platform': return true;
      case 'get_local_transcription_state':
      case 'get_local_llm_state': return { is_downloading: false, downloading_model_id: null, loaded_model_id: null, loading: false };
      case 'get_local_llm_runtime_info': return { installed: false, is_downloading: false, downloading_model_id: null };
      case 'get_model_performance':
      case 'list_local_stt_models':
      case 'list_local_llm_models':
      case 'get_microphones':
      case 'lint_cleanup_prompt': return [];
      case 'get_default_cleanup_prompt': return 'Clean up the text.';
      case 'test_cleanup_prompt': return { passed: true };
      default: return null;
    }
  });
}

const called = (command: string) => invoke.mock.calls.some(([c]) => c === command);

beforeEach(() => {
  Element.prototype.animate = function () {
    return { cancel() {}, effect: null as unknown, set onfinish(fn: () => void) { queueMicrotask(fn); } } as unknown as Animation;
  };
  invoke.mockReset();
  resetConsentStoreForTests();
  cleanupPromptEditor.open = false;
});
afterEach(cleanup);

type Surface = {
  name: string;
  mount: () => void | Promise<void>;
  control: () => HTMLElement;
  hint: RegExp;
  enabled: () => boolean;
};

const retry = vi.fn();

const SURFACES: Surface[] = [
  {
    name: 'Sync now (paired device)',
    mount: async () => { render(SyncSection); await refreshSyncStatus(); },
    control: () => screen.getByRole('button', { name: 'Sync Laptop now' }),
    hint: /Pairing and syncing devices is off/,
    enabled: () => called('sync_now'),
  },
  {
    name: 'Pair (nearby device)',
    mount: async () => { render(SyncSection); await refreshSyncStatus(); },
    control: () => screen.getByRole('button', { name: 'Pair' }),
    hint: /Pairing and syncing devices is off/,
    enabled: () => called('sync_start_pairing'),
  },
  {
    name: 'cloud preset in Models',
    mount: () => { render(ModelsSection); },
    control: () => screen.getByRole('button', { name: 'Use Quality' }),
    hint: /Cloud transcription and cleanup, and model downloads is off/,
    enabled: () => screen.getByRole('button', { name: 'Use Quality' }).getAttribute('aria-pressed') === 'true',
  },
  {
    name: 'local model download in Models',
    mount: () => { render(ModelsSection); },
    control: () => screen.getByRole('button', { name: 'Download 1.5 GB' }),
    hint: /model downloads is off/,
    enabled: () => called('download_local_stt_model'),
  },
  {
    name: 'NVIDIA RE-USE toggle',
    mount: () => { render(AudioSection); },
    control: () => screen.getByRole('switch', { name: 'Use NVIDIA RE-USE' }),
    hint: /The NVIDIA runtime download is off/,
    enabled: () => called('nvidia_reuse_install'),
  },
  {
    name: 'cleanup prompt audit',
    mount: async () => {
      cleanupPromptEditor.open = true;
      cleanupPromptEditor.provider = 'groq';
      cleanupPromptEditor.model = 'm';
      cleanupPromptEditor.intensity = 'medium';
      render(CleanupPromptModal);
      await waitFor(() => expect(screen.getByRole('button', { name: 'Audit & save' })).toBeTruthy());
    },
    control: () => screen.getByRole('button', { name: 'Audit & save' }),
    hint: /Auditing a prompt with a cloud model is off/,
    enabled: () => called('test_cleanup_prompt'),
  },
  {
    name: 'Home dictation retry',
    mount: () => {
      retry.mockReset();
      render(HistoryList, {
        props: {
          recents: [], failedEntry: { created_at: '2026-11-03T12:00:00Z' }, cancelledEntry: null, loading: false,
          hasMoreHistory: false, loadingMore: false, retrying: false, resumingCancelled: false, copiedId: null,
          keyLabels: [], onRetry: retry, onContinueCancelled: () => {}, onDismissCancelled: () => {},
          onLoadOlder: () => {}, onCopy: () => {},
        } as never,
      });
    },
    control: () => screen.getByRole('button', { name: 'Retry' }),
    hint: /Retrying with a cloud provider is off/,
    enabled: () => retry.mock.calls.length > 0,
  },
];

describe.each(SURFACES)('$name after Skip', (s) => {
  async function mountSkipped() {
    backend('skipped');
    render(ConsentFlowDialog);
    await s.mount();
    await loadConsent();
    await waitFor(() => expect(consentStore.state?.status).toBe('skipped'));
    await waitFor(() => expect(() => s.control()).not.toThrow());
  }

  it('is disabled and explains why', async () => {
    await mountSkipped();
    await waitFor(() => expect(screen.getAllByText(s.hint).length).toBeGreaterThan(0), { timeout: 5000 });
    await waitFor(() => {
      const el = s.control();
      const dimmed = el.getAttribute('aria-disabled') === 'true' || el.classList.contains('consent-dim') || el.classList.contains('consent-gated');
      expect(dimmed, el.outerHTML.slice(0, 300)).toBe(true);
    }, { timeout: 5000 });
    expect(s.enabled()).toBe(false);
  });

  it('opens the Terms dialog when tapped and stays off', async () => {
    await mountSkipped();
    await fireEvent.click(s.control());
    expect(await screen.findByTestId('consent-flow')).toBeTruthy();
    expect(s.enabled()).toBe(false);
  });

  it('enables the tapped feature after Accept', async () => {
    await mountSkipped();
    await fireEvent.click(s.control());
    await fireEvent.click(await screen.findByRole('button', { name: 'Accept and continue' }));
    await waitFor(() => expect(consentStore.state?.status).toBe('accepted'));
    await waitFor(() => expect(s.enabled()).toBe(true));
  });

  it('stays off after Not now', async () => {
    await mountSkipped();
    await fireEvent.click(s.control());
    await fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(screen.queryByTestId('consent-flow')).toBeNull());
    expect(called('accept_legal')).toBe(false);
    expect(s.enabled()).toBe(false);
  });
});
