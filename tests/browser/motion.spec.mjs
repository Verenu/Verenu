import { test, expect } from './fixtures.mjs';

for (const preference of ['no-preference', 'reduce']) {
  test(`Context dialog motion settles and remains operable with ${preference}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: preference });
    await page.evaluate(() => {
      window.__contextDialogMotion = [];
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (...args) {
        const animation = animate.apply(this, args);
        if (this.matches('.context-modal')) window.__contextDialogMotion.push(animation.effect.getTiming().duration);
        return animation;
      };
    });
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    const motion = () => page.evaluate(() => window.__contextDialogMotion);
    await expect.poll(async () => (await motion()).some(duration => duration > 0)).toBe(true);
    if (preference === 'reduce') expect((await motion()).every(duration => duration <= 132)).toBe(true);
    await expect.poll(() => dialog.evaluate(element => element.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length)).toBe(0);
    await dialog.getByLabel('Context group name').fill('Motion regression');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'New context group', exact: true })).toBeEnabled();
  });
}
