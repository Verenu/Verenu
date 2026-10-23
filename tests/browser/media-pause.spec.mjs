import { test, expect } from './fixtures.mjs';

test('Linux exposes media pause separately from the persistent mute preference', async ({ page, session }, testInfo) => {
  const previous = await session.invoke('get_all_settings');
  try {
    await session.invoke('save_setting', { key: 'mute_audio', value: false });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.advanced"]').click();
    const pause = page.getByRole('switch', { name: 'Pause media while dictating', exact: true });
    await expect(pause).toBeVisible();
    await expect(page.locator('[data-setting-target="audio-pause-media"]')).toContainText('MPRIS');
    await expect(pause).toHaveAttribute('aria-checked', String(previous.pause_media_during_dictation ?? false));
    expect((await session.invoke('get_all_settings')).mute_audio).toBe(false);
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.advanced"]').click();
    await expect(pause).toHaveAttribute('aria-checked', String(previous.pause_media_during_dictation ?? false));
    expect((await session.invoke('get_all_settings')).mute_audio).toBe(false);
    await pause.scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath('linux-media-pause.png'), animations: 'disabled' });
  } finally {
    await session.invoke('save_setting', { key: 'mute_audio', value: previous.mute_audio ?? false });
  }
});
