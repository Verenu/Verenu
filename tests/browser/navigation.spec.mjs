import { test, expect } from './fixtures.mjs';

// Delay delivery of the real module, without substituting its response or IPC.
async function holdModule(page, filename) {
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let requested;
  const started = new Promise(resolve => { requested = resolve; });
  await page.route(`**/${filename}.svelte*`, async route => {
    requested();
    await held;
    await route.continue();
  });
  return { started, release };
}

test('slow page imports retain the visible page and cancel superseded navigation', async ({ page }) => {
  const home = page.locator('h1.page-h').filter({ hasText: 'Welcome back' });
  await expect(home).toBeVisible();
  const held = await holdModule(page, 'Insights');
  try {
    await page.locator('[data-debug-id="nav.insights"]').click();
    await held.started;
    await expect(page.getByText('Loading page…', { exact: true })).toBeVisible();
    await expect(home).toBeVisible();
    await expect(page.locator('.page-wrapper')).toHaveCount(1);
    await expect.poll(() => home.evaluate(el => getComputedStyle(el.closest('.page-wrapper')).opacity)).toBe('1');
    await page.locator('[data-debug-id="nav.home"]').click();
    held.release();
    await expect(page.getByText('Loading page…', { exact: true })).toHaveCount(0);
    await expect(home).toBeVisible();
    await page.waitForTimeout(500);
    await expect(page.locator('.page-wrapper')).toHaveCount(1);
    await expect(page.locator('.page-wrapper h1')).toContainText('Welcome back');
    await page.locator('[data-debug-id="nav.insights"]').click();
    await expect(page.locator('.page-wrapper h1').filter({ hasText: 'Insights' })).toBeVisible();
    for (let i = 0; i < 4; i++) {
      await page.locator('[data-debug-id="nav.home"]').click();
      await page.locator('[data-debug-id="nav.insights"]').click();
    }
    await expect(page.locator('.page-wrapper')).toHaveCount(1);
    await expect(page.locator('.page-wrapper h1')).toHaveText('Insights');
  } finally { held.release(); }
});

test('Settings sections stay lazy until Settings opens', async ({ page }) => {
  let requests = 0;
  await page.route('**/GeneralSection.svelte*', async route => {
    requests++;
    await route.continue();
  });
  await page.reload();
  await expect(page.locator('h1.page-h').filter({ hasText: 'Welcome back' })).toBeVisible();
  expect(requests).toBe(0);
  await page.locator('[data-debug-id="nav.settings"]').click();
  await expect(page.getByRole('switch', { name: 'Legacy pages', exact: true })).toBeVisible();
  expect(requests).toBeGreaterThan(0);
});

test('slow Settings tabs retain the previous panel, then swap once', async ({ page }) => {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await expect(page.getByRole('switch', { name: 'Legacy pages', exact: true })).toBeVisible();
  const held = await holdModule(page, 'ApiKeysSection');
  try {
    await page.locator('[data-debug-id="settings.keys"]').click();
    await held.started;
    await expect(page.getByText('Loading settings…', { exact: true })).toBeVisible();
    await expect(page.getByRole('switch', { name: 'Legacy pages', exact: true, includeHidden: true })).toBeVisible();
    await expect(page.locator('.panel')).toHaveCount(1);
    await expect(page.locator('.panel-inner')).toHaveAttribute('inert', '');
    await expect.poll(() => page.locator('.panel').evaluate(el => getComputedStyle(el).opacity)).toBe('1');
    held.release();
    await expect(page.locator('.panel h2').filter({ hasText: 'Providers' })).toBeVisible();
    await expect(page.locator('.panel')).toHaveCount(1);
    await expect(page.getByText('Loading settings…', { exact: true })).toHaveCount(0);
    await expect(page.locator('.panel-inner')).not.toHaveAttribute('inert', '');
  } finally { held.release(); }
});

test('a failed page import shows recovery and navigation remains usable', async ({ page }) => {
  await page.route('**/Insights.svelte*', route => route.abort());
  await page.locator('[data-debug-id="nav.insights"]').click();
  await expect(page.getByRole('alert').filter({ hasText: "Couldn't load this view." })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reload app', exact: true })).toBeVisible();
  await page.locator('[data-debug-id="nav.home"]').click();
  await expect(page.locator('h1.page-h').filter({ hasText: 'Welcome back' })).toBeVisible();
});

test('slow Context data fills the same tab panel without replaying its entrance', async ({ page }) => {
  let release;
  const held = new Promise(resolve => { release = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    if (['get_context_dictionary', 'get_context_snippets'].includes(route.request().postDataJSON().command)) await held;
    await route.continue();
  });
  try {
    await page.locator('.ctx-row').filter({ hasText: 'Everywhere' }).click();
    await expect(page.getByText('Loading this context group…')).toBeVisible();
    await expect(page.locator('.tab-panel')).toHaveCount(1);
    await page.waitForTimeout(600);
    const panel = await page.locator('.tab-panel').elementHandle();
    release();
    await expect(page.getByText('Loading this context group…')).toHaveCount(0);
    expect(await panel.evaluate(el => el.isConnected)).toBe(true);
    await expect.poll(() => panel.evaluate(el => getComputedStyle(el).opacity)).toBe('1');
    await expect.poll(() => panel.evaluate(el => el.getAnimations().filter(animation => animation.playState === 'running').length)).toBe(0);
    for (let i = 0; i < 4; i++) {
      await page.getByRole('tab', { name: /Snippets/ }).click();
      await page.getByRole('tab', { name: /Vocabulary/ }).click();
    }
    await expect(page.locator('.tab-panel')).toHaveCount(1);
    await expect(page.locator('.tab-panel')).toHaveAttribute('id', 'context-panel-vocabulary');
  } finally { release(); }
});
