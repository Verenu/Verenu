import { test, expect } from './fixtures.mjs';

test('Groups show their own content while Everywhere remains inherited', async ({ page, session }, testInfo) => {
  const original = (await session.invoke('get_contexts')).find((context) => context.is_everywhere);
  const save = (contextId, contextualFormattingOverride) => session.invoke('update_context_settings', {
    contextId, contextualFormattingOverride,
  });
  const group = await session.invoke('create_context', { name: 'Baseline regression', contextualFormattingOverride: false });
  const word = await session.invoke('create_dictionary_entry', { term: 'BaselineWord', mistake: 'baseline-word', contextId: original.id });
  const localWord = await session.invoke('create_dictionary_entry', { term: 'LocalWord', mistake: 'local-word', contextId: group.id });
  const snippet = await session.invoke('create_snippet', { trigger: 'baseline snippet', expansion: 'Synthetic expansion', instructions: '', contextId: original.id });
  const movableSnippet = await session.invoke('create_snippet', { trigger: 'move this snippet', expansion: 'Everywhere move target expansion', instructions: '', contextId: original.id });
  const localSnippet = await session.invoke('create_snippet', { trigger: 'local snippet', expansion: 'Local expansion', instructions: '', contextId: group.id });
  try {
    await save(original.id, true);
    await page.reload();
    const groupLink = page.locator(`[data-debug-id="context.${group.id}"]`);
    await groupLink.click();
    await expect(page.getByText('Layers over Everywhere when this context group matches.')).toBeVisible();
    const row = page.locator('.item-row').filter({ hasText: 'BaselineWord' });
    await expect(row).not.toBeVisible();
    await expect(page.locator('.item-row').filter({ hasText: 'LocalWord' })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('group-vocabulary.png'), animations: 'disabled' });
    await page.getByRole('tab', { name: /Snippets/ }).click();
    await expect(page.locator('.item-row').filter({ hasText: 'baseline snippet' })).not.toBeVisible();
    await expect(page.locator('.item-row').filter({ hasText: 'move this snippet' })).not.toBeVisible();
    await expect(page.locator('.item-row').filter({ hasText: 'local snippet' })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('group-snippets.png'), animations: 'disabled' });
    await page.getByRole('button', { name: '+ Snippet', exact: true }).click();
    await page.locator('#trigger-input').fill('move this snippet');
    await page.locator('#expansion-input').fill('Group move target expansion');
    await page.locator('#instructions-input').fill('');
    await page.getByRole('button', { name: 'Add snippet', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('Move it here?');
    await page.getByRole('button', { name: 'Move it here', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    const movedSnippetRow = page.locator('.item-row').filter({ hasText: 'move this snippet' });
    await expect(movedSnippetRow).toBeVisible();
    await expect(movedSnippetRow).toContainText('Group move target expansion');
    await expect(movedSnippetRow.locator('.item-origin')).toHaveCount(0);
    await page.getByRole('tab', { name: /Vocabulary/ }).click();
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
    await session.invoke('set_snippet_context_assignment', { contextId: group.id, snippetId: snippet.id, assigned: true });
    await page.reload();
    await page.locator(`[data-debug-id="context.${group.id}"]`).click();
    await page.getByRole('tab', { name: /Snippets/ }).click();
    const inheritedSnippet = page.locator('.item-row').filter({ hasText: 'baseline snippet' });
    await expect(inheritedSnippet).toBeVisible();
    await expect(inheritedSnippet.locator('.item-origin')).toHaveCount(0);
    await page.getByRole('tab', { name: /Vocabulary/ }).click();
    await session.invoke('set_dictionary_context_assignment', { contextId: group.id, dictionaryId: word.id, assigned: true });
    await page.reload();
    await page.locator(`[data-debug-id="context.${group.id}"]`).click();
    await expect(row).toBeVisible();
    await expect(row.locator('.item-origin')).toHaveCount(0);
    await row.click();
    await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
    await expect(page.locator('#dict-term')).not.toBeEditable();
    await page.locator('#dict-mistake').fill('local-baseline');
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    await expect(row).toContainText('local-baseline');
    expect((await session.invoke('get_context_dictionary', { contextId: original.id })).find((entry) => entry.id === word.id).mistake).toBe('baseline-word');
    await row.click();
    await page.getByRole('menuitem', { name: 'Clear local override', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Confirm delete', exact: true }).click();
    await expect(row).not.toBeVisible();
  } finally {
    await session.invoke('remove_dictionary_entry', { id: word.id });
    await session.invoke('remove_dictionary_entry', { id: localWord.id });
    await session.invoke('remove_snippet', { id: snippet.id });
    await session.invoke('remove_snippet', { id: movableSnippet.id });
    await session.invoke('remove_snippet', { id: localSnippet.id });
    await session.invoke('delete_context', { contextId: group.id });
    await session.invoke('update_context_settings', {
      contextId: original.id, icon: original.icon, tone: original.tone,
      cleanupIntensity: original.cleanup_intensity, customInstructions: original.custom_instructions,
      contextualFormattingOverride: original.contextual_formatting_override,
    });
  }
});

test('A delayed snippet assignment refresh does not replace the selected group', async ({ page, session }) => {
  const sourceGroup = await session.invoke('create_context', { name: 'Assignment source', contextualFormattingOverride: false });
  const destinationGroup = await session.invoke('create_context', { name: 'Assignment destination', contextualFormattingOverride: false });
  const sourceSnippet = await session.invoke('create_snippet', {
    trigger: 'source group snippet', expansion: 'Source expansion', instructions: '', contextId: sourceGroup.id,
  });
  const destinationSnippet = await session.invoke('create_snippet', {
    trigger: 'destination group snippet', expansion: 'Destination expansion', instructions: '', contextId: destinationGroup.id,
  });
  let releaseAssignment;
  let signalAssignmentStarted;
  const waitForRelease = new Promise((resolve) => { releaseAssignment = resolve; });
  const assignmentStarted = new Promise((resolve) => { signalAssignmentStarted = resolve; });
  const holdAssignmentResponse = async (route) => {
    const request = route.request().postDataJSON();
    if (request?.command === 'set_snippet_context_assignment'
      && request.args?.contextId === sourceGroup.id
      && request.args?.assigned === true) {
      const response = await route.fetch();
      signalAssignmentStarted();
      await waitForRelease;
      await route.fulfill({ response });
      return;
    }
    await route.continue();
  };

  try {
    await page.reload();
    await page.locator(`[data-debug-id="context.${sourceGroup.id}"]`).click();
    await page.getByRole('tab', { name: /Snippets/ }).click();
    await page.route('**/__verenu_dev/invoke', holdAssignmentResponse);
    await page.getByRole('button', { name: '+ Snippet', exact: true }).click();
    await page.locator('#trigger-input').fill('pending assignment snippet');
    await page.locator('#expansion-input').fill('Pending expansion');
    await page.getByRole('button', { name: 'Add snippet', exact: true }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
    await assignmentStarted;
    await page.locator(`[data-debug-id="context.${destinationGroup.id}"]`).click();
    await expect(page.getByRole('heading', { name: 'Assignment destination' })).toBeVisible();
    await page.getByRole('tab', { name: /Snippets/ }).click();
    const destinationRow = page.locator('.item-row').filter({ hasText: 'destination group snippet' });
    await expect(destinationRow).toBeVisible();
    const staleRefreshStarted = page.evaluate(() => new Promise((resolve) => {
      let observed = false;
      const observer = new MutationObserver(() => {
        if (document.querySelector('.tab-panel.is-refreshing')) observed = true;
      });
      observer.observe(document.querySelector('main'), { attributes: true, subtree: true, attributeFilter: ['class'] });
      window.setTimeout(() => {
        observer.disconnect();
        resolve(observed);
      }, 500);
    }));
    releaseAssignment();
    expect(await staleRefreshStarted).toBe(false);
    await expect(page.getByRole('heading', { name: 'Assignment destination' })).toBeVisible();
    await expect(destinationRow).toBeVisible();
    await expect(page.locator('.item-row').filter({ hasText: 'pending assignment snippet' })).not.toBeVisible();
  } finally {
    releaseAssignment();
    if (!page.isClosed()) await page.unroute('**/__verenu_dev/invoke', holdAssignmentResponse);
    await session.invoke('remove_snippet', { id: sourceSnippet.id });
    await session.invoke('remove_snippet', { id: destinationSnippet.id });
    await session.invoke('delete_context', { contextId: sourceGroup.id });
    await session.invoke('delete_context', { contextId: destinationGroup.id });
  }
});
