import { test, expect } from './fixtures.mjs';

test('API key actions never expose the inactive label', async ({ page, session, cachedCatalogs }, testInfo) => {
  const status = await session.invoke('get_api_key_status');
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.keys"]').click();
  const rows = page.locator('.key-row');
  await expect(rows).toHaveCount(6);

  for (const row of await rows.all()) {
    const provider = (await row.getAttribute('data-setting-target')).replace('api-key-', '');
    const save = row.locator('.flip-face.front');
    const clear = row.locator('.flip-face.back');
    const input = row.locator('.key-input');
    const saved = status[provider] === true;
    await expect(saved ? clear : save).toBeVisible();
    await expect(saved ? save : clear).toBeHidden();
    await expect(saved ? clear : save).toHaveAttribute('aria-hidden', 'false');
    await expect(save).toBeDisabled();

    // Public synthetic draft only. Never save it or change native credentials.
    await input.fill('synthetic-draft');
    await expect(save).toBeVisible();
    await expect(save).toBeEnabled();
    await expect(clear).toBeHidden();
    await expect(clear).toHaveAttribute('tabindex', '-1');
    await input.press('Tab');
    await expect(save).toBeFocused();
    await input.fill('');
    await expect(saved ? clear : save).toBeVisible();
    await expect(saved ? save : clear).toBeHidden();
    const box = await (saved ? clear : save).boundingBox();
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize().width);
  }

  await page.screenshot({ path: testInfo.outputPath('api-keys.png') });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(page.locator('.flip-face[aria-hidden="true"]:visible')).toHaveCount(0);
});
