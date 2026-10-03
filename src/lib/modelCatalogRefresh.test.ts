import { expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), saveSetting: vi.fn().mockResolvedValue(undefined) }));
vi.mock('./tauri', () => ({ invoke: mocks.invoke }));
vi.mock('./settings', () => ({ saveSetting: mocks.saveSetting }));
import { modelCatalogStore, refreshCatalog } from './modelCatalogStore.svelte';

it('does not overwrite unread saved catalogs and retries hydration on the next refresh', async () => {
  const warning = vi.spyOn(console, 'warn').mockImplementation(() => {});
  const cached = {
    ids: ['preserved-model'], everSeen: ['preserved-model'], missing: {},
    lastSuccessAt: 1000, lastAttemptAt: 1000, lastError: null,
    metadata: { 'preserved-model': { label: 'Preserved', tasks: ['cleanup'] } },
  };
  mocks.invoke.mockRejectedValueOnce(new Error('Temporary native read failure'));
  try {
    await refreshCatalog('groq', [], 2000);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(mocks.saveSetting).not.toHaveBeenCalled();
    expect(modelCatalogStore.cache).toEqual({});
    expect(modelCatalogStore.refreshing.groq).toBe(false);

    mocks.invoke.mockResolvedValueOnce({ openai: cached });
    mocks.invoke.mockResolvedValueOnce({ ids: ['new-model'], metadata: {}, warning: null });
    await refreshCatalog('groq', [], 3000);
    expect(mocks.invoke.mock.calls[1]).toEqual(['get_setting', { key: 'provider_model_cache' }]);
    expect(mocks.saveSetting).toHaveBeenCalledTimes(1);
    expect(mocks.saveSetting.mock.calls[0][1].openai).toMatchObject(cached);
    expect(mocks.saveSetting.mock.calls[0][1].groq.ids).toEqual(['new-model']);
  } finally {
    warning.mockRestore();
  }
});
