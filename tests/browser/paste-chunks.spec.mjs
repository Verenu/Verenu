import { test, expect } from './fixtures.mjs';
import path from 'node:path';
const evidenceDir = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);

test('desktop Context paste preference saves and survives reload and duplication', async ({ page, session }) => {
  await page.getByRole('button', { name: 'New context group', exact: true }).click();
  await page.locator('#context-name').fill('Synthetic CLI context');
  await page.locator('.advanced-trigger').click();
  const toggle = page.getByRole('switch', { name: 'Paste in Chunks', exact: true });
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await toggle.click();
  await page.getByRole('button', { name: 'Create context group', exact: true }).click();
  let context = (await session.invoke('get_contexts')).find(row => row.name === 'Synthetic CLI context');
  let duplicate;
  try {
    expect(context.paste_in_chunks).toBe(true);
    await page.reload();
    const viewport = page.viewportSize();
    // Desktop sidebar names collapse below the native 900px minimum.
    if (viewport.width < 900) await page.setViewportSize({ width: 1320, height: 860 });
    await page.getByRole('button', { name: 'Synthetic CLI context', exact: true }).click();
    await page.getByRole('button', { name: 'More actions for Synthetic CLI context', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.locator('.advanced-trigger').click();
    await page.setViewportSize(viewport);
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await toggle.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(evidenceDir, `paste-chunks-desktop-${test.info().project.name}.png`) });
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    duplicate = await session.invoke('duplicate_context', { contextId: context.id });
    expect(duplicate.paste_in_chunks).toBe(true);
    // Mobile and older callers omit this desktop option when editing style.
    await session.invoke('update_context_settings', { contextId: context.id, contextualFormattingDisabled: false });
    context = (await session.invoke('get_contexts')).find(row => row.id === context.id);
    expect(context.paste_in_chunks).toBe(true);
    await session.invoke('update_context_settings', { contextId: context.id, contextualFormattingDisabled: false, pasteInChunks: false });
    expect((await session.invoke('get_contexts')).find(row => row.id === context.id).paste_in_chunks).toBe(false);
  } finally {
    if (duplicate) await session.invoke('delete_context', { contextId: duplicate.id });
    if (context) await session.invoke('delete_context', { contextId: context.id });
  }
});

test('mobile Context editor hides desktop paste preference and preserves synced value', async ({ browser, session }) => {
  const mobile = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, userAgent: 'Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/131.0.0.0 Mobile Safari/537.36' });
  const page = await mobile.newPage();
  const context = await session.invoke('create_context', { name: 'Synced desktop CLI', contextualFormattingDisabled: false, pasteInChunks: true });
  try {
    await page.goto(session.access.localAccessUrl);
    await page.getByRole('button', { name: 'Contexts', exact: true }).click();
    await page.getByRole('tab', { name: 'Synced desktop CLI', exact: true }).click();
    await page.getByRole('button', { name: 'Edit context group', exact: true }).click();
    await page.locator('.advanced-trigger').click();
    await page.getByRole('switch', { name: 'Disable smart formatting for this context', exact: true }).scrollIntoViewIfNeeded();
    await expect(page.getByRole('switch', { name: 'Paste in Chunks', exact: true })).toHaveCount(0);
    await page.screenshot({ path: path.join(evidenceDir, `paste-chunks-mobile-${test.info().project.name}.png`) });
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    expect((await session.invoke('get_contexts')).find(row => row.id === context.id).paste_in_chunks).toBe(true);
  } finally {
    await mobile.close();
    await session.invoke('delete_context', { contextId: context.id });
  }
});
