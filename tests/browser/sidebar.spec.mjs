import { test, expect } from './fixtures.mjs';

test.use({ deviceScaleFactor: 1.25, colorScheme: 'dark' });

test('context group add icon stays centered and opens creation', async ({ page }, testInfo) => {
  const button = page.locator('.ctx-add');
  await expect(button).toBeVisible();
  const expectCentered = async () => {
    const bounds = await button.boundingBox();
    const icon = await button.locator('svg').boundingBox();
    expect(Math.abs(icon.x + icon.width / 2 - bounds.x - bounds.width / 2)).toBeLessThan(0.5);
    expect(Math.abs(icon.y + icon.height / 2 - bounds.y - bounds.height / 2)).toBeLessThan(0.5);
  };
  await expectCentered();
  await button.hover();
  await expectCentered();
  await button.evaluate(async (element) => {
    await new Promise(requestAnimationFrame);
    await Promise.all(element.getAnimations().map((animation) => animation.finished));
  });
  await page.screenshot({ path: testInfo.outputPath('context-plus-hover.png') });
  await button.screenshot({ path: testInfo.outputPath('context-plus-detail.png'), scale: 'css' });
  await button.focus();
  await expect(button).toBeFocused();
  await expectCentered();
  await button.press('Enter');
  await expect(page.getByRole('dialog', { name: 'New context group', exact: true })).toBeVisible();
});

test('pinned context rows stay one line with their app icons beside the name', async ({ page, session }, testInfo) => {
  const pinned = await session.invoke('create_context', { name: 'Synthetic pinned row', contextualFormattingDisabled: false });
  const plain = await session.invoke('create_context', { name: 'Synthetic plain row', contextualFormattingDisabled: false });
  try {
    await session.invoke('assign_context_target', { contextId: pinned.id, executable: 'chatgpt.exe', appName: 'Chat GPT' });
    await session.invoke('assign_context_website', { contextId: pinned.id, domain: 'example.com' });
    await session.invoke('set_context_pinned', { contextId: pinned.id, pinned: true });
    await page.reload();
    const pinnedRow = page.locator(`[data-debug-id="context.${pinned.id}"]`);
    const plainRow = page.locator(`[data-debug-id="context.${plain.id}"]`);
    if (testInfo.project.name !== 'desktop') {
      await expect(pinnedRow).toBeAttached();
      return;
    }
    await expect(pinnedRow.locator('.ctx-stack-item')).toHaveCount(2);
    const pinnedBox = await pinnedRow.boundingBox();
    const plainBox = await plainRow.boundingBox();
    expect(pinnedBox.height).toBeCloseTo(plainBox.height, 0);
    const name = await pinnedRow.locator('.ctx-name').boundingBox();
    const stack = await pinnedRow.locator('.ctx-stack').boundingBox();
    expect(stack.x).toBeGreaterThanOrEqual(name.x + name.width);
    expect(Math.abs(stack.y + stack.height / 2 - (name.y + name.height / 2))).toBeLessThan(1);
    await expect(page.locator('.ctx-age, .ctx-pin')).toHaveCount(0);
    await pinnedRow.hover();
    const kebab = page.getByRole('button', { name: 'More actions for Synthetic pinned row', exact: true });
    await expect(kebab).toBeVisible();
    const kebabBox = await kebab.boundingBox();
    expect(kebabBox.x).toBeGreaterThanOrEqual(stack.x + stack.width - 0.5);
    await page.screenshot({ path: testInfo.outputPath('context-rows.png'), clip: { x: 0, y: 0, width: 420, height: 520 } });
  } finally {
    await session.invoke('delete_context', { contextId: pinned.id });
    await session.invoke('delete_context', { contextId: plain.id });
  }
});
