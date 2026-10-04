import { test, expect } from './fixtures.mjs';

test.use({ deviceScaleFactor: 1.25, colorScheme: 'dark' });

test('context group add icon stays centered and opens creation', async ({ page }, testInfo) => {
  const button = page.locator('.ctx-add');
  await expect(button).toBeVisible();
  const expectCentered = async () => {
    const bounds = await button.boundingBox();
    const icon = await button.locator('svg').boundingBox();
    expect(Math.abs(icon.x + icon.width / 2 - bounds.x - bounds.width / 2)).toBeLessThan(0.5);
    expect(Math.abs(icon.y + icon.height / 2 - bounds.y - bounds.height / 2)).toBeLessThan(0.5);
  };
  await expectCentered();
  await button.hover();
  await expectCentered();
  await button.evaluate(async (element) => {
    await new Promise(requestAnimationFrame);
    await Promise.all(element.getAnimations().map((animation) => animation.finished));
  });
  await page.screenshot({ path: testInfo.outputPath('context-plus-hover.png') });
  await button.screenshot({ path: testInfo.outputPath('context-plus-detail.png'), scale: 'css' });
  await button.focus();
  await expect(button).toBeFocused();
  await expectCentered();
  await button.press('Enter');
  await expect(page.getByRole('dialog', { name: 'New context group', exact: true })).toBeVisible();
});
