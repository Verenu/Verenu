import { test, expect } from './fixtures.mjs';

const BUILTIN = { groq: 'Groq', openai: 'OpenAI', google: 'Gemini', assemblyai: 'AssemblyAI', openrouter: 'OpenRouter', xai: 'xAI' };

test('Providers lists only added providers and adds new ones from one flyout', async ({ page, session, cachedCatalogs }, testInfo) => {
  const status = await session.invoke('get_api_key_status');
  const settings = await session.invoke('get_all_settings');
  const saved = Object.keys(BUILTIN).filter(id => status[id] === true);
  const unsaved = Object.keys(BUILTIN).filter(id => status[id] !== true);
  const custom = (settings.custom_providers ?? []).length;

  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.keys"]').click();
  await expect(page.locator('[data-setting-target^="api-key-"]')).toHaveCount(saved.length + custom);
  for (const id of unsaved) await expect(page.locator(`[data-setting-target="api-key-${id}"]`)).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('providers-list.png') });

  await page.getByRole('button', { name: 'Add provider', exact: true }).first().click();
  const picker = page.getByRole('group', { name: 'Choose a provider' });
  await expect(picker).toBeVisible();
  for (const id of saved) await expect(picker.getByRole('button', { name: new RegExp(`^${BUILTIN[id]}`) })).toHaveCount(0);
  for (const id of unsaved) await expect(picker.getByRole('button', { name: new RegExp(`^${BUILTIN[id]}`) })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('providers-picker.png') });

  if (unsaved.length) {
    await picker.getByRole('button', { name: new RegExp(`^${BUILTIN[unsaved[0]]}`) }).click();
    const form = page.getByRole('form', { name: 'Provider key form' });
    const add = form.getByRole('button', { name: 'Add provider', exact: true });
    await expect(add).toBeDisabled();
    await form.getByLabel(`${BUILTIN[unsaved[0]]} API key`).fill('synthetic-draft');
    await expect(add).toBeEnabled();
    await page.screenshot({ path: testInfo.outputPath('providers-key-form.png') });
    await page.getByRole('button', { name: 'Back to providers' }).click();
    await expect(picker).toBeVisible();
  }

  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
});
