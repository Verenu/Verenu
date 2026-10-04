import { test, expect } from './fixtures.mjs';

test('Context app and website removal icons stay centered on hover and focus', async ({ page, session }, testInfo) => {
  const context = await session.invoke('create_context', { name: 'Synthetic alignment', contextualFormattingDisabled: false });
  await session.invoke('assign_context_target', { contextId: context.id, executable: 'chatgpt.exe', appName: 'Chat GPT' });
  await session.invoke('assign_context_website', { contextId: context.id, domain: 'example.com' });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    const buttons = page.locator('.target-strip .target-chip button');
    await expect(buttons).toHaveCount(2);
    for (const button of await buttons.all()) {
      await button.hover();
      const measure = () => button.evaluate(b => {
        const box = b.getBoundingClientRect();
        const icon = b.querySelector('svg').getBoundingClientRect();
        return { width: box.width, height: box.height, dx: icon.x + icon.width / 2 - box.x - box.width / 2, dy: icon.y + icon.height / 2 - box.y - box.height / 2 };
      });
      await expect.poll(measure).toEqual({ width: 16, height: 16, dx: 0, dy: 0 });
      await button.focus();
      await expect(button).toBeFocused();
      expect(await measure()).toEqual({ width: 16, height: 16, dx: 0, dy: 0 });
    }
    await buttons.first().hover();
    await page.screenshot({ path: testInfo.outputPath('context-target-hover.png') });
    await buttons.first().click();
    await expect(buttons).toHaveCount(1);
    expect((await session.invoke('get_context_targets')).filter(t => t.context_id === context.id)).toHaveLength(0);
    await buttons.first().focus();
    await page.keyboard.press('Enter');
    await expect(buttons).toHaveCount(0);
    expect((await session.invoke('get_context_websites')).filter(t => t.context_id === context.id)).toHaveLength(0);
  } finally {
    await session.invoke('delete_context', { contextId: context.id });
  }
});
