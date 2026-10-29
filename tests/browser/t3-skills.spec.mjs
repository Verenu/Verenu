import { test, expect } from './fixtures.mjs';

test('T3 integration shows compatibility and protects native pairing in browser sessions', async ({ page, session }) => {
  const status = await session.invoke('get_t3_skills');
  expect(status.minimumVersion).toBe('0.46');
  expect(status.connection).toBeNull();
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.integrations"]').click();
  const panel = page.getByRole('region', { name: 'T3 Code integration', exact: true });
  await expect(panel.getByRole('button', { name: 'Connect T3 Code', exact: true })).toBeDisabled();
  await expect(panel).toContainText('Use localhost, a private LAN address, or an HTTPS address.');
  await expect(panel).toContainText(`T3 Code ${status.minimumVersion} or newer`);
  const input = panel.locator('#t3-pairing-link');
  await input.fill('https://synthetic.invalid/pair#token=synthetic');
  await panel.getByRole('button', { name: 'Connect T3 Code', exact: true }).click();
  await expect(panel.getByRole('alert')).toBeVisible();
  await expect(input).toHaveValue('');
  expect(await session.invoke('get_t3_skills')).toEqual(status);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('Context groups omit T3 integration copy and preserve saved skill preferences', async ({ page, session }) => {
  const contexts = await session.invoke('get_contexts');
  const everywhere = contexts.find(context => context.is_everywhere);
  const previous = !!everywhere.t3_skill_mentions_disabled;
  const coding = await session.invoke('create_context', { name: 'Synthetic AI coding', contextualFormattingDisabled: false });
  expect(coding.t3_skill_mentions_disabled).toBe(false);
  await session.invoke('assign_context_target', { contextId: coding.id, executable: 't3-code', appName: 'T3 Code' });
  await session.invoke('set_context_t3_skill_mentions', { contextId: coding.id, enabled: false });
  const viewport = page.viewportSize();
  const expectNoIntegrationBlock = async () => {
    await expect(page.getByRole('region', { name: 'T3 Code skill mentions', exact: true })).toHaveCount(0);
    await expect(page.getByText('T3 Code · Format spoken skill names', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/Say “use my babysit skill”/)).toHaveCount(0);
    await expect(page.getByText(/Only skill names go to that provider/)).toHaveCount(0);
  };
  try {
    await session.invoke('set_context_t3_skill_mentions', { contextId: everywhere.id, enabled: true });
    await page.reload();
    for (const context of [everywhere, coding]) {
      await page.locator(`[data-debug-id="context.${context.id}"]`).click();
      await expect(page.locator('.context-content:not([inert]) h2')).toHaveText(context.name);
      await expectNoIntegrationBlock();
      await page.setViewportSize({ width: 1320, height: 860 });
      await page.getByRole('button', { name: `More actions for ${context.name}`, exact: true }).click();
      await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
      await page.setViewportSize(viewport);
      await page.getByRole('dialog').waitFor();
      await expectNoIntegrationBlock();
      await page.getByRole('button', { name: 'Save changes', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
    }
    await page.reload();
    const saved = await session.invoke('get_contexts');
    expect(saved.find(context => context.id === everywhere.id).t3_skill_mentions_disabled).toBe(false);
    expect(saved.find(context => context.id === coding.id).t3_skill_mentions_disabled).toBe(true);
    for (const context of [everywhere, coding]) {
      await page.setViewportSize({ width: 1320, height: 860 });
      await page.locator(`[data-debug-id="context.${context.id}"]`).click();
      await page.getByRole('button', { name: `More actions for ${context.name}`, exact: true }).click();
      await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
      await page.setViewportSize(viewport);
      await page.locator('.advanced-trigger').click();
      const toggle = page.getByRole('switch', { name: 'Format spoken skill names in T3 Code', exact: true });
      await expect(toggle).toHaveAttribute('aria-checked', context.id === everywhere.id ? 'true' : 'false');
      await expectNoIntegrationBlock();
      await toggle.click();
      await page.getByRole('button', { name: 'Save changes', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await page.reload();
      expect((await session.invoke('get_contexts')).find(row => row.id === context.id).t3_skill_mentions_disabled).toBe(context.id === everywhere.id);
    }
  } finally {
    await session.invoke('set_context_t3_skill_mentions', { contextId: everywhere.id, enabled: !previous });
    await session.invoke('delete_context', { contextId: coding.id });
  }
});
