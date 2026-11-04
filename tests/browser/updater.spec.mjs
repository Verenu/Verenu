import { test, expect } from './fixtures.mjs';
import path from 'node:path';
const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);

test('About source opens the organization repository', async ({ page }) => {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]:visible, button.settings-tab:visible').filter({ hasText: 'About' }).click();
  const source = page.locator('[data-setting-target="about-source"]').getByRole('button', { name: 'github.com/Verenu/Verenu', exact: true });
  await expect(source).toBeVisible();
  const popupPromise = page.waitForEvent('popup');
  await source.click();
  const popup = await popupPromise;
  await expect(popup).toHaveURL('https://github.com/Verenu/Verenu');
  await popup.close();
});

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
  const checking = row.getByRole('button', { name: 'Checking…', exact: true });
  await expect(checking).toBeVisible({ timeout: 10_000 });
  await expect(checking).toHaveCount(0, { timeout: 30_000 });
  const status = row.locator('[role="status"]');
  await expect(status).toHaveText(/\S/, { timeout: 30_000 });
  await expect(status).toContainText(/latest version|is available|Could not check|unavailable|compatible/i, { timeout: 30_000 });
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
    body: JSON.stringify({ command: 'install_update', args: { downloadUrl: 'https://github.com/Verenu/Verenu/releases/download/test/Verenu_99.0.0_amd64.AppImage' } }),
  });
  expect(response.status).toBe(403);
});

test('an installed update asks the user to quit and reopen', async ({ page }) => {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]:visible, button.settings-tab:visible').filter({ hasText: 'About' }).click();
  await page.evaluate(async () => {
    const { appStore } = await import('/src/lib/stores.svelte.ts');
    appStore.updateInfo = { version: '99.0.0', assetName: 'Verenu_99.0.0_amd64.AppImage', downloadUrl: 'https://example.invalid/synthetic.AppImage', installMode: 'install' };
    appStore.updateInstalled = true;
    appStore.updateProgress = 'Update installed. Quit and reopen Verenu to finish.';
  });
  const row = page.locator('[data-setting-target="about-updates"]');
  await expect(row.getByText('Update installed. Quit and reopen Verenu to finish.')).toBeVisible();
  await expect(row.getByRole('button', { name: 'Installed', exact: true })).toBeDisabled();
});

test('a late update check cannot hide the installed update', async ({ page }) => {
  let releaseCheck;
  let checkStarted;
  let checkDelivered;
  const pendingCheck = new Promise(resolve => { releaseCheck = resolve; });
  const checkArrived = new Promise(resolve => { checkStarted = resolve; });
  const checkReturned = new Promise(resolve => { checkDelivered = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    const command = route.request().postDataJSON()?.command;
    if (command === 'check_for_update') {
      checkStarted();
      await pendingCheck;
      await route.fulfill({ status: 200, json: null });
      checkDelivered();
      return;
    }
    if (command === 'install_update') return route.fulfill({ status: 200, json: 'installed' });
    return route.fallback();
  });
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]').click();
  await page.evaluate(async () => {
    const { appStore } = await import('/src/lib/stores.svelte.ts');
    appStore.updateInfo = { version: '99.0.0', assetName: 'Verenu_99.0.0_amd64.AppImage', downloadUrl: 'https://example.invalid/synthetic.AppImage', installMode: 'install' };
  });
  const row = page.locator('[data-setting-target="about-updates"]');
  await row.getByRole('button', { name: 'Check for Updates', exact: true }).click();
  await checkArrived;
  await page.locator('[data-debug-id="nav.back"]').click();
  await page.locator('.update-btn').click();
  await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.updateInstalled)).toBe(true);
  releaseCheck();
  await checkReturned;
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(page.locator('.update-btn')).toHaveText('Installed');
  await expect(page.locator('.update-btn')).toBeDisabled();
  await expect(page.locator('.update-banner')).toContainText('Quit and reopen');
});

test('a late dismissal cannot hide the installed update', async ({ page }) => {
  let releaseDismissal;
  let dismissalStarted;
  let dismissalDelivered;
  const pendingDismissal = new Promise(resolve => { releaseDismissal = resolve; });
  const dismissalArrived = new Promise(resolve => { dismissalStarted = resolve; });
  const dismissalReturned = new Promise(resolve => { dismissalDelivered = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    const { command, args } = route.request().postDataJSON() ?? {};
    if (command === 'save_setting' && args?.key === 'update_dismissed_version' && args?.value === '99.0.0') {
      dismissalStarted();
      await pendingDismissal;
      await route.fulfill({ status: 200, json: null });
      dismissalDelivered();
      return;
    }
    if (command === 'install_update') return route.fulfill({ status: 200, json: 'installed' });
    return route.fallback();
  });
  await page.evaluate(async () => {
    const { appStore } = await import('/src/lib/stores.svelte.ts');
    appStore.updateInfo = { version: '99.0.0', assetName: 'Verenu_99.0.0_amd64.AppImage', downloadUrl: 'https://example.invalid/synthetic.AppImage', installMode: 'install' };
  });
  const banner = page.locator('.update-banner');
  await banner.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await dismissalArrived;
  await banner.locator('.update-btn').click();
  await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.updateInstalled)).toBe(true);
  releaseDismissal();
  await dismissalReturned;
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(banner.getByRole('button', { name: 'Installed', exact: true })).toBeDisabled();
  await expect(banner).toContainText('Quit and reopen');
});

