import { test, expect } from './fixtures.mjs';

async function openSettings(page, section) {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator(`[data-debug-id="settings.${section}"]`).click();
}

test('Audio values roll their digits and advanced fades disclose smoothly', async ({ page, session }) => {
  test.skip(process.platform !== 'linux', 'The volume control is Linux only');
  const previous = await session.invoke('get_setting', { key: 'duck_target' });
  try {
    await openSettings(page, 'advanced');
    const slider = page.getByRole('slider', { name: 'Volume of audio while dictating' });
    await slider.fill('40');
    const value = page.locator('.sound-volume-value').first();
    await expect(value.locator('.rolling-sr')).toHaveText('40%');
    await expect(value.locator('.rolling-strip').first()).toHaveCSS('transition-property', 'transform');

    const fades = page.getByRole('button', { name: 'Advanced volume fades' });
    await expect(fades).toHaveAttribute('aria-expanded', 'false');
    await fades.click();
    await expect(fades).toHaveAttribute('aria-expanded', 'true');
    await expect(fades).toHaveAttribute('aria-controls', 'volume-fades-details');
    await expect(page.locator('#volume-fades-details').getByText('Fade down duration')).toBeVisible();
    await fades.click();
    await expect(page.locator('#volume-fades-details')).toHaveCount(0);
  } finally {
    await session.invoke('save_setting', { key: 'duck_target', value: previous ?? 100 });
  }
});

test('Switching Settings pages never shows the old and new page together', async ({ page }) => {
  await openSettings(page, 'general');
  await expect(page.locator('.settings-page .panel')).toHaveCount(1);
  await page.locator('[data-debug-id="settings.text"]').click();
  const overlap = await page.evaluate(async () => {
    const deadline = performance.now() + 900;
    while (performance.now() < deadline) {
      const visible = [...document.querySelectorAll('.settings-page .panel')]
        .filter((panel) => Number(getComputedStyle(panel).opacity) > 0.05);
      if (visible.length > 1) return true;
      await new Promise((resolve) => requestAnimationFrame(resolve));
    }
    return false;
  });
  expect(overlap).toBe(false);
});

test('Integrations render as cards like the other Settings pages', async ({ page }) => {
  await openSettings(page, 'integrations');
  const integration = page.locator('.integration').first();
  await expect(integration).toBeVisible();
  await expect(integration).toHaveCSS('border-top-left-radius', /^(1[4-9]|[2-9]\d)px$/);
});
