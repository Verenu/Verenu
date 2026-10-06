import { test, expect } from './fixtures.mjs';

for (const preference of ['no-preference', 'reduce']) {
  test(`Context dialog motion settles and remains operable with ${preference}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: preference });
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    const motion = await dialog.evaluate(element => element.getAnimations({ subtree: true }).map(animation => animation.effect.getTiming().duration));
    if (preference === 'no-preference') expect(motion.some(duration => duration > 0)).toBe(true);
    // The design system reduces 220 ms modal motion to 132 ms, preserving
    // orientation feedback rather than removing the transition entirely.
    else expect(motion.every(duration => duration <= 132)).toBe(true);
    await expect.poll(() => dialog.evaluate(element => element.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length)).toBe(0);
    await dialog.getByLabel('Context group name').fill('Motion regression');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'New context group', exact: true })).toBeEnabled();
  });
}
