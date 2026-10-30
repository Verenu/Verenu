import { test, expect } from './fixtures.mjs';

test('Context creation, editing and deletion persist through the UI without touching other groups', async ({ page, session }) => {
  const before = await session.invoke('get_contexts');
  const name = `Synthetic flow ${Date.now()}`;
  let created;
  const open = async (id) => {
    await page.locator(`[data-debug-id="context.${id}"]`).click();
  };
  const actions = async (id, label) => {
    if (page.viewportSize().width < 700) {
      await page.locator(`[data-debug-id="context.${id}"]`).click({ button: 'right' });
    } else {
      const menuButton = page.getByRole('button', { name: `More actions for ${label}`, exact: true });
      await expect(menuButton).toHaveCount(1);
      await menuButton.click();
    }
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
    await page.getByRole('menuitem', { name: 'Pin', exact: true }).click();
    await expect.poll(async () => (await session.invoke('get_contexts')).find(row => row.id === created.id).pinned_at).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    await actions(created.id, renamed);
    await expect(page.getByRole('menuitem', { name: 'Unpin', exact: true })).toBeVisible();
    await page.getByRole('menuitem', { name: 'Unpin', exact: true }).click();
    await expect.poll(async () => (await session.invoke('get_contexts')).find(row => row.id === created.id).pinned_at).toBeNull();
    await actions(created.id, renamed);
    await page.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    expect((await session.invoke('get_contexts')).some(row => row.id === created.id)).toBe(true);
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();
    await expect.poll(async () => (await session.invoke('get_contexts')).some(row => row.id === created.id)).toBe(false);
    await page.reload();
    expect(await session.invoke('get_contexts')).toEqual(before);
  } finally {
    if (created && (await session.invoke('get_contexts')).some(row => row.id === created.id)) await session.invoke('delete_context', { contextId: created.id });
  }
});

