import { test, expect } from './fixtures.mjs';

test('deleting the last sidebar cleanup override refreshes Home readiness', async ({ page, session, readySpeech }, testInfo) => {
  const originalViewport = page.viewportSize();
  const compactViewport = (originalViewport?.width ?? 0) < 700;

  const previous = await session.invoke('get_all_settings');
  const cleanupSettings = {
    cleanup_enabled: true,
    cleanup_intensity: 'none',
    dual_transcription_enabled: false,
    cleanup_default_model: 'local/readiness-missing-cleanup',
    cleanup_fallback_models: [],
  };
  const cleanupRestoreDefaults = {
    cleanup_enabled: false,
    cleanup_intensity: 'medium',
    dual_transcription_enabled: false,
    cleanup_default_model: null,
    cleanup_fallback_models: [],
  };
  const name = `Ready ${Date.now()}`;
  let context;

  try {
    for (const [key, value] of Object.entries(cleanupSettings)) {
      await session.invoke('save_setting', { key, value });
    }
    context = await session.invoke('create_context', {
      name,
      cleanupIntensity: 'light',
      contextualFormattingDisabled: false,
    });
    await page.reload();

    const notice = page.locator('.readiness-notice');
    await expect(page.getByRole('heading', { name: /^Hold .* to dictate/ })).toBeVisible();
    await expect(notice).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`home-unassigned-context-${compactViewport ? 'phone' : 'desktop'}.png`) });
    await session.invoke('assign_context_website', { contextId: context.id, domain: 'readiness.example' });
    await page.evaluate(() => window.dispatchEvent(new Event('verenu:context-saved')));
    await expect(notice).toContainText('Optional cleanup model readiness-missing-cleanup is not installed.');
    await session.invoke('remove_context_website', { contextId: context.id, domain: 'readiness.example' });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-data-changed', { tables: ['context_website_targets'] });
    });
    await expect(notice).toHaveCount(0);
    await expect(page.getByRole('heading', { name: /^Hold .* to dictate/ })).toBeVisible();
    await session.invoke('assign_context_website', { contextId: context.id, domain: 'readiness.example' });
    await page.evaluate(() => window.dispatchEvent(new Event('verenu:context-saved')));
    await expect(notice).toContainText('Optional cleanup model readiness-missing-cleanup is not installed.');
    await page.screenshot({ path: testInfo.outputPath(`home-cleanup-override-before-delete-${compactViewport ? 'phone' : 'desktop'}.png`) });

    // The phone layout has no sidebar context menu. Keep the real phone state
    // asserted, resize only to perform the same Rust-backed deletion, then
    // return to phone and verify the cleared readiness state there as well.
    if (compactViewport && originalViewport) {
      await page.setViewportSize({ width: 1320, height: originalViewport.height });
      await expect(notice).toContainText('Optional cleanup model readiness-missing-cleanup is not installed.');
    }

    await page.getByRole('button', { name: `More actions for ${name}`, exact: true }).click();
    await page.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();

    await expect.poll(async () => (await session.invoke('get_contexts')).some(row => row.id === context.id)).toBe(false);
    await expect(notice).toHaveCount(0);
    if (compactViewport && originalViewport) await page.setViewportSize(originalViewport);
    await expect(page.getByRole('heading', { name: /^Hold .* to dictate/ })).toBeVisible();
    await expect(notice).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`home-cleanup-override-after-delete-${compactViewport ? 'phone' : 'desktop'}.png`) });
  } finally {
    if (context && (await session.invoke('get_contexts')).some(row => row.id === context.id)) {
      await session.invoke('delete_context', { contextId: context.id });
    }
    for (const key of Object.keys(cleanupSettings)) {
      const value = previous[key] ?? cleanupRestoreDefaults[key];
      await session.invoke('save_setting', { key, value });
    }
    if (compactViewport && originalViewport) await page.setViewportSize(originalViewport);
    await page.reload();
  }
});
