import { test, expect } from './fixtures.mjs';

for (const preference of ['no-preference', 'reduce']) {
  test(`Context dialog motion settles and remains operable with ${preference}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: preference });
    const motion = await page.evaluate(async () => {
      [...document.querySelectorAll('button')].find(button => button.getAttribute('aria-label') === 'New context group').click();
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const dialog = document.querySelector('[role="dialog"]');
      if (!dialog) throw new Error('Required Context dialog did not mount');
      return dialog.getAnimations({ subtree: true }).map(animation => animation.effect.getTiming().duration);
    });
    if (preference === 'no-preference') expect(motion.some(duration => duration > 0)).toBe(true);
    // The design system reduces 220 ms modal motion to 132 ms, preserving
    // orientation feedback rather than removing the transition entirely.
    else expect(motion.every(duration => duration <= 132)).toBe(true);
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    await expect.poll(() => dialog.evaluate(element => element.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length)).toBe(0);
    await dialog.getByLabel('Context group name').fill('Motion regression');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'New context group', exact: true })).toBeEnabled();
  });
}
