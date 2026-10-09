import { test, expect } from './fixtures.mjs';

test('Apple availability refresh stays reachable with device-appropriate picker navigation', async ({ page, session, cachedCatalogs }) => {
  void cachedCatalogs;
  const previous = await session.invoke('get_setting', { key: 'advanced_model_ui' });
  let availabilityRequests = 0;
  const availability = await session.invoke('get_apple_intelligence_availability');
  const supported = ['available', 'intelligence-disabled', 'model-not-ready'].includes(availability.state);
  page.on('request', request => {
    if (request.url().endsWith('/__verenu_dev/invoke') && request.postDataJSON()?.command === 'get_apple_intelligence_availability') availabilityRequests++;
  });
  try {
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await page.locator('[data-setting-target="models-cleanup"]').getByRole('button', { name: 'Change model' }).click();
    const appleRail = page.locator('.picker-rail').getByRole('button', { name: /Apple Intelligence/ });
    if (supported) await appleRail.click();
    else {
      await expect(appleRail).toHaveCount(0);
      await page.locator('.picker-rail').getByRole('button', { name: /All providers/ }).click();
    }
    const refresh = page.getByRole('button', { name: 'Refresh models', exact: true });
    await expect(refresh).toBeEnabled();
    const before = availabilityRequests;
    await refresh.click();
    await expect.poll(() => availabilityRequests).toBeGreaterThan(before);
    await expect(refresh).toBeEnabled();
    await page.getByRole('button', { name: 'Close', exact: true }).click();
  } finally {
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: previous ?? false });
  }
});

test('Apple cleanup defaults and fallbacks stay keyless while keyed endpoints retain warnings', async ({ page, session }) => {
  const previous = await session.invoke('get_all_settings');
  const provider = {
    id: 'custom:12345678-1234-4234-8234-123456789019', name: 'Synthetic keyed endpoint',
    protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: true,
    supports_transcription: false, supports_cleanup: true, auth_header: null,
    extra_headers: {}, body_overrides: null, transcription_models: [], cleanup_models: ['synthetic-cleanup'],
  };
  const keys = ['custom_providers', 'advanced_model_ui', 'cleanup_default_model', 'cleanup_fallback_models'];
  try {
    await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
    for (const [defaultModel, fallbacks] of [
      ['apple-intelligence/system', []],
      [`${provider.id}/synthetic-cleanup`, ['apple-intelligence/system']],
    ]) {
      await session.invoke('save_setting', { key: 'cleanup_default_model', value: defaultModel });
      await session.invoke('save_setting', { key: 'cleanup_fallback_models', value: fallbacks });
      await page.reload();
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.models"]').click();
      const tile = page.locator('[data-setting-target="models-cleanup"]');
      await expect(tile).toBeVisible();
      await expect(tile).toContainText('Apple Intelligence');
      const warnings = tile.locator('.warn-banner').filter({ hasText: 'Missing API keys for:' });
      if (fallbacks.length) {
        await expect(warnings).toHaveText('Missing API keys for: Synthetic keyed endpoint');
      } else {
        await expect(warnings).toHaveCount(0);
      }
      expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).toBe(defaultModel);
      expect((await session.invoke('get_api_key_status'))['apple-intelligence'] === true).toBe(false);
    }
  } finally {
    for (const key of keys) {
      await session.invoke('save_setting', { key, value: previous[key] ?? (key === 'custom_providers' || key === 'cleanup_fallback_models' ? [] : key === 'advanced_model_ui' ? false : null) });
    }
  }
});
