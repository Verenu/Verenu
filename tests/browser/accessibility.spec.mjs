import AxeBuilder from '@axe-core/playwright';
import { test, expect } from './fixtures.mjs';

test.use({ reducedMotion: 'reduce', colorScheme: 'light' });

test('Context and settings accessibility does not regress against reviewed contrast debt', async ({ page, session, cachedCatalogs }) => {
  await session.invoke('save_setting', { key: 'appearance_mode', value: 'light' });
  await page.reload();
  const audit = async (view, include) => {
    // Navigation also animates ancestors of the audited panel. Sampling during
    // their fade changes effective text colors and creates false contrast debt.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await expect.poll(() => page.evaluate(() => document.getAnimations().filter(animation => animation.playState === 'running' && Number.isFinite(animation.effect.getComputedTiming().endTime)).length)).toBe(0);
    let builder = new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']);
    if (include) builder = builder.include(include);
    const result = await builder.analyze();
    expect(result.violations.filter(row => row.id !== 'color-contrast').map(row => ({ rule: row.id, targets: row.nodes.map(node => node.target) }))).toEqual([]);
    // Existing contrast debt is explicit, reviewed and versioned. New affected
    // elements or worse ratios fail; removing debt requires reviewing this file.
    const contrast = result.violations.filter(row => row.id === 'color-contrast').flatMap(row => row.nodes.map(node => ({ target: node.target, checks: node.any.map(check => check.data) }))).sort((a, b) => JSON.stringify(a.target).localeCompare(JSON.stringify(b.target)));
    expect(JSON.stringify(contrast, null, 2)).toMatchSnapshot(`${view}-contrast.json`);
  };
  await page.getByRole('button', { name: 'New context group', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'New context group', exact: true })).toBeVisible();
  await audit('context', '[role="dialog"]');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await page.locator('[data-debug-id="nav.settings"]').click();
  await audit('general', '.settings-page');
  await page.locator('[data-debug-id="settings.privacy"]').click();
  await audit('privacy', '.settings-page');
});
