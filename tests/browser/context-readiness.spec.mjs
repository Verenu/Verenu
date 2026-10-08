import { test, expect } from './fixtures.mjs';

test('deleting the last sidebar cleanup override refreshes Home readiness', async ({ page, session, readySpeech }, testInfo) => {
  test.skip((page.viewportSize()?.width ?? 0) < 700, 'The compact phone navigation has no sidebar context menu.');

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
    await expect(notice).toContainText('Optional cleanup model readiness-missing-cleanup is not installed.');
    await page.screenshot({ path: testInfo.outputPath('home-cleanup-override-before-delete.png') });

    await page.getByRole('button', { name: `More actions for ${name}`, exact: true }).click();
    await page.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();

    await expect.poll(async () => (await session.invoke('get_contexts')).some(row => row.id === context.id)).toBe(false);
    await expect(notice).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath('home-cleanup-override-after-delete.png') });
  } finally {
    if (context && (await session.invoke('get_contexts')).some(row => row.id === context.id)) {
      await session.invoke('delete_context', { contextId: context.id });
    }
    for (const key of Object.keys(cleanupSettings)) {
      const value = previous[key] ?? cleanupRestoreDefaults[key];
      await session.invoke('save_setting', { key, value });
    }
    await page.reload();
  }
});
