import { test, expect } from './fixtures.mjs';

test('privacy and prompt settings no longer offer app context hints', async ({ page, session }) => {
  const settings = await session.invoke('get_all_settings');
  expect(settings).not.toHaveProperty('app_context_hint');
  const prompt = await session.invoke('get_default_cleanup_prompt');
  expect(prompt).not.toContain('{{ active_app }}');
  expect(prompt).not.toContain('target_context');
  expect(prompt).toContain('uncertainty');
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('.settings-nav-item', { hasText: 'Privacy' }).click();
  await expect(page.getByRole('switch', { name: 'Allow Verenu service checks', exact: true })).toBeVisible();
  await expect(page.getByRole('switch', { name: 'App context hint', exact: true })).toHaveCount(0);
  await expect(page.locator('[data-setting-target="privacy-context"]')).toHaveCount(0);
  if (page.viewportSize().width > 600) {
    await page.getByRole('searchbox', { name: 'Search settings' }).fill('App context hint');
    await expect(page.getByText('No matching settings', { exact: true })).toBeVisible();
    await expect(page.locator('.settings-search-result')).toHaveCount(0);
  }
});
