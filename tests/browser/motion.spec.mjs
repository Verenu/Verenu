import { test, expect } from './fixtures.mjs';

for (const preference of ['no-preference', 'reduce']) {
  test(`Context dialog motion settles and remains operable with ${preference}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: preference });
    await page.evaluate(() => {
      const animate = Element.prototype.animate;
      window.__dialogMotion = [];
      const capture = (element, animation) => {
        if (element.closest('[role="dialog"]')) window.__dialogMotion.push(animation.effect.getTiming().duration);
      };
      Element.prototype.animate = function (...args) {
        const animation = animate.apply(this, args);
        capture(this, animation);
        return animation;
      };
      for (const event of ['animationstart', 'transitionrun']) {
        document.addEventListener(event, ({ target }) => {
          for (const animation of target.getAnimations()) capture(target, animation);
        }, true);
      }
    });
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    await expect.poll(() => dialog.evaluate(element => element.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length)).toBe(0);
    const motion = await page.evaluate(() => window.__dialogMotion);
    expect(motion.length).toBeGreaterThan(0);
    if (preference === 'no-preference') expect(motion.some(duration => duration > 0)).toBe(true);
    else expect(motion.every(duration => duration <= 132)).toBe(true);
    await dialog.getByLabel('Context group name').fill('Motion regression');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'New context group', exact: true })).toBeEnabled();
  });
}