test('an open editor cannot overwrite a concurrent Context update', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Concurrent ${Date.now()}` });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    if (page.viewportSize().width < 700) {
      await page.locator(`[data-debug-id="context.${context.id}"]`).click({ button: 'right' });
    } else {
      await page.getByRole('button', { name: `More actions for ${context.name}`, exact: true }).click();
    }
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    const edit = page.getByRole('dialog', { name: 'Edit context group', exact: true });
    await edit.getByLabel('Context group name').fill('User rename');
    await session.invoke('update_context_settings', { contextId: context.id, tone: 'formal' });
    await edit.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(edit.getByText(/changed while you were editing/)).toBeVisible();
    const current = (await session.invoke('get_contexts')).find(row => row.id === context.id);
    expect(current.name).toBe(context.name);
    expect(current.tone).toBe('formal');
  } finally {
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('refreshing Context items preserves an open item menu', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Menu refresh ${Date.now()}` });
  const word = await session.invoke('create_dictionary_entry', { term: 'MenuRefreshWord', contextId: context.id });
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let received;
  const requested = new Promise(resolve => { received = resolve; });
  let releaseContexts;
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.locator('.item-row').filter({ hasText: 'MenuRefreshWord' }).click();
    const edit = page.getByRole('menuitem', { name: 'Edit', exact: true });
    await expect(edit).toBeVisible();
    await page.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    await expect(page.getByRole('menuitem', { name: 'Confirm delete', exact: true })).toBeVisible();
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_context_dictionary') return route.continue();
      const response = await route.fetch();
      received();
      await held;
      await route.fulfill({ response });
    });
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:agent-contexts-changed'); });
    await requested;
    await expect(page.locator('.tab-panel.is-refreshing')).toBeVisible();
    await expect(page.getByRole('menuitem', { name: 'Delete', exact: true })).toBeVisible();
    await expect(page.getByRole('menuitem', { name: 'Delete', exact: true })).toBeDisabled();
    release();
    await expect(page.locator('.is-refreshing')).toHaveCount(0);
    await page.evaluate(async () => { await Promise.all(document.getAnimations().map(animation => animation.finished.catch(() => {}))); });
    await expect(edit).toBeVisible();
    await page.unrouteAll({ behavior: 'wait' });
    const heldContexts = new Promise(resolve => { releaseContexts = resolve; });
    let receivedContexts;
    const requestedContexts = new Promise(resolve => { receivedContexts = resolve; });
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_contexts') return route.continue();
      const response = await route.fetch();
      receivedContexts();
      await heldContexts;
      await route.fulfill({ response });
    });
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:context-updated'); });
    await requestedContexts;
    await expect(edit).not.toBeVisible();
    await page.locator('.item-row').filter({ hasText: 'MenuRefreshWord' }).click();
    await expect(edit).toBeVisible();
    releaseContexts();
    await expect(edit).not.toBeVisible();
    await page.unrouteAll({ behavior: 'wait' });
    await page.locator('.item-row').filter({ hasText: 'MenuRefreshWord' }).click();
    await expect(edit).toBeVisible();
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/diagnostics.ts')).frontendIpcActivity.snapshot().find(metric => metric.command === 'get_context_stats')?.currently_running ?? 0)).toBe(0);
    await page.unrouteAll({ behavior: 'wait' });
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_context_stats') return route.continue();
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ dictations: 1, words: 2, last_used_at: null }) });
    });
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:agent-contexts-changed'); });
    await expect(page.locator('.context-stats')).toBeVisible();
    await expect(edit).not.toBeVisible();
  } finally {
    release();
    releaseContexts?.();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('refreshing a changed search result closes its item menu', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Menu search ${Date.now()}` });
  const word = await session.invoke('create_dictionary_entry', { term: 'SearchMenuWord', contextId: context.id });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.getByRole('textbox', { name: 'Search this context group' }).fill('SearchMenuWord');
    await page.locator('.item-row').filter({ hasText: 'SearchMenuWord' }).click();
    await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toBeVisible();
    await page.getByRole('textbox', { name: 'Search this context group' }).focus();
    await page.keyboard.type('x');
    await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).not.toBeVisible();
    await page.getByRole('textbox', { name: 'Search this context group' }).fill('SearchMenuWord');
    await page.locator('.item-row').filter({ hasText: 'SearchMenuWord' }).click();
    await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toBeVisible();
    await session.invoke('edit_dictionary_entry', { id: word.id, term: 'RenamedMenuWord', contextId: context.id });
    const refresh = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_context_dictionary');
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:agent-contexts-changed'); });
    await refresh;
    await expect(page.locator('.item-row').filter({ hasText: 'SearchMenuWord' })).toHaveCount(0);
    await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).not.toBeVisible();
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('refreshing Context items preserves an open vocabulary editor', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Editor refresh ${Date.now()}` });
  const word = await session.invoke('create_dictionary_entry', { term: 'SyntheticTerm', mistake: 'old spelling', contextId: context.id });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    const row = page.locator('.item-row').filter({ hasText: 'SyntheticTerm' });
    await row.click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.locator('#dict-mistake').fill('new spelling');
    const refresh = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_context_dictionary');
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:agent-contexts-changed'); });
    await refresh;
    await expect(page.locator('#dict-mistake')).toHaveValue('new spelling');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    const entries = await session.invoke('get_context_dictionary', { contextId: context.id });
    expect(entries.filter(entry => entry.id === word.id)).toHaveLength(1);
    expect(entries.find(entry => entry.id === word.id).mistake).toBe('new spelling');
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('editing inherited vocabulary preserves the Everywhere correction', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Inherited edit ${Date.now()}` });
  const word = await session.invoke('create_dictionary_entry', { term: 'InheritedSynthetic', mistake: 'global spelling', contextId: 1 });
  try {
    await session.invoke('set_dictionary_context_assignment', { contextId: context.id, dictionaryId: word.id, assigned: true });
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.locator('.item-row').filter({ hasText: 'InheritedSynthetic' }).click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await expect(page.locator('#dict-mistake')).toHaveValue('');
    await page.locator('#dict-mistake').fill('local spelling');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    const entry = (await session.invoke('get_context_dictionary', { contextId: context.id })).find(item => item.id === word.id);
    expect(entry.context_corrections.map(item => item.mistake)).toContain('local spelling');
    expect((await session.invoke('get_context_dictionary', { contextId: 1 })).find(item => item.id === word.id).mistake).toBe('global spelling');
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('refreshing Context items preserves an open snippet editor', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Snippet refresh ${Date.now()}` });
  const snippet = await session.invoke('create_snippet', { trigger: 'synthetic snippet', expansion: 'old expansion', instructions: '', contextId: context.id });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.getByRole('tab', { name: /Snippets/ }).click();
    await page.locator('.item-row').filter({ hasText: 'synthetic snippet' }).click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.locator('#expansion-input').fill('new expansion');
    const refresh = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_context_snippets');
    await page.evaluate(async () => { const api = await import('/src/lib/tauri.ts'); await api.emit('verenu:agent-contexts-changed'); });
    await refresh;
    await expect(page.locator('#expansion-input')).toHaveValue('new expansion');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    const entries = await session.invoke('get_context_snippets', { contextId: context.id });
    expect(entries.filter(entry => entry.id === snippet.id)).toHaveLength(1);
    expect(entries.find(entry => entry.id === snippet.id).expansion).toBe('new expansion');
  } finally {
    await session.invoke('remove_snippet', { id: snippet.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('open vocabulary and snippet drafts cannot overwrite concurrent edits', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Concurrent items ${Date.now()}` });
  const word = await session.invoke('create_dictionary_entry', { term: 'SyntheticTerm', mistake: 'old spelling', contextId: context.id });
  const snippet = await session.invoke('create_snippet', { trigger: 'synthetic snippet', expansion: 'old expansion', instructions: '', contextId: context.id });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.locator('.item-row').filter({ hasText: 'SyntheticTerm' }).click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.locator('#dict-term').fill('User rename');
    await session.invoke('edit_dictionary_entry', { id: word.id, term: 'SyntheticTerm', mistake: 'agent spelling', contextId: context.id });
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByText(/changed while you were editing/)).toBeVisible();
    expect((await session.invoke('get_context_dictionary', { contextId: context.id })).find(item => item.id === word.id).mistake).toBe('agent spelling');
    await page.keyboard.press('Escape');
    await page.getByRole('tab', { name: /Snippets/ }).click();
    await page.locator('.item-row').filter({ hasText: 'synthetic snippet' }).click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await page.locator('#expansion-input').fill('user expansion');
    await session.invoke('edit_snippet', { id: snippet.id, trigger: 'synthetic snippet', expansion: 'agent expansion', instructions: '' });
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByText(/changed while you were editing/)).toBeVisible();
    expect((await session.invoke('get_context_snippets', { contextId: context.id })).find(item => item.id === snippet.id).expansion).toBe('agent expansion');
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('remove_snippet', { id: snippet.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});

test('moving a conflicting snippet rejects changes made after the conflict appeared', async ({ page, session }) => {
  const context = await session.invoke('create_context', { name: `Move conflict ${Date.now()}` });
  const snippet = await session.invoke('create_snippet', { trigger: 'synthetic move', expansion: 'original', instructions: '', contextId: 1 });
  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${context.id}"]`).click();
    await page.getByRole('tab', { name: /Snippets/ }).click();
    await page.getByRole('button', { name: 'Add snippet' }).click();
    await page.locator('#trigger-input').fill('synthetic move');
    await page.locator('#expansion-input').fill('user draft');
    await page.getByRole('button', { name: 'Add snippet', exact: true }).last().click();
    await expect(page.getByRole('button', { name: 'Move it here' })).toBeVisible();
    await session.invoke('edit_snippet', { id: snippet.id, trigger: 'synthetic move', expansion: 'agent edit', instructions: '' });
    await page.getByRole('button', { name: /Move.*here/i }).click();
    await expect(page.getByText(/changed while you were editing/)).toBeVisible();
    expect((await session.invoke('get_snippets')).find(item => item.id === snippet.id).expansion).toBe('agent edit');
  } finally {
    await session.invoke('remove_snippet', { id: snippet.id });
    await session.invoke('delete_context', { contextId: context.id });
  }
});