test('a pending beta setting change blocks installation until the offer is refreshed', async ({ page }) => {
  let releaseSave;
  let saveStarted;
  let saveReturned;
  let notificationSaved;
  let installs = 0;
  const pendingSave = new Promise(resolve => { releaseSave = resolve; });
  const saveArrived = new Promise(resolve => { saveStarted = resolve; });
  const returned = new Promise(resolve => { saveReturned = resolve; });
  const notification = new Promise(resolve => { notificationSaved = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    const { command, args } = route.request().postDataJSON() ?? {};
    if (command === 'get_setting' && args?.key === 'beta_updates_enabled') {
      return route.fulfill({ status: 200, json: false });
    }
    if (command === 'save_setting' && args?.key === 'beta_updates_enabled') {
      saveStarted();
      await pendingSave;
      await route.fulfill({ status: 200, json: null });
      saveReturned();
      return;
    }
    if (command === 'save_setting' && args?.key === 'update_notified_version') {
      await route.fulfill({ status: 200, json: null });
      notificationSaved();
      return;
    }
    if (command === 'install_update') {
      installs++;
      return route.fulfill({ status: 200, json: 'installed' });
    }
    if (command === 'check_for_update') return route.fulfill({ status: 200, json: null });
    return route.fallback();
  });
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.about"]').click();
    await page.evaluate(async () => {
      const { appStore } = await import('/src/lib/stores.svelte.ts');
      appStore.updateInfo = { version: '99.0.0', assetName: 'Verenu_99.0.0_amd64.AppImage', downloadUrl: 'https://example.invalid/synthetic.AppImage', installMode: 'install' };
    });
    await page.getByRole('switch', { name: 'Beta updates' }).click();
    await page.getByRole('button', { name: 'Enable beta updates' }).click();
    await saveArrived;
    await page.locator('[data-debug-id="nav.back"]').click();
    await expect(page.locator('.update-btn')).toBeDisabled();
    expect(installs).toBe(0);
    releaseSave();
    await returned;
    await notification;
    await expect(page.locator('.update-banner')).toHaveCount(0);
    expect(installs).toBe(0);
  } finally {
    releaseSave();
  }
});

test('a failed beta save restores the toggle after navigating away and back', async ({ page }) => {
  let releaseSave;
  let saveStarted;
  const pending = new Promise(resolve => { releaseSave = resolve; });
  const started = new Promise(resolve => { saveStarted = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    const { command, args } = route.request().postDataJSON() ?? {};
    if (command === 'get_setting' && args?.key === 'beta_updates_enabled') return route.fulfill({ status: 200, json: false });
    if (command === 'save_setting' && args?.key === 'beta_updates_enabled') {
      saveStarted();
      await pending;
      return route.fulfill({ status: 500, json: { error: 'Synthetic setting failure' } });
    }
    return route.fallback();
  });
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.about"]').click();
    await page.getByRole('switch', { name: 'Beta updates' }).click();
    await page.getByRole('button', { name: 'Enable beta updates' }).click();
    await started;
    await page.locator('[data-debug-id="nav.back"]').click();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.about"]').click();
    const toggle = page.getByRole('switch', { name: 'Beta updates' });
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await expect(toggle).toBeDisabled();
    releaseSave();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    await expect(toggle).toBeEnabled();
  } finally {
    releaseSave();
  }
});

test('developer reinstall survives navigation and asks the user to reopen', async ({ page }) => {
  let installs = 0;
  let restarts = 0;
  let completeInstall;
  const installPending = new Promise(resolve => { completeInstall = resolve; });
  await page.route('**/__verenu_dev/invoke', async route => {
    const command = route.request().postDataJSON()?.command;
    if (command === 'reinstall_latest_update') {
      installs++;
      await installPending;
      return route.fulfill({ status: 200, json: { version: '0.21.0', outcome: 'installed' } });
    }
    if (command === 'restart_app') {
      restarts++;
      return route.fulfill({ status: 500, json: { error: 'Synthetic replacement failure' } });
    }
    return route.fallback();
  });
  await page.evaluate(async () => {
    const { appStore } = await import('/src/lib/stores.svelte.ts');
    appStore.updateInfo = { version: '99.0.0', assetName: 'Verenu_99.0.0_amd64.AppImage', downloadUrl: 'https://example.invalid/synthetic.AppImage', installMode: 'install' };
  });
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.about"]').click();
  const version = page.locator('button.version-tap');
  await expect(version).toBeVisible();
  for (let tap = 0; tap < 10; tap++) await version.click();
  await page.locator('[data-debug-id="settings.developer"]').click();
  await page.getByRole('tab', { name: 'Developer Settings' }).click();
  const button = page.locator('[data-setting-target="developer-installer"]');
  await button.click();
  await expect(button).toBeDisabled();
  await page.getByRole('tab', { name: 'Overview', exact: true }).click();
  await page.getByRole('tab', { name: 'Developer Settings' }).click();
  await expect(button).toBeDisabled();
  completeInstall();
  await expect(button).toBeDisabled();
  await expect(page.getByRole('status').filter({ hasText: 'installed' })).toContainText('Quit and reopen');
  await page.getByRole('tab', { name: 'Overview', exact: true }).click();
  await page.getByRole('tab', { name: 'Developer Settings' }).click();
  await expect(button).toHaveText('Reinstall latest');
  await expect(button).toBeDisabled();
  await expect(page.getByRole('status').filter({ hasText: 'installed' })).toContainText('Quit and reopen');
  await page.locator('[data-debug-id="nav.back"]').click();
  await expect(page.locator('.update-btn')).toHaveText('Installed');
  await expect(page.locator('.update-btn')).toBeDisabled();
  expect(restarts).toBe(0);
  expect(installs).toBe(1);
});
