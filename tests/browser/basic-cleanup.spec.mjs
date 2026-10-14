import { test, expect } from './fixtures.mjs';
import path from 'node:path';

test('Context hints locate the independent voice commands control in Settings General', async ({ page }) => {
  await page.getByRole('button', { name: 'New context group', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
  await expect(dialog.locator('.field-hint').filter({ hasText: 'Voice commands' })).toContainText('Settings > General');
  const cleanup = dialog.locator('.field-col').filter({ hasText: 'Cleanup' }).getByRole('button');
  await cleanup.click();
  await page.getByRole('option', { name: 'Basic', exact: true }).click();
  const hint = dialog.getByRole('note');
  await expect(hint).toContainText('Voice commands are a separate global option in Settings > General.');
  await hint.scrollIntoViewIfNeeded();
  await expect(hint).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.locator('[data-debug-id="nav.settings"]').click();
  await expect(page.getByRole('switch', { name: 'Voice commands', exact: true })).toBeVisible();
});

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

test('Context cleanup hint follows the effective cleanup when Use default is selected', async ({ page, session }) => {
  const previousIntensity = await session.invoke('get_setting', { key: 'cleanup_intensity' });
  const name = `Inherited hint ${Date.now()}`;
  let created;
  const basicNote = 'Basic cleanup runs on this device and ignores the tone and custom instructions above.';
  const aiHint = 'Sent to the AI cleanup model for this context.';
  const openNew = async () => {
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    return page.getByRole('dialog', { name: 'New context group', exact: true });
  };
  const choose = async (dialog, option) => {
    await dialog.locator('.field-col').filter({ hasText: 'Cleanup' }).getByRole('button').click();
    await page.getByRole('option', { name: option, exact: true }).click();
  };
  const expectBasic = async (dialog, inherited) => {
    const note = dialog.getByRole('note');
    await expect(note).toContainText(basicNote);
    if (inherited) await expect(note).toContainText('Your default cleanup is Basic.');
    else await expect(note).not.toContainText('Your default cleanup is Basic.');
    await expect(dialog.getByText(aiHint)).toHaveCount(0);
  };
  const expectAi = async (dialog) => {
    await expect(dialog.getByText(aiHint)).toBeVisible();
    await expect(dialog.getByRole('note')).toHaveCount(0);
  };
  try {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'rules' });
    await page.reload();
    let dialog = await openNew();
    await expectBasic(dialog, true);
    await choose(dialog, 'Light');
    await expectAi(dialog);
    await choose(dialog, 'Use default');
    await expectBasic(dialog, true);
    await choose(dialog, 'Basic');
    await expectBasic(dialog, false);
    await choose(dialog, 'Use default');
    await dialog.getByLabel('Context group name').fill(name);
    await dialog.getByRole('button', { name: 'Create context group', exact: true }).click();
    await expect(dialog).toBeHidden();
    created = (await session.invoke('get_contexts')).find((row) => row.name === name);
    expect(created).toBeTruthy();
    // Inheriting stays inheriting: the saved override is null, not 'rules'.
    expect(created.cleanup_intensity ?? null).toBeNull();

    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'medium' });
    await page.reload();
    dialog = await openNew();
    await expectAi(dialog);
    await choose(dialog, 'Basic');
    await expectBasic(dialog, false);
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
  } finally {
    if (created) await session.invoke('delete_context', { contextId: created.id });
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: previousIntensity ?? 'medium' });
    await page.reload();
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
