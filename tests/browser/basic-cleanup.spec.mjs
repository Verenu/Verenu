import { test, expect } from './fixtures.mjs';
import path from 'node:path';

test('Basic selection and independently opt-in commands persist without a prompt editor', async ({ page, session }, testInfo) => {
  const previous = await session.invoke('get_all_settings');
  try {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await session.invoke('save_setting', { key: 'voice_commands_enabled', value: false });
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    const commands = page.getByRole('switch', { name: 'Voice commands', exact: true });
    await expect(commands).toHaveAttribute('aria-checked', 'false');
    await commands.click();
    await expect(commands).toHaveAttribute('aria-checked', 'true');
    expect(await session.invoke('get_setting', { key: 'voice_commands_enabled' })).toBe(true);
    const basic = page.getByRole('button', { name: /^Basic / });
    await basic.click();
    await expect(basic).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: 'Edit Basic cleanup prompt' })).toHaveCount(0);
    await expect(page.getByText('Tone and custom AI instructions do not apply', { exact: false })).toBeVisible();
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    await expect(commands).toHaveAttribute('aria-checked', 'true');
    await expect(basic).toHaveAttribute('aria-pressed', 'true');
    expect(await session.invoke('get_setting', { key: 'cleanup_intensity' })).toBe('rules');
    await commands.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(path.dirname(process.env.VERENU_SESSION_ACCESS_FILE), `basic-cleanup-${testInfo.project.name}.png`) });
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: previous.cleanup_intensity ?? 'medium' });
    await session.invoke('save_setting', { key: 'voice_commands_enabled', value: previous.voice_commands_enabled ?? false });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: previous.cleanup_enabled ?? true });
  }
});
