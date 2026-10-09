import { test, expect } from './fixtures.mjs';

test('native device eligibility controls Apple picker options without replacing saved selection', async ({ page, session }) => {
  const availability = await session.invoke('get_apple_intelligence_availability');
  const supported = ['available', 'intelligence-disabled', 'model-not-ready'].includes(availability.state);
  const saved = await session.invoke('get_all_settings');
  try {
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });
    await session.invoke('save_setting', { key: 'cleanup_default_model', value: 'apple-intelligence/system' });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await page.getByRole('button', { name: 'Change model', exact: true }).last().click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    if (supported) await expect(dialog).toContainText('Apple Intelligence');
    else await expect(dialog.getByText('Apple Intelligence', { exact: true })).toHaveCount(0);
    await dialog.getByRole('searchbox', { name: 'Search models' }).fill('apple');
    if (supported) await expect(dialog).toContainText('Apple Intelligence');
    else await expect(dialog.getByText('Apple Intelligence', { exact: true })).toHaveCount(0);
    expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).toBe('apple-intelligence/system');
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_default_model', value: saved.cleanup_default_model });
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: saved.advanced_model_ui ?? false });
  }
});
