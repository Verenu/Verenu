import { test, expect } from './fixtures.mjs';
import { defaultMicGain } from '../../src/lib/components/settings/audioGain.ts';

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
  const outer = await row.boundingBox();
  const padding = await row.evaluate(element => {
    const style = getComputedStyle(element);
    return { left: parseFloat(style.paddingLeft), right: parseFloat(style.paddingRight) };
  });
  const content = outer && { x: outer.x + padding.left, width: outer.width - padding.left - padding.right };
  const bounds = await advice.boundingBox();
  expect(content, 'Audio row has rendered bounds').not.toBeNull();
  expect(bounds, 'Advice has rendered bounds').not.toBeNull();
  expect(Math.abs(bounds.x - content.x), 'Warning aligns with the Audio content left edge').toBeLessThanOrEqual(1);
  expect(Math.abs(bounds.x + bounds.width - content.x - content.width), 'Warning fills the Audio content right edge').toBeLessThanOrEqual(1);
  expect(await advice.evaluate(element => element.scrollWidth <= element.clientWidth + 1), 'Advice wraps within its border').toBe(true);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(page.viewportSize().width);
}

test('a delayed gain save preserves a newer agent change', async ({ page, session }) => {
  const initial = await session.invoke('get_setting', { key: 'mic_gain' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const pending = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'mic_gain', value: 2 });
    await page.reload();
    await openAudio(page);
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_setting_if_unchanged' || request.args?.key !== 'mic_gain') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    const gain = page.getByRole('slider', { name: 'Microphone gain', exact: true });
    await gain.fill('3');
    await pending;
    await session.invoke('save_setting', { key: 'mic_gain', value: 4 });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['mic_gain']));
    release();
    await expect.poll(() => session.invoke('get_setting', { key: 'mic_gain' })).toBe(4);
    await expect(page.getByRole('alert').filter({ hasText: 'Could not save microphone gain' })).toBeVisible();
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'mic_gain', value: initial ?? defaultMicGain(false) });
  }
});

test('high-gain advice fills the Audio content width for both noise reduction states', async ({ page, session }, testInfo) => {
  const initial = await session.invoke('get_all_settings');
  // Browser sessions run a desktop Rust worker, including the phone viewport.
  const initialGain = initial.mic_gain ?? defaultMicGain(false);
  await openAudio(page);
  const gain = page.getByRole('slider', { name: 'Microphone gain', exact: true });
  await expect(gain).toHaveValue(String(initialGain));
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
