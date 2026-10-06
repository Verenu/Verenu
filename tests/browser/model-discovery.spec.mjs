import { test, expect } from './fixtures.mjs';

test('capability discoveries survive reload without changing selected models', async ({ page, session, cachedCatalogs }) => {
  // Complete first-open migration before recording established selections.
  // Legacy single-provider settings may store a bare ID while priority settings
  // store provider/ID; both must identify the same model.
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.models"]').click();
  await expect.poll(async () => {
    const stored = await session.invoke('get_all_settings');
    const sameModel = (task) => stored[`${task}_model`] === stored[`${task}_default_model`] ||
      `${stored[`${task}_provider`]}/${stored[`${task}_model`]}` === stored[`${task}_default_model`];
    return !!stored.cleanup_default_model && !!stored.transcription_default_model && sameModel('cleanup') && sameModel('transcription');
  }).toBe(true);
  const settings = await session.invoke('get_all_settings');
  const providers = ['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai'];
  const now = Date.now();
  const cache = Object.fromEntries(providers.map((provider) => [provider, {
    ids: [], everSeen: [], lastSuccessAt: now, lastAttemptAt: now,
    lastError: null, missing: {}, metadata: {}, warning: null,
  }]));
  cache.openrouter.ids = ['synthetic/future-model:free', 'synthetic/incompatible'];
  cache.openrouter.everSeen = [...cache.openrouter.ids];
  cache.openrouter.metadata = {
    'synthetic/future-model:free': { label: 'Synthetic future cleanup', tasks: ['cleanup'] },
    'synthetic/incompatible': { label: 'Synthetic incompatible model', tasks: [] },
  };
  try {
    // Real native settings storage, with a fresh cache to make this regression
    // deterministic without intercepting provider or IPC responses.
    await session.invoke('save_setting', { key: 'provider_model_cache', value: cache });
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
    await page.reload();
    for (let pass = 0; pass < 2; pass++) {
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.models"]').click();
      await page.getByRole('button', { name: 'Change model', exact: true }).nth(1).click();
      const dialog = page.getByRole('dialog');
      await dialog.getByRole('navigation', { name: 'Filter by provider' }).getByRole('button', { name: /OpenRouter/ }).click();
      await expect(dialog.getByText('Synthetic future cleanup', { exact: true })).toBeVisible();
      await expect(dialog.getByText('Synthetic incompatible model', { exact: true })).toHaveCount(0);
      await expect(dialog.getByRole('button', { name: 'Refresh models', exact: true })).toBeVisible();
      await expect(dialog.getByText(/Model lists refresh daily/)).toBeVisible();
      await page.screenshot({ path: test.info().outputPath(`model-discovery-${pass}.png`) });
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      const stored = await session.invoke('get_all_settings');
      expect(stored.cleanup_model).toEqual(settings.cleanup_model);
      expect(stored.transcription_model).toEqual(settings.transcription_model);
      expect(stored.provider_model_cache.openrouter.metadata).toEqual(cache.openrouter.metadata);
      if (pass === 0) await page.reload();
    }
  } finally {
    await session.invoke('save_setting', { key: 'provider_model_cache', value: settings.provider_model_cache ?? {} });
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: settings.advanced_model_ui === true });
  }
});
