import { test, expect } from './fixtures.mjs';

test('real settings save, reload, and restore through the UI', async ({ page, session }) => {
  const initial = (await session.invoke('get_all_settings')).legacy_features_enabled === true;
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    const control = page.getByRole('switch', { name: 'Legacy pages', exact: true });
    await expect(control).toHaveAttribute('aria-checked', String(initial));
    await control.click();
    if (!initial) await page.getByRole('dialog', { name: 'Turn on Legacy pages?' }).getByRole('button', { name: 'Turn on', exact: true }).click();
    await expect(control).toHaveAttribute('aria-checked', String(!initial));
    await expect.poll(async () => (await session.invoke('get_all_settings')).legacy_features_enabled).toBe(!initial);
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await expect(control).toHaveAttribute('aria-checked', String(!initial));
  } finally { await session.invoke('save_setting', { key: 'legacy_features_enabled', value: initial }); }
});

test('failed Context validation leaves real data intact and session recovers', async ({ session }) => {
  const before = await session.invoke('get_contexts');
  const created = await session.invoke('create_context', { name: 'Synthetic recovery', contextualFormattingDisabled: false });
  try {
    const response = await fetch(new URL('/__verenu_dev/invoke', session.access.localAccessUrl), { method: 'POST', headers: { Authorization: `Bearer ${session.access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command: 'update_context', args: { contextId: created.id, name: 'x'.repeat(31) } }) });
    expect(response.ok).toBeFalsy();
    expect((await session.invoke('get_contexts')).find((row) => row.id === created.id).name).toBe('Synthetic recovery');
    await session.invoke('update_context', { contextId: created.id, name: 'Recovered' });
    expect((await session.invoke('get_contexts')).find((row) => row.id === created.id).name).toBe('Recovered');
  } finally { await session.invoke('delete_context', { contextId: created.id }); }
  expect((await session.invoke('get_contexts')).length).toBe(before.length);
});
