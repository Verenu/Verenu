import { test, expect } from './fixtures.mjs';

async function openAudio(page) {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.advanced"]:visible, button.settings-tab:visible').filter({ hasText: 'Audio' }).click();
  await expect(page.getByRole('slider', { name: 'Microphone gain', exact: true })).toBeVisible();
}

async function expectFullWidthAdvice(page) {
  const row = page.locator('[data-setting-target="audio-gain"]');
  const advice = row.getByRole('note');
  await expect(advice).toBeVisible();
  await advice.evaluate(async (element) => {
    const panel = element.closest('.panel-inner') ?? element;
    await Promise.all(panel.getAnimations({ subtree: true })
      .map(animation => animation.finished.catch(() => {})));
  });
  const content = await row.boundingBox();
  const bounds = await advice.boundingBox();
  expect(content, 'Audio row has rendered bounds').not.toBeNull();
  expect(bounds, 'Advice has rendered bounds').not.toBeNull();
  expect(Math.abs(bounds.x - content.x), 'Warning aligns with the Audio content left edge').toBeLessThanOrEqual(1);
  expect(Math.abs(bounds.x + bounds.width - content.x - content.width), 'Warning fills the Audio content right edge').toBeLessThanOrEqual(1);
  expect(await advice.evaluate(element => element.scrollWidth <= element.clientWidth + 1), 'Advice wraps within its border').toBe(true);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(page.viewportSize().width);
}

test('high-gain advice fills the Audio content width for both noise reduction states', async ({ page, session }, testInfo) => {
  const initial = await session.invoke('get_all_settings');
  await openAudio(page);
  const gain = page.getByRole('slider', { name: 'Microphone gain', exact: true });
  const initialGain = Number(await gain.inputValue());
  try {
    await session.invoke('save_setting', { key: 'mic_gain', value: 8 });
    await session.invoke('save_setting', { key: 'noise_reduction', value: false });
    await page.reload();
    await openAudio(page);
    await expect(gain).toHaveValue('8');
    await expect(page.getByRole('note')).toContainText('Turn on Noise reduction');
    await expectFullWidthAdvice(page);
    await page.screenshot({ path: testInfo.outputPath('audio-warning-noise-off.png') });

    await page.getByRole('switch', { name: 'Noise reduction', exact: true }).click();
    await expect(page.getByRole('note')).toContainText('Raise the input volume');
    await expectFullWidthAdvice(page);
    await page.screenshot({ path: testInfo.outputPath('audio-warning-noise-on.png') });
    await expect(gain).toHaveValue('8');

    await gain.focus();
    await page.keyboard.press('Home');
    await page.keyboard.press('Tab');
    await expect(gain).toHaveValue('1');
    await expect(page.getByRole('note')).toHaveCount(0);
    await expect.poll(() => session.invoke('get_setting', { key: 'mic_gain' })).toBe(1);
    await page.reload();
    await openAudio(page);
    await expect(gain).toHaveValue('1');
    await expect(page.getByRole('note')).toHaveCount(0);
  } finally {
    await session.invoke('save_setting', { key: 'mic_gain', value: initialGain });
    await session.invoke('save_setting', { key: 'noise_reduction', value: initial.noise_reduction ?? true });
  }
});
