import { test, expect } from './fixtures.mjs';

test('theme editor previews, stays open during navigation, cancels and saves', async ({ page, session }, testInfo) => {
  const keys = ['appearance_mode', 'custom_theme', 'accent_color', 'custom_themes'];
  const previous = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  const expectLauncherClearOfDock = async () => {
    const launcher = await page.getByRole('button', { name: 'Dev tests', exact: true }).boundingBox();
    const dock = await page.locator('.theme-dock').boundingBox();
    expect(launcher && dock).toBeTruthy();
    expect(launcher.x + launcher.width <= dock.x || dock.x + dock.width <= launcher.x || launcher.y + launcher.height <= dock.y || dock.y + dock.height <= launcher.y).toBe(true);
  };
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.getByRole('radio', { name: 'Dark', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'appearance_mode' })).toBe('dark');
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    const editor = page.getByRole('group', { name: 'Create theme', exact: true });
    await editor.getByRole('textbox', { name: 'Background hex value' }).fill('182633');
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--paper').trim().toLowerCase())).toBe('#182633');
    expect(await session.invoke('get_setting', { key: 'custom_theme' })).toEqual(previous.custom_theme);
    await page.locator('[data-debug-id="nav.back"]').click();
    await expect(page.locator('.settings-page')).toHaveCount(0);
    await expect(editor).toBeVisible();
    await expectLauncherClearOfDock();
    if (testInfo.project.name === 'phone') {
      await page.setViewportSize({ width: 390, height: 600 });
      await expectLauncherClearOfDock();
      await page.setViewportSize({ width: 390, height: 844 });
    }
    await editor.getByRole('button', { name: 'Close theme editor' }).click();
    await editor.getByRole('button', { name: 'Keep editing', exact: true }).click();
    await expect(editor).toBeVisible();
    await editor.getByRole('button', { name: 'Close theme editor' }).click();
    await editor.getByRole('button', { name: 'Discard', exact: true }).click();
    await expect(editor).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe('dark');
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    await editor.getByRole('textbox', { name: 'Name', exact: true }).fill(`Synthetic ${testInfo.project.name}`);
    await editor.getByRole('textbox', { name: 'Background hex value' }).fill('182633');
    await editor.getByRole('textbox', { name: 'Accent hex value' }).fill('5CA6E8');
    const rect = await editor.boundingBox();
    const viewport = page.viewportSize();
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.width).toBeLessThanOrEqual(viewport.width);
    expect(rect.y + rect.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({ path: testInfo.outputPath('theme-editor.png') });
    const saveTheme = editor.getByRole('button', { name: 'Save theme', exact: true });
    await saveTheme.scrollIntoViewIfNeeded();
    await expectLauncherClearOfDock();
    expect(await saveTheme.evaluate((button) => {
      const rect = button.getBoundingClientRect();
      return button.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    })).toBe(true);
    await saveTheme.click();
    await expect(editor).toHaveCount(0);
    const saved = await session.invoke('get_setting', { key: 'custom_themes' });
    const theme = saved.find(t => t.name === `Synthetic ${testInfo.project.name}`);
    expect(theme.palette.background.toLowerCase()).toBe('#182633');
    expect(theme.accent.toLowerCase()).toBe('#5ca6e8');
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await expect(page.getByRole('button', { name: theme.name, exact: true })).toHaveAttribute('aria-pressed', 'true');
    await page.getByRole('button', { name: `Edit ${theme.name}`, exact: true }).click();
    const edit = page.getByRole('group', { name: 'Edit theme', exact: true });
    await expect(edit.getByRole('textbox', { name: 'Background hex value' })).toHaveValue(/182633/i);
    await edit.getByRole('button', { name: 'Close theme editor' }).click();
    await page.getByRole('radio', { name: 'System', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'appearance_mode' })).toBe('system');
    await page.getByRole('button', { name: 'Tokyo Night', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'appearance_mode' })).toBe('custom');
  } finally {
    for (const key of ['custom_themes', 'custom_theme', 'accent_color', 'appearance_mode']) {
      await session.invoke('save_setting', { key, value: previous[key] ?? (key === 'appearance_mode' ? 'system' : null) });
    }
  }
});
