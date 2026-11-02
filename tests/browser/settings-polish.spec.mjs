import { test, expect } from './fixtures.mjs';

async function openSettings(page, section) {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator(`[data-debug-id="settings.${section}"]`).click();
}

test('the About footer is removed when another section is selected', async ({ page }) => {
  await openSettings(page, 'privacy');
  await expect(page.getByRole('heading', { name: 'Privacy', exact: true })).toBeVisible();
  await page.locator('[data-debug-id="settings.about"]').click();
  await expect(page.getByRole('heading', { name: 'About', exact: true })).toBeVisible();
  await expect(page.locator('.settings-foot')).toBeVisible();
  await page.locator('[data-debug-id="settings.privacy"]').click();
  const state = await page.evaluate(() => ({
    selected: document.querySelector('.settings-nav-item.active')?.getAttribute('data-debug-id'),
    footers: document.querySelectorAll('.settings-foot').length,
  }));
  expect(state).toEqual({ selected: 'settings.privacy', footers: 0 });
});

test('Audio values roll their digits and advanced fades disclose smoothly', async ({ page, session }) => {
  test.skip(process.platform !== 'linux', 'The volume control is Linux only');
  const previous = await session.invoke('get_setting', { key: 'duck_target' });
  try {
    await session.invoke('save_setting', { key: 'duck_target', value: 40 });
    await openSettings(page, 'advanced');
    const slider = page.getByRole('slider', { name: 'Volume of audio while dictating' });
    const value = page.locator('.sound-volume-value').first();
    await expect(value.locator('.rolling-sr')).toHaveText('40%');
    await slider.fill('73');
    await expect(value.locator('.rolling-sr')).toHaveText('73%');
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
