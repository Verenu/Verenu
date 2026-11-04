import { afterEach, expect, it, vi } from 'vitest';
import { invoke } from './tauri';
import { customProviderStore, refreshCustomProviders, type CustomProvider } from './customProviders.svelte';

vi.mock('./tauri', () => ({ invoke: vi.fn() }));

const old: CustomProvider = {
  id: 'custom:87654321-1234-4234-8234-123456789123', name: 'Old name', protocol: 'openai',
  base_url: 'http://127.0.0.1:1/v1', requires_key: false, supports_transcription: true,
  supports_cleanup: false, auth_header: null, extra_headers: {}, body_overrides: null,
  transcription_models: ['synthetic'], cleanup_models: [],
};

afterEach(() => {
  customProviderStore.providers = [];
  vi.resetAllMocks();
});

it('keeps newer provider settings when an older refresh finishes later', async () => {
  let resolveOld!: (value: { custom_providers: CustomProvider[] }) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise(resolve => { resolveOld = resolve; }));
  vi.mocked(invoke).mockResolvedValueOnce({ custom_providers: [{ ...old, name: 'New name' }] });

  const older = refreshCustomProviders();
  await refreshCustomProviders();
  expect(customProviderStore.providers[0]?.name).toBe('New name');
  resolveOld({ custom_providers: [old] });
  await older;
  expect(customProviderStore.providers[0]?.name).toBe('New name');
});

it('uses an older successful refresh when a newer refresh fails', async () => {
  let resolveOld!: (value: { custom_providers: CustomProvider[] }) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise(resolve => { resolveOld = resolve; }));
  vi.mocked(invoke).mockRejectedValueOnce(new Error('Synthetic failure'));

  const older = refreshCustomProviders();
  await expect(refreshCustomProviders()).rejects.toThrow('Synthetic failure');
  resolveOld({ custom_providers: [old] });
  await older;
  expect(customProviderStore.providers[0]?.name).toBe('Old name');
});
