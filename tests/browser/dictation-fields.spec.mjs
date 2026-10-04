import { test, expect } from './fixtures.mjs';

async function pasteSynthetic(page, field, text) {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await field.focus();
  await page.evaluate((value) => navigator.clipboard.writeText(value), text);
  await field.press(process.platform === 'darwin' ? 'Meta+V' : 'Control+V');
  await expect(field).toHaveValue(text);
}

async function openEverywhere(page) {
  await page.locator('[data-debug-id="context.1"]').click();
  await expect(page.getByRole('button', { name: '+ Term', exact: true })).toBeVisible();
}

test('Context vocabulary accepts normal paste and saves without microphone controls', async ({ page, session }) => {
  const term = `Synthetic vocabulary ${Date.now()}`;
  let created;
  try {
    await openEverywhere(page);
    await page.getByRole('button', { name: '+ Term', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Add term', exact: true });
    await expect(dialog.getByRole('button', { name: 'Transcribe with microphone' })).toHaveCount(0);
    await pasteSynthetic(page, dialog.locator('#dict-term'), term);
    await pasteSynthetic(page, dialog.locator('#dict-mistake'), 'synthetic mistake');
    await expect(dialog.locator('.char-count').first()).toHaveText(`${term.length}/120`);
    await dialog.getByRole('button', { name: 'Add term', exact: true }).click();
    await expect(dialog).toBeHidden();
    created = (await session.invoke('get_dictionary')).find((entry) => entry.term === term);
    expect(created?.mistake).toBe('synthetic mistake');
    await page.reload();
    await openEverywhere(page);
    await expect(page.getByText(term, { exact: true })).toBeVisible();
  } finally {
    if (created) await session.invoke('remove_dictionary_entry', { id: created.dictionary_id ?? created.id });
  }
});

test('Context snippets accept normal paste in every field and persist', async ({ page, session }) => {
  const trigger = `synthetic snippet ${Date.now()}`;
  let created;
  try {
    await openEverywhere(page);
    await page.getByRole('tab', { name: /^Snippets/ }).click();
    await page.getByRole('button', { name: '+ Snippet', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New snippet', exact: true });
    await expect(dialog.getByRole('button', { name: 'Transcribe with microphone' })).toHaveCount(0);
    await pasteSynthetic(page, dialog.locator('#trigger-input'), trigger);
    await pasteSynthetic(page, dialog.locator('#expansion-input'), 'Public synthetic expansion.\nSecond line.');
    await pasteSynthetic(page, dialog.locator('#instructions-input'), 'Keep the line break.');
    await dialog.getByRole('button', { name: 'Add snippet', exact: true }).click();
    await expect(dialog).toBeHidden();
    created = (await session.invoke('get_snippets')).find((entry) => entry.trigger === trigger);
    expect(created?.expansion).toBe('Public synthetic expansion.\nSecond line.');
    expect(created?.instructions).toBe('Keep the line break.');
    await page.reload();
    await openEverywhere(page);
    await page.getByRole('tab', { name: /^Snippets/ }).click();
    await expect(page.getByText(trigger, { exact: true })).toBeVisible();
  } finally {
    if (created) await session.invoke('remove_snippet', { id: created.id });
  }
});
