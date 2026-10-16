import { test, expect } from './fixtures.mjs';

for (const [index, task] of ['transcription', 'cleanup'].entries()) {
  test(`${task} picker opens promptly and stays usable during slow and failed refreshes`, async ({ page, session, cachedCatalogs }) => {
    // Let first-open migration establish defaults before taking the restore
    // snapshot, so an initially unset legacy setting is never saved as null.
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await expect.poll(async () => {
      const settings = await session.invoke('get_all_settings');
      return Boolean(settings.transcription_default_model && settings.cleanup_default_model);
    }).toBe(true);
    const previous = await session.invoke('get_all_settings');
    const provider = {
      id: 'custom:12345678-1234-1234-8234-123456789014', name: 'Synthetic picker endpoint',
      protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: false,
      supports_transcription: true, supports_cleanup: true, auth_header: null,
      extra_headers: {}, body_overrides: null,
      transcription_models: ['synthetic-primary', 'synthetic-alternative'],
      cleanup_models: ['synthetic-primary', 'synthetic-alternative'],
    };
    const ids = Array.from({ length: 500 }, (_, i) => `synthetic/model-${i}`);
    const cache = structuredClone(cachedCatalogs);
    cache.openrouter.ids = ids;
    cache.openrouter.everSeen = ids;
    cache.openrouter.metadata = Object.fromEntries(ids.map(id => [id, { label: id, tasks: ['transcription', 'cleanup'] }]));
    let release;
    let requests = 0;
    // Hold actual catalog transport pending, then disconnect it. No success
    // response or provider catalog is fabricated; all other IPC goes to Rust.
    const route = async route => {
      if (route.request().postDataJSON()?.command !== 'get_provider_model_catalog') return route.continue();
      requests++;
      await new Promise(resolve => { release = resolve; });
      await route.abort('connectionfailed');
    };
    try {
      await session.invoke('save_setting', { key: 'provider_model_cache', value: cache });
      await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
      await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
      await session.invoke('save_setting', { key: `${task}_default_model`, value: `${provider.id}/synthetic-primary` });
      await page.reload();
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.models"]').click();
      const trigger = page.getByRole('button', { name: 'Change model', exact: true }).nth(index);
      await trigger.waitFor();
      const dialog = page.getByRole('dialog');
      async function openPromptly() {
        // Start at the click event, excluding Playwright's selector/actionability
        // work, and wait for a painted, nearly opaque card rather than DOM alone.
        const opening = await trigger.evaluate(button => new Promise(resolve => {
          const start = performance.now();
          button.click();
          let hiddenRows;
          const frame = () => {
            const card = document.querySelector('.picker-card');
            if (card && hiddenRows === undefined) {
              hiddenRows = [...card.querySelectorAll('.model-row')].filter(row => Number(getComputedStyle(row).opacity) < 1).length;
            }
            if (card && Number(getComputedStyle(card).opacity) >= 0.9) {
              requestAnimationFrame(() => resolve({ elapsed: performance.now() - start, hiddenRows }));
            } else requestAnimationFrame(frame);
          };
          requestAnimationFrame(frame);
        }));
        // Cached choices must arrive with the dialog rather than each waiting
        // for its own staggered entrance, including offscreen discoveries.
        expect(opening.hiddenRows).toBe(0);
        test.info().annotations.push({ type: 'click-to-visible-ms', description: String(opening.elapsed) });
        await expect(dialog.getByRole('searchbox', { name: 'Search models' })).toBeEnabled();
      }
      await openPromptly();
      const rail = dialog.getByRole('navigation', { name: 'Filter by provider' });
      await rail.getByRole('button', { name: /OpenRouter/ }).click();
      await page.route('**/__verenu_dev/invoke', route);
      await dialog.getByRole('button', { name: 'Refresh models', exact: true }).click();
      await expect.poll(() => requests).toBe(1);
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      await dialog.waitFor({ state: 'detached' });
      await openPromptly();
      await expect(dialog.getByRole('status')).toContainText('You can still choose a model');
      const search = dialog.getByRole('searchbox', { name: 'Search models' });
      await search.fill('synthetic/model-499');
      await expect(dialog.locator('.model-row')).toHaveCount(1);
      await expect(dialog.getByText('synthetic/model-499', { exact: true }).first()).toBeVisible();
      await search.fill('');
      await page.screenshot({ path: test.info().outputPath(`${task}-refresh-pending.png`) });
      release();
      await expect(dialog.getByRole('status')).toContainText('Refresh unavailable');
      await expect.poll(async () => (await session.invoke('get_setting', { key: 'provider_model_cache' })).openrouter.lastError).not.toBeNull();
      const failedAttempt = (await session.invoke('get_setting', { key: 'provider_model_cache' })).openrouter.lastAttemptAt;
      // The picker renders cached catalogs in batches; keep proving the full
      // cached list remains searchable after the provider refresh fails.
      await search.fill('synthetic/model-499');
      await expect(dialog.locator('.model-row')).toHaveCount(1);
      await expect(dialog.getByText('synthetic/model-499', { exact: true }).first()).toBeVisible();
      await search.fill('');
      await rail.getByRole('button', { name: /OpenRouter/ }).click();
      await dialog.getByRole('button', { name: 'Refresh models', exact: true }).click();
      await expect.poll(() => requests).toBe(2);
      // Choices remain interactive when viewing another provider during a refresh.
      await rail.getByRole('button', { name: /Synthetic picker endpoint/ }).click();
      await expect(dialog.getByRole('status')).not.toContainText('Refreshing');
      await dialog.locator('.row-main').filter({ hasText: 'synthetic-alternative' }).click();
      await expect(dialog).toHaveCount(0);
      await expect.poll(() => session.invoke('get_setting', { key: `${task}_default_model` })).toBe(`${provider.id}/synthetic-alternative`);
      release();
      await expect.poll(async () => (await session.invoke('get_setting', { key: 'provider_model_cache' })).openrouter.lastAttemptAt).toBeGreaterThan(failedAttempt);
      expect(await session.invoke('get_setting', { key: `${task}_default_model` })).toBe(`${provider.id}/synthetic-alternative`);
    } finally {
      release?.();
      await page.unroute('**/__verenu_dev/invoke', route);
      for (const key of ['custom_providers', `${task}_default_model`, `${task}_provider`, `${task}_model`, `${task}_models_by_provider`, 'model_selection_mode']) {
        if (previous[key] !== undefined && previous[key] !== null) await session.invoke('save_setting', { key, value: previous[key] });
      }
      await session.invoke('save_setting', { key: 'advanced_model_ui', value: previous.advanced_model_ui === true });
    }
    expect(await session.invoke('get_setting', { key: 'advanced_model_ui' })).toBe(previous.advanced_model_ui === true);
  });
}
