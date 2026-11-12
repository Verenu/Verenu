import { test, expect } from './fixtures.mjs';

test.use({ reducedMotion: 'reduce', colorScheme: 'dark' });

const settledBox = async (locator) => {
  await expect(locator).toBeVisible();
  let previous = null;
  await expect.poll(async () => {
    const box = await locator.boundingBox();
    const same = previous && Math.abs(previous.x - box.x) < 0.01 && Math.abs(previous.y - box.y) < 0.01;
    previous = box;
    return same;
  }).toBe(true);
  return previous;
};

test('Home, Insights, Style, and a context group start their title at the same place', async ({ page }, testInfo) => {
  const titles = {};
  titles.home = await settledBox(page.locator('.page-wrapper[data-page="home"] h1'));
  await page.locator('[data-debug-id="nav.insights"]').click();
  titles.insights = await settledBox(page.locator('.page-wrapper[data-page="insights"] h1'));
  await page.locator('[data-debug-id="nav.style"]').click();
  titles.style = await settledBox(page.locator('.page-wrapper[data-page="style"] h1'));
  await page.locator('.ctx-row').first().click();
  titles.context = await settledBox(page.locator('.context-header h2'));
  await page.screenshot({ path: testInfo.outputPath('context-page.png') });
  for (const [name, box] of Object.entries(titles)) {
    expect(Math.abs(box.x - titles.home.x), `${name} title left edge`).toBeLessThan(1);
    expect(Math.abs(box.y - titles.home.y), `${name} title top edge`).toBeLessThan(1);
  }
});

test('Style marks exactly one cleanup level and one tone as selected', async ({ page, session }, testInfo) => {
  const original = await session.invoke('get_setting', { key: 'cleanup_intensity' });
  try {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'medium' });
    await page.reload();
    await page.locator('[data-debug-id="nav.style"]').click();
    const grids = page.locator('.style-grid');
    await expect(grids).toHaveCount(2);
    const dot = (card) => card.locator('.style-radio').evaluate((el) => getComputedStyle(el, '::after').opacity);
    for (const grid of await grids.all()) {
      await expect(grid.locator('.style-card[aria-pressed="true"]')).toHaveCount(1);
      await expect.poll(() => dot(grid.locator('.style-card[aria-pressed="true"]'))).toBe('1');
      expect(await dot(grid.locator('.style-card[aria-pressed="false"]').first())).toBe('0');
    }
    const cleanup = grids.first();
    await expect(cleanup.locator('.style-card[aria-pressed="true"] .style-card-title')).toHaveText('Medium');
    await cleanup.getByRole('button', { name: /^Light/ }).click();
    await expect(cleanup.locator('.style-card[aria-pressed="true"] .style-card-title')).toHaveText('Light');
    await expect(cleanup.locator('.style-card[aria-pressed="true"]')).toHaveCount(1);
    await expect.poll(() => session.invoke('get_setting', { key: 'cleanup_intensity' })).toBe('light');
    await page.screenshot({ path: testInfo.outputPath('style.png') });
  } finally { await session.invoke('save_setting', { key: 'cleanup_intensity', value: original ?? 'medium' }); }
});

test('an empty context group keeps search and add on one row above one centered prompt', async ({ page, session }, testInfo) => {
  const context = await session.invoke('create_context', { name: 'Synthetic empty group', contextualFormattingDisabled: false });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    const search = page.locator('.library-search');
    const add = page.getByRole('button', { name: '+ Term', exact: true });
    const searchBox = await settledBox(search);
    const addBox = await add.boundingBox();
    expect(Math.abs(searchBox.y + searchBox.height / 2 - addBox.y - addBox.height / 2), 'search and add share a row').toBeLessThan(2);
    expect(addBox.x).toBeGreaterThan(searchBox.x + searchBox.width);
    const empty = page.locator('.section-empty');
    await expect(empty.getByText('No vocabulary in this context group')).toBeVisible();
    const prompt = await empty.getByRole('button', { name: 'Add term', exact: true }).boundingBox();
    const column = await page.locator('.tab-panel').boundingBox();
    expect(Math.abs(prompt.x + prompt.width / 2 - column.x - column.width / 2), 'empty prompt is centered').toBeLessThan(1);
    // The apps strip offers its three add actions by name even though the labels are short.
    for (const name of ['Add app', 'Add website', 'Add sub-app']) {
      await expect(page.locator('.target-strip').getByRole('button', { name, exact: true })).toBeVisible();
    }
    await page.screenshot({ path: testInfo.outputPath('empty-context.png') });
    await empty.getByRole('button', { name: 'Add term', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Add term', exact: true })).toBeVisible();
  } finally { await session.invoke('delete_context', { contextId: context.id }); }
});
