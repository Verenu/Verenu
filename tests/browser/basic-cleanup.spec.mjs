import { test, expect } from './fixtures.mjs';
import path from 'node:path';

test('Basic selection and independently opt-in commands persist without a prompt editor', async ({ page, session }, testInfo) => {
  const previous = await session.invoke('get_all_settings');
  // get_all_settings omits cleanup_intensity, so read it canonically.
  const previousIntensity = await session.invoke('get_setting', { key: 'cleanup_intensity' });
  try {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await session.invoke('save_setting', { key: 'voice_commands_enabled', value: false });
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    await expect(page.getByRole('switch', { name: 'Voice commands', exact: true })).toHaveCount(0);
    await page.locator('[data-debug-id="nav.settings"]').click();
    const commands = page.getByRole('switch', { name: 'Voice commands', exact: true });
    await expect(commands).toHaveAttribute('aria-checked', 'false');
    await commands.click();
    await expect(commands).toHaveAttribute('aria-checked', 'true');
    expect(await session.invoke('get_setting', { key: 'voice_commands_enabled' })).toBe(true);
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    const basic = page.getByRole('button', { name: /^Basic / });
    await basic.click();
    await expect(basic).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: 'Edit Basic cleanup prompt' })).toHaveCount(0);
    await expect(page.getByText('Tone and custom AI instructions do not apply', { exact: false })).toBeVisible();
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await expect(commands).toHaveAttribute('aria-checked', 'true');
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    await expect(basic).toHaveAttribute('aria-pressed', 'true');
    expect(await session.invoke('get_setting', { key: 'cleanup_intensity' })).toBe('rules');
    await page.locator('[data-debug-id="nav.settings"]').click();
    await commands.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(path.dirname(process.env.VERENU_SESSION_ACCESS_FILE), `basic-cleanup-${testInfo.project.name}.png`) });
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: previousIntensity ?? 'medium' });
    await session.invoke('save_setting', { key: 'voice_commands_enabled', value: previous.voice_commands_enabled ?? false });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: previous.cleanup_enabled ?? true });
  }
});

for (const reducedMotion of ['no-preference', 'reduce']) {
  test(`Voice commands details animate open and closed (${reducedMotion})`, async ({ page, session }) => {
    const previous = await session.invoke('get_all_settings');
    try {
      await page.emulateMedia({ reducedMotion });
      await page.reload();
      await page.locator('[data-debug-id="nav.settings"]').click();
      const more = page.getByRole('button', { name: 'What you can say' });
      const details = page.locator('#voice-commands-details');
      await expect(more).toHaveAttribute('aria-expanded', 'false');
      await expect(more).not.toHaveAttribute('aria-controls', /.+/);
      await expect(details).toHaveCount(0);
      // Svelte transitions run through Element.animate; record durations so
      // the check cannot race the animation finishing.
      await page.evaluate(() => {
        window.__voiceAnimations = [];
        const animate = Element.prototype.animate;
        Element.prototype.animate = function (...args) {
          if (this.id === 'voice-commands-details') window.__voiceAnimations.push(args[1]?.duration ?? args[1]);
          return animate.apply(this, args);
        };
      });
      const durations = () => page.evaluate(() => window.__voiceAnimations);
      await more.click();
      await expect(more).toHaveAttribute('aria-expanded', 'true');
      await expect(more).toHaveAttribute('aria-controls', 'voice-commands-details');
      // Svelte first creates a zero-duration delay animation, then starts the
      // slide in its finish callback. Wait for the real slide without relaxing
      // the duration contract.
      await expect.poll(durations).toContain(reducedMotion === 'reduce' ? 132 : 220);
      await expect(details).toBeVisible();
      await expect(details).toContainText('scratch that');
      await expect(details).toContainText('never edit text already in another app');
      const chevron = more.locator('svg');
      await expect(chevron).toHaveClass(/open/);
      await expect.poll(() => chevron.evaluate((el) => getComputedStyle(el).transform)).not.toBe('none');
      await more.press('Enter');
      await expect(more).toHaveAttribute('aria-expanded', 'false');
      await expect(details).toHaveCount(0);
    } finally {
      await page.emulateMedia({ reducedMotion: null });
      await session.invoke('save_setting', { key: 'voice_commands_enabled', value: previous.voice_commands_enabled ?? false });
    }
  });
}
