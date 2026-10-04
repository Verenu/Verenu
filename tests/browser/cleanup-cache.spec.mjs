import { test, expect } from './fixtures.mjs';

test('invalid cache preferences fail without changing persisted settings', async ({ session }) => {
  const before = await session.invoke('get_setting', { key: 'cleanup_cache_enabled' });
  const response = await fetch(new URL('/__verenu_dev/invoke', session.access.localAccessUrl), {
    method: 'POST', headers: { Authorization: `Bearer ${session.access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command: 'save_setting', args: { key: 'cleanup_cache_enabled', value: 'false' } }),
  });
  expect(response.ok).toBeFalsy();
  expect(await session.invoke('get_setting', { key: 'cleanup_cache_enabled' })).toBe(before);
});

test('cleanup caching can be disabled, reloaded, enabled, and cleared', async ({ page, session }) => {
  const initial = (await session.invoke('get_setting', { key: 'cleanup_cache_enabled' })) ?? true;
  try {
    await session.invoke('save_setting', { key: 'cleanup_cache_enabled', value: true });
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.privacy"]').click();
    const toggle = page.getByRole('switch', { name: 'Cleanup cache', exact: true });
    const storage = page.locator('[data-setting-target="privacy-cache-storage"]');
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await expect(storage).toContainText('of text.');
    await expect(storage).not.toContainText('GB free');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await expect(storage).toContainText('Caching disabled and stored results cleared.');
    const status = await session.invoke('get_cleanup_cache_status');
    expect(status.entry_count).toBe(0);
    expect(status.payload_bytes).toBe(0);
    expect(status.session.hits).toBeGreaterThanOrEqual(0);
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.privacy"]').click();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await page.getByRole('button', { name: 'Clear Cache', exact: true }).click();
    await expect(storage).toContainText('Cache cleared.');
    await expect(storage).toContainText('0 cached results');
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_cache_enabled', value: initial });
  }
});
