import { test, expect } from './fixtures.mjs';

for (const task of ['transcription', 'cleanup']) {
  test(`${task} picker stays usable with a large catalog and a pending refresh`, async ({ page, session }) => {
    // Normalize a fresh install before taking the restoration snapshot. Absent
    // model settings are returned as null and cannot be written back as IDs.
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await expect.poll(async () => {
      const settings = await session.invoke('get_all_settings');
      return typeof settings.transcription_default_model === 'string'
        && typeof settings.cleanup_default_model === 'string';
    }).toBe(true);
    const original = await session.invoke('get_all_settings');
    original.advanced_model_ui ??= false;
    original.provider_model_cache ??= {};
    const provider = {
      id: 'custom:12345678-1234-1234-8234-123456789014', name: 'Synthetic picker provider',
      protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: false,
      supports_transcription: true, supports_cleanup: true, auth_header: null,
      extra_headers: {}, body_overrides: null,
      transcription_models: ['speech-one', 'speech-two', ...Array.from({ length: 25 }, (_, i) => `aaa-speech-${i}`)],
      cleanup_models: ['cleanup-one', 'cleanup-two', ...Array.from({ length: 25 }, (_, i) => `aaa-cleanup-${i}`)],
    };
    const ids = Array.from({ length: 650 }, (_, i) => `synthetic/model-${String(i).padStart(4, '0')}`);
    const cache = { openrouter: {
      ids, everSeen: ids, lastSuccessAt: 1, lastAttemptAt: 1, lastError: null, missing: {},
      metadata: Object.fromEntries(ids.map(id => [id, { label: id, tasks: ['transcription', 'cleanup'] }])), warning: null,
    } };
    const pending = [];
    // Delay transport, not a fabricated catalog: requests are held before they
    // reach the Rust bridge, then fail as a real offline browser request would.
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_provider_model_catalog') return route.continue();
      await new Promise(resolve => pending.push({ route, resolve }));
    });
    const failPending = async () => {
      for (const entry of pending.splice(0)) { await entry.route.abort('internetdisconnected'); entry.resolve(); }
    };
    const keys = ['custom_providers', 'advanced_model_ui', 'provider_model_cache', `${task}_default_model`, `${task}_fallback_models`, `${task}_model`, `${task}_provider`, `${task}_models_by_provider`, 'model_selection_mode'];
    try {
      await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
      await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
      await session.invoke('save_setting', { key: 'provider_model_cache', value: cache });
      const model = task === 'transcription' ? 'speech-two' : 'cleanup-two';
      const fallbacks = (task === 'transcription' ? provider.transcription_models : provider.cleanup_models)
        .filter(id => id.startsWith('aaa-')).map(id => `${provider.id}/${id}`);
      await session.invoke('save_setting', { key: `${task}_fallback_models`, value: fallbacks });
      await session.invoke('save_setting', { key: `${task}_default_model`, value: `${provider.id}/${model}` });
      await page.reload();
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.models"]').click();
      await expect.poll(() => pending.length).toBeGreaterThan(0);
      const trigger = page.getByRole('button', { name: 'Change model', exact: true }).nth(task === 'transcription' ? 0 : 1);
      await trigger.click();
      const dialog = page.getByRole('dialog');
      await expect(dialog).toBeVisible();
      await expect(dialog.locator('.row-main[aria-current="true"]')).toContainText(model);
      await expect(dialog.getByRole('status')).toContainText('Refreshing model lists…');
      expect(pending.length).toBeGreaterThan(0);
      // Bounds the expensive DOM/transition work regardless of catalog size.
      expect(await dialog.locator('.model-row').count()).toBeLessThan(160);
      await dialog.getByRole('navigation', { name: 'Filter by provider' }).getByRole('button', { name: /OpenRouter/ }).click();
      const rowsBefore = await dialog.locator('.model-row').count();
      await dialog.getByRole('button', { name: /Show next .* OpenRouter models/ }).click();
      expect(await dialog.locator('.model-row').count()).toBeGreaterThan(rowsBefore);
      await dialog.locator('.picker-search').fill('synthetic/model-0649');
      await expect(dialog.getByText('synthetic/model-0649', { exact: true }).first()).toBeVisible();
      await page.screenshot({ path: test.info().outputPath(`${task}-pending-refresh.png`) });
      await failPending();
      await expect(dialog.getByText('Refresh unavailable. Keeping cached models.', { exact: true })).toBeVisible();
      await expect(dialog.getByText('synthetic/model-0649', { exact: true }).first()).toBeVisible();
      await dialog.getByRole('button', { name: 'Refresh models', exact: true }).click();
      await expect.poll(() => pending.length).toBeGreaterThan(0);
      await expect(dialog.getByRole('status')).toContainText('Refreshing model lists…');
      await failPending();
      await expect(dialog.getByText('Refresh unavailable. Keeping cached models.', { exact: true })).toBeVisible();
      await dialog.locator('.picker-search').fill('');
      await dialog.getByRole('navigation', { name: 'Filter by provider' }).getByRole('button', { name: /Synthetic picker provider/ }).click();
      const nextModel = task === 'transcription' ? 'speech-one' : 'cleanup-one';
      await dialog.locator('.picker-search').fill(nextModel);
      await dialog.locator('.row-main').filter({ has: page.locator('.row-name', { hasText: nextModel }) }).click();
      await expect(dialog).toHaveCount(0);
      await expect.poll(() => session.invoke('get_setting', { key: `${task}_default_model` })).toBe(`${provider.id}/${nextModel}`);
      await page.reload();
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.models"]').click();
      await expect(page.locator(`[data-setting-target="models-${task}"]`)).toContainText(nextModel);
      await trigger.click();
      await expect(dialog.locator('.row-main[aria-current="true"]')).toContainText(nextModel);
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      await page.locator(`[data-setting-target="models-${task}"]`).getByRole('button', { name: /Add fallback/ }).click();
      await dialog.getByRole('navigation', { name: 'Filter by provider' }).getByRole('button', { name: /Synthetic picker provider/ }).click();
      const fallback = model;
      await dialog.locator('.picker-search').fill(fallback);
      await dialog.locator('.row-main').filter({ has: page.locator('.row-name', { hasText: fallback }) }).click();
      await expect.poll(() => session.invoke('get_setting', { key: `${task}_fallback_models` })).toContain(`${provider.id}/${fallback}`);
      expect(await session.invoke('get_setting', { key: `${task}_default_model` })).toBe(`${provider.id}/${nextModel}`);
    } finally {
      await failPending();
      await page.unroute('**/__verenu_dev/invoke');
      for (const key of keys) if (original[key] !== undefined) await session.invoke('save_setting', { key, value: original[key] });
    }
  });
}
