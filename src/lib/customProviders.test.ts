import { beforeEach, describe, expect, it, vi } from 'vitest';
import { beginCustomProviderLoad, customProviderStore, refreshCustomProviders, type CustomProvider } from './customProviders.svelte';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('./tauri', () => ({ invoke }));

function provider(name: string): CustomProvider {
  return {
    id: 'custom:00000000-0000-0000-0000-000000000001', name, protocol: 'openai',
    base_url: 'http://localhost:8000/v1', requires_key: false,
    supports_transcription: true, supports_cleanup: false, auth_header: null,
    extra_headers: {}, body_overrides: null, transcription_models: [], cleanup_models: [],
  };
}

describe('custom provider loads', () => {
  beforeEach(() => {
    invoke.mockReset();
    customProviderStore.providers = [];
  });

  it('does not let an older refresh overwrite a newer result', async () => {
    const pending: Array<(value: { custom_providers: CustomProvider[] }) => void> = [];
    invoke.mockImplementation(() => new Promise(resolve => pending.push(resolve)));
    const older = refreshCustomProviders();
    const newer = refreshCustomProviders();
    pending[1]({ custom_providers: [provider('Renamed')] });
    await newer;
    pending[0]({ custom_providers: [provider('Original')] });
    await older;
    expect(customProviderStore.providers[0]?.name).toBe('Renamed');
  });

  it('does not let a delayed Models snapshot overwrite a later refresh', async () => {
    const applyModelsSnapshot = beginCustomProviderLoad();
    invoke.mockResolvedValue({ custom_providers: [provider('Renamed')] });
    await refreshCustomProviders();
    applyModelsSnapshot([provider('Original')]);
    expect(customProviderStore.providers[0]?.name).toBe('Renamed');
  });

  it('does not apply an older snapshot after a newer refresh fails', async () => {
    customProviderStore.providers = [provider('Current')];
    const applyModelsSnapshot = beginCustomProviderLoad();
    invoke.mockRejectedValue(new Error('settings unavailable'));
    await expect(refreshCustomProviders()).rejects.toThrow('settings unavailable');
    applyModelsSnapshot([provider('Original')]);
    expect(customProviderStore.providers[0]?.name).toBe('Current');
  });
});
