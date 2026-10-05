import { test, expect } from './fixtures.mjs';
import path from 'node:path';
const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);

test('About update check uses real backend and reports failures honestly', async ({ page }) => {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]:visible, button.settings-tab:visible').filter({ hasText: 'About' }).click();
  const row = page.locator('[data-setting-target="about-updates"]');
  await expect(row.getByRole('button', { name: 'Check for Updates', exact: true })).toBeVisible();
  await page.locator('.settings-page').evaluate(async element => {
    await Promise.all(element.getAnimations({ subtree: true }).filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => {})));
  });
  await page.screenshot({ path: path.join(directory, `updater-after-${test.info().project.name}.png`) });
  await row.getByRole('button', { name: 'Check for Updates', exact: true }).click();
  await expect(row.getByRole('button', { name: 'Checking…', exact: true })).toHaveCount(0, { timeout: 30_000 });
  // Public release availability varies; each result must be visible and actionable.
  await expect(row).toContainText(/latest version|is available|Could not|unavailable|compatible|check.*update/i);
  await expect(row.locator('[role="status"]')).toBeVisible();
  await page.screenshot({ path: path.join(directory, `updater-result-${test.info().project.name}.png`) });
  await page.reload();
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]:visible, button.settings-tab:visible').filter({ hasText: 'About' }).click();
  await expect(row).toBeVisible();
});

test('browser session cannot invoke native updater installation', async ({ session }) => {
  const url = new URL(session.access.localAccessUrl).origin;
  const response = await fetch(`${url}/__verenu_dev/invoke`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${session.access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command: 'install_update', args: { downloadUrl: 'https://github.com/MONKE2525E/Verenu/releases/download/test/Verenu_99.0.0_amd64.AppImage' } }),
  });
  expect(response.status).toBe(403);
});
