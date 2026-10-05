import { test, expect } from './fixtures.mjs';

async function expectMenuInsideViewport(menu, page) {
  const bounds = await menu.boundingBox();
  const viewport = page.viewportSize();
  const content = await page.locator('.settings-page .panel:visible').last().boundingBox();
  expect(bounds).not.toBeNull();
  expect(content).not.toBeNull();
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width);
  expect(bounds.y).toBeGreaterThanOrEqual(content.y);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(content.y + content.height);
  expect(bounds.x).toBeGreaterThanOrEqual(content.x);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(content.x + content.width);
}

async function expectMenuOptionsNotClipped(menu) {
  const hasClippedText = await menu.evaluate((element) =>
    [...element.querySelectorAll('.models-dropdown-item')]
      .some((option) => option.scrollWidth > option.clientWidth + 1),
  );
  expect(hasClippedText).toBe(false);
}

async function waitForPanelMotion(target) {
  await expect(target).toBeVisible();
  await target.evaluate(async (element) => {
    const panel = element.closest('.panel-inner') ?? element;
    const animations = panel.getAnimations({ subtree: true });
    await Promise.all(animations.map((animation) => animation.finished.catch(() => {})));
  });
}

test('privacy and model choice menus use compact styling and stay in the viewport', async ({ page, session }) => {
  const initialAdvanced = await session.invoke('get_setting', { key: 'advanced_model_ui' });
  await session.invoke('save_setting', { key: 'advanced_model_ui', value: true });

  try {
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();

    await page.locator('[data-debug-id="settings.privacy"]:visible, button.settings-tab:visible').filter({ hasText: 'Privacy' }).click();
    const retention = page.getByRole('button', { name: 'Transcription history retention' });
    await waitForPanelMotion(retention);
    await expect(retention).toHaveClass(/ui-dropdown-trigger--compact/);
    await expect(retention).not.toHaveClass(/btn-ghost/);

    await retention.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('option', { name: '30 days' })).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(page.getByRole('option', { name: '90 days' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(retention).toBeFocused();

    await retention.evaluate((element) => element.scrollIntoView({ block: 'center', inline: 'nearest' }));
    await waitForPanelMotion(retention);
    await retention.click();

    const retentionMenu = page.locator('#history-retention-menu');
    await expect(retentionMenu).toBeVisible();
    await expect(retentionMenu).toHaveClass(/ui-dropdown-menu--padded/);
    await waitForPanelMotion(retentionMenu);
    await expectMenuInsideViewport(retentionMenu, page);
    await page.keyboard.press('Escape');

    await page.locator('[data-debug-id="settings.models"]:visible, button.settings-tab:visible').filter({ hasText: 'Models' }).click();
    const strategy = page.getByRole('button', { name: 'Transcription strategy' });
    await waitForPanelMotion(strategy);
    await expect(strategy).toHaveClass(/ui-dropdown-trigger--compact/);
    await expect(strategy).not.toHaveClass(/btn-ghost/);
    await strategy.evaluate((element) => element.scrollIntoView({ block: 'center', inline: 'nearest' }));
    await waitForPanelMotion(strategy);
    await strategy.click();

    const strategyMenu = page.locator('#transcription-mode-menu');
    await expect(strategyMenu).toBeVisible();
    await expect(strategyMenu).toHaveClass(/ui-dropdown-menu--padded/);
    await waitForPanelMotion(strategyMenu);
    await expectMenuInsideViewport(strategyMenu, page);
    const strategyMenuWidth = await strategyMenu.evaluate((element) => element.getBoundingClientRect().width);
    expect(strategyMenuWidth).toBeGreaterThanOrEqual(240);
    await expectMenuOptionsNotClipped(strategyMenu);

    const about = page.locator('[data-debug-id="settings.about"]');
    await about.click();
    const version = page.locator('button.version-tap');
    await expect(version).toBeVisible();
    for (let tap = 0; tap < 10; tap += 1) await version.click();

    const developer = page.locator('[data-debug-id="settings.developer"]');
    await expect(developer).toBeVisible();

    await page.evaluate(() => {
      window.addEventListener('unhandledrejection', (event) => {
        if (String(event.reason).includes('set_diagnostics_monitoring')) {
          document.documentElement.dataset.diagnosticsMonitorUnhandled = String(event.reason);
        }
      });
    });
    const monitorRequestPromise = page.waitForRequest((request) =>
      request.url().endsWith('/__verenu_dev/invoke')
      && request.method() === 'POST'
      && request.postData()?.includes('"command":"set_diagnostics_monitoring"'),
    );
    await developer.click();
    const monitorRequest = await monitorRequestPromise;
    expect(monitorRequest.postDataJSON()).toEqual({ command: 'set_diagnostics_monitoring', args: { enabled: true } });
    expect((await monitorRequest.response())?.status()).toBe(403);
    const unhandledMonitorError = await page.evaluate(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
      return document.documentElement.dataset.diagnosticsMonitorUnhandled ?? '';
    });
    expect(unhandledMonitorError).toBe('');
    await page.getByRole('tab', { name: 'Fault Injection' }).click();

    const provider = page.getByRole('button', { name: 'Fault injection provider' });
    await waitForPanelMotion(provider);
    await expect(provider).toHaveClass(/ui-dropdown-trigger--compact/);
    await expect(provider).not.toHaveClass(/btn-ghost/);
    await provider.evaluate((element) => element.scrollIntoView({ block: 'center', inline: 'nearest' }));
    await provider.click();

    const providerMenu = page.getByRole('listbox', { name: 'Fault injection provider options' });
    await waitForPanelMotion(providerMenu);
    await expect(providerMenu).toHaveClass(/ui-dropdown-menu--padded/);
    await expectMenuInsideViewport(providerMenu, page);
    const lastProviderOption = providerMenu.locator('.ui-dropdown-option').last();
    const lastOptionIsUncovered = await lastProviderOption.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      const hit = document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
      return hit === element || element.contains(hit);
    });
    expect(lastOptionIsUncovered).toBe(true);

    await page.getByRole('tab', { name: 'Logs' }).click();
    const logLevel = page.getByRole('button', { name: 'Log level' });
    await expect(logLevel).toHaveClass(/ui-dropdown-trigger--compact/);
    await expect(logLevel).not.toHaveClass(/btn-ghost/);
    await logLevel.click();
    await expect(page.getByRole('listbox', { name: 'Log level options' })).toHaveClass(/ui-dropdown-menu--padded/);
    await page.keyboard.press('Escape');

    const logSubsystem = page.getByRole('button', { name: 'Log subsystem' });
    await expect(logSubsystem).toHaveClass(/ui-dropdown-trigger--compact/);
    await expect(logSubsystem).not.toHaveClass(/btn-ghost/);
    await logSubsystem.click();
    await expect(page.getByRole('listbox', { name: 'Log subsystem options' })).toHaveClass(/ui-dropdown-menu--padded/);
  } finally {
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: initialAdvanced ?? false });
  }
});
