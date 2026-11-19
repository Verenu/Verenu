import { test, expect } from './fixtures.mjs';

async function openAppearance(page) {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.appearance"]').click();
  await expect(page.getByRole('heading', { name: 'Appearance', level: 2 })).toBeVisible();
}

test('theme editor previews, stays open during navigation, cancels and saves', async ({ page, session }, testInfo) => {
  const keys = ['appearance_mode', 'custom_theme', 'accent_color', 'custom_themes'];
  const previous = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  try {
    await openAppearance(page);
    await page.getByRole('radio', { name: 'Dark', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'appearance_mode' })).toBe('dark');
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    const editor = page.getByRole('group', { name: 'Create theme', exact: true });
    await editor.getByRole('textbox', { name: 'Background hex value' }).fill('182633');
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--paper').trim().toLowerCase())).toBe('#182633');
    expect(await session.invoke('get_setting', { key: 'custom_theme' })).toEqual(previous.custom_theme);
    await page.locator('[data-debug-id="nav.back"]').click();
    await expect(editor).toBeVisible();
    await editor.getByRole('button', { name: 'Close theme editor' }).click();
    await editor.getByRole('button', { name: 'Keep editing', exact: true }).click();
    await expect(editor).toBeVisible();
    await editor.getByRole('button', { name: 'Close theme editor' }).click();
    await editor.getByRole('button', { name: 'Discard', exact: true }).click();
    await expect(editor).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).toBe('dark');
    await openAppearance(page);
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
    await editor.getByRole('button', { name: 'Save theme', exact: true }).click();
    await expect(editor).toHaveCount(0);
    const saved = await session.invoke('get_setting', { key: 'custom_themes' });
    const theme = saved.find(t => t.name === `Synthetic ${testInfo.project.name}`);
    expect(theme.palette.background.toLowerCase()).toBe('#182633');
    expect(theme.accent.toLowerCase()).toBe('#5ca6e8');
    await page.reload();
    await openAppearance(page);
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

test('appearance tab saves accent and font, filters themes, and General keeps a quick mode switch', async ({ page, session }) => {
  const keys = ['appearance_mode', 'accent_color', 'interface_font'];
  const previous = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  try {
    await openAppearance(page);
    await page.getByRole('radio', { name: 'Violet', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'accent_color' })).toBe('#8B6FD6');
    await expect(page.getByRole('radio', { name: 'Violet', exact: true })).toHaveAttribute('aria-checked', 'true');

    await page.getByRole('radio', { name: 'Mono', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'interface_font' })).toBe('mono');
    await expect(page.locator('html')).toHaveAttribute('data-font', 'mono');

    await page.getByRole('radio', { name: 'Light themes' }).click();
    await expect(page.getByRole('button', { name: 'Solarized Light', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Tokyo Night', exact: true })).toHaveCount(0);

    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-font', 'mono');

    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.general"]').click();
    const quick = page.locator('[data-setting-target="general-appearance"]');
    await quick.getByRole('radio', { name: 'Light', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'appearance_mode' })).toBe('light');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await quick.getByRole('button', { name: 'Appearance', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Appearance', level: 2 })).toBeVisible();

    await page.getByRole('button', { name: 'Reset', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'interface_font' })).toBe('default');
    expect(await session.invoke('get_setting', { key: 'appearance_mode' })).toBe('system');
    expect(await session.invoke('get_setting', { key: 'accent_color' })).toBeNull();
    await expect(page.locator('html')).not.toHaveAttribute('data-font', /./);
  } finally {
    for (const key of keys) {
      await session.invoke('save_setting', { key, value: previous[key] ?? (key === 'appearance_mode' ? 'system' : key === 'interface_font' ? 'default' : null) });
    }
  }
});

test('Reset waits for an in-flight font save instead of half resetting', async ({ page, session }) => {
  const keys = ['appearance_mode', 'accent_color', 'interface_font'];
  const previous = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const saving = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'interface_font', value: 'default' });
    await openAppearance(page);
    await page.getByRole('radio', { name: 'Violet', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'accent_color' })).toBe('#8B6FD6');
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_setting' || request.args?.key !== 'interface_font') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await page.getByRole('radio', { name: 'Mono', exact: true }).click();
    await saving;
    await expect(page.getByRole('button', { name: 'Reset', exact: true })).toHaveAttribute('aria-disabled', 'true');
    // aria-disabled keeps it focusable, so dispatch the click to prove the handler itself refuses.
    await page.getByRole('button', { name: 'Reset', exact: true }).dispatchEvent('click');
    release();
    await expect.poll(() => session.invoke('get_setting', { key: 'interface_font' })).toBe('mono');
    expect(await session.invoke('get_setting', { key: 'accent_color' })).toBe('#8B6FD6');
    await expect(page.getByRole('button', { name: 'Reset', exact: true })).toHaveAttribute('aria-disabled', 'false');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    for (const key of keys) {
      await session.invoke('save_setting', { key, value: previous[key] ?? (key === 'appearance_mode' ? 'system' : key === 'interface_font' ? 'default' : null) });
    }
  }
});
