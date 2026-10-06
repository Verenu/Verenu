import { test, expect } from './fixtures.mjs';

test.use({ reducedMotion: 'reduce', colorScheme: 'light' });

test('Context dialog and privacy settings retain their reviewed appearance', async ({ page, session, cachedCatalogs }) => {
  const original = await session.invoke('get_setting', { key: 'appearance_mode' });
  try {
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'light' });
    await page.reload();
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expect(dialog).toHaveScreenshot('new-context.png', { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.002 });
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.privacy"]').click();
    const privacy = page.locator('.settings-page .panel:visible').last();
    await expect(privacy.getByRole('switch', { name: 'Cleanup cache', exact: true })).toBeVisible();
    await expect(privacy).toHaveScreenshot('privacy.png', { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.002 });
  } finally { await session.invoke('save_setting', { key: 'appearance_mode', value: original ?? 'system' }); }
});
