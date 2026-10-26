import { test, expect } from './fixtures.mjs';

test('Everywhere content is visible in groups and clearing local formatting restores inheritance', async ({ page, session }) => {
  const original = (await session.invoke('get_contexts')).find((context) => context.is_everywhere);
  const save = (contextId, contextualFormattingOverride) => session.invoke('update_context_settings', {
    contextId, contextualFormattingOverride,
  });
  const group = await session.invoke('create_context', { name: 'Baseline regression', contextualFormattingOverride: false });
  const word = await session.invoke('create_dictionary_entry', { term: 'BaselineWord', mistake: 'baseline-word', contextId: original.id });
  try {
    await save(original.id, true);
    await page.reload();
    const groupLink = page.locator(`[data-debug-id="context.${group.id}"]`);
    await groupLink.click();
    await expect(page.getByText('Layers over Everywhere when this context group matches.')).toBeVisible();
    const row = page.locator('.item-row').filter({ hasText: 'BaselineWord' });
    await expect(row).toBeVisible();
    await expect(row.getByText('Everywhere', { exact: true })).toBeVisible();
    await groupLink.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.getByRole('button', { name: /Advanced/ }).click();
    await page.getByRole('button', { name: 'Smart formatting', exact: true }).click();
    await expect(page.getByRole('option', { name: 'Off', exact: true })).toBeInViewport();
    await page.getByRole('option', { name: 'Off', exact: true }).click();
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Edit context group' })).not.toBeVisible();
    expect((await session.invoke('get_contexts')).find((context) => context.id === group.id).contextual_formatting_override).toBe(true);
    await groupLink.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.getByRole('button', { name: /Advanced/ }).click();
    await page.getByRole('button', { name: 'Smart formatting', exact: true }).click();
    await page.getByRole('option', { name: 'On', exact: true }).click();
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Edit context group' })).not.toBeVisible();
    expect((await session.invoke('get_contexts')).find((context) => context.id === group.id).contextual_formatting_override).toBe(false);
    await groupLink.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.getByRole('button', { name: /Advanced/ }).click();
    await page.getByRole('button', { name: 'Smart formatting', exact: true }).click();
    await page.getByRole('option', { name: 'Use Everywhere', exact: true }).click();
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Edit context group' })).not.toBeVisible();
    const saved = (await session.invoke('get_contexts')).find((context) => context.id === group.id);
    expect(saved.contextual_formatting_override).toBeNull();
    expect((await session.invoke('get_context_dictionary', { contextId: group.id })).some((entry) => entry.id === word.id && entry.inherited_from_everywhere)).toBeTruthy();
    await row.click();
    await page.getByRole('menuitem', { name: 'Customize here', exact: true }).click();
    await page.locator('#dict-mistake').fill('local-baseline');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    await expect(row.getByText('Everywhere + this group', { exact: true })).toBeVisible();
    expect((await session.invoke('get_context_dictionary', { contextId: original.id })).find((entry) => entry.id === word.id).mistake).toBe('baseline-word');
    await row.click();
    await page.getByRole('menuitem', { name: 'Clear local override', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();
    await expect(row.getByText('Everywhere', { exact: true })).toBeVisible();
    await expect(row).not.toContainText('local-baseline');
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('delete_context', { contextId: group.id });
    await session.invoke('update_context_settings', {
      contextId: original.id, icon: original.icon, tone: original.tone,
      cleanupIntensity: original.cleanup_intensity, customInstructions: original.custom_instructions,
      contextualFormattingOverride: original.contextual_formatting_override,
    });
  }
});
