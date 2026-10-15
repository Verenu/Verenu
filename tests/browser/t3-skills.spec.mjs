import { test, expect } from './fixtures.mjs';

test('T3 integration shows compatibility and protects native pairing in browser sessions', async ({ page, session }) => {
  const status = await session.invoke('get_t3_skills');
  expect(status.minimumVersion).toBe('0.46');
  expect(status.connection).toBeNull();
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.integrations"]').click();
  const panel = page.getByRole('region', { name: 'T3 Code integration', exact: true });
  await expect(panel.getByRole('button', { name: 'Connect T3 Code', exact: true })).toBeDisabled();
  await expect(panel).toContainText('Use localhost, a private LAN address, or an HTTPS address. Tailscale is optional.');
  // Compatibility and privacy details sit behind the card's info control.
  const about = panel.getByRole('button', { name: 'About T3 Code', exact: true });
  await expect(about).toHaveAttribute('aria-expanded', 'false');
  await about.click();
  await expect(about).toHaveAttribute('aria-expanded', 'true');
  await expect(panel.getByText(`T3 Code ${status.minimumVersion} or newer.`, { exact: true })).toBeVisible();
  await expect(panel).toContainText('Skill instructions are never imported');
  // Escape closes the disclosure and leaves Settings open.
  await page.keyboard.press('Escape');
  await expect(about).toHaveAttribute('aria-expanded', 'false');
  await expect(panel).toBeVisible();
  const input = panel.locator('#t3-pairing-link');
  await input.fill('https://synthetic.invalid/pair#token=synthetic');
  await panel.getByRole('button', { name: 'Connect T3 Code', exact: true }).click();
  await expect(panel.getByRole('alert')).toBeVisible();
  await expect(input).toHaveValue('');
  expect(await session.invoke('get_t3_skills')).toEqual(status);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('T3 skill opt-out defaults on and persists across renderer reload', async ({ page, session }) => {
  const contexts = await session.invoke('get_contexts');
  const everywhere = contexts.find(context => context.is_everywhere);
  const previous = !!everywhere.t3_skill_mentions_disabled;
  try {
    await session.invoke('set_context_t3_skill_mentions', { contextId: everywhere.id, enabled: true });
    await page.reload();
    await page.locator(`[data-debug-id="context.${everywhere.id}"]`).click();
    const toggle = page.getByRole('switch', { name: 'Format spoken skill names in T3 Code' });
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect((await session.invoke('get_contexts')).find(context => context.id === everywhere.id).t3_skill_mentions_disabled).toBe(true);
    await page.reload();
    await page.locator(`[data-debug-id="context.${everywhere.id}"]`).click();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
  } finally {
    await session.invoke('set_context_t3_skill_mentions', { contextId: everywhere.id, enabled: !previous });
  }
});
