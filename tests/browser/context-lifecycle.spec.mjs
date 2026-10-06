import { test, expect } from './fixtures.mjs';

test('Context creation, editing and deletion persist through the UI without touching other groups', async ({ page, session }) => {
  const before = await session.invoke('get_contexts');
  const name = `Synthetic flow ${Date.now()}`;
  let created;
  const open = async (id) => {
    // Desktop Context labels collapse at narrow widths; the ID remains stable.
    await page.locator(`[data-debug-id="context.${id}"]`).click();
  };
  const actions = async (id, label) => {
    if (page.viewportSize().width < 700) {
      // The compact rail exposes the row's context menu instead of a kebab.
      await page.locator(`[data-debug-id="context.${id}"]`).click({ button: 'right' });
    } else await page.getByRole('button', { name: `More actions for ${label}`, exact: true }).click();
  };
  try {
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await dialog.getByLabel('Context group name').fill(name);
    await dialog.getByRole('button', { name: 'Create context group', exact: true }).click();
    await expect(dialog).toBeHidden();
    created = (await session.invoke('get_contexts')).find(row => row.name === name);
    expect(created).toBeTruthy();
    await page.reload();
    await open(created.id);
    await actions(created.id, name);
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    const edit = page.getByRole('dialog', { name: 'Edit context group', exact: true });
    await expect(edit.getByLabel('Context group name')).toHaveValue(name);
    const renamed = `Edited flow ${Date.now()}`;
    await edit.getByLabel('Context group name').fill(renamed);
    await edit.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(edit).toBeHidden();
    await page.reload();
    expect((await session.invoke('get_contexts')).find(row => row.id === created.id).name).toBe(renamed);
    await open(created.id);
    await actions(created.id, renamed);
    await page.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    // The first click arms deletion; it must not mutate data yet.
    expect((await session.invoke('get_contexts')).some(row => row.id === created.id)).toBe(true);
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();
    await expect.poll(async () => (await session.invoke('get_contexts')).some(row => row.id === created.id)).toBe(false);
    await page.reload();
    expect(await session.invoke('get_contexts')).toEqual(before);
  } finally {
    if (created && (await session.invoke('get_contexts')).some(row => row.id === created.id)) await session.invoke('delete_context', { contextId: created.id });
  }
});
