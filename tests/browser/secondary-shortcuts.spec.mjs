import { test, expect } from './fixtures.mjs';

async function openGeneral(page) {
  await expect(page.getByRole('button', { name: /^Dev tests/ })).toBeVisible({ timeout: 30_000 });
  await page.locator('[data-debug-id="nav.settings"]').click();
  await expect(page.getByRole('button', { name: 'Change Cancel dictation shortcut', exact: true })).toBeVisible();
}

async function waitForCaptureReady(button) {
  await expect(button).toContainText('Press a key');
}

test('secondary shortcut controls edit unavailable keys and persist through reload', async ({ page, session }) => {
  const previous = await session.invoke('get_all_settings');
  try {
    await openGeneral(page);
    // Exercise the native status event contract. This does not claim OS registration.
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:shortcuts-changed', [
        { id: 'cancel', requested: 'Escape', active: null, codes: [], note: 'No available shortcut. Change the desktop binding or choose another shortcut.' },
        { id: 'handsfree', requested: 'Space', active: null, codes: [], note: 'No available shortcut. Change the desktop binding or choose another shortcut.' },
      ]);
    });
    const cancel = page.getByRole('button', { name: 'Change Cancel dictation shortcut', exact: true });
    const handsfree = page.getByRole('button', { name: 'Change Switch to hands-free shortcut', exact: true });
    await expect(cancel).toBeEnabled();
    await expect(handsfree).toBeEnabled();
    await cancel.click();
    await waitForCaptureReady(cancel);
    await page.keyboard.press('F8');
    await expect.poll(async () => (await session.invoke('get_all_settings')).linux_cancel_key).toBe('F8');
    await expect(cancel).toContainText('F8');
    await handsfree.click();
    await waitForCaptureReady(handsfree);
    await page.keyboard.press('F9');
    await expect.poll(async () => (await session.invoke('get_all_settings')).linux_handsfree_key).toBe('F9');
    await expect(handsfree).toContainText('F9');
    await page.reload();
    await openGeneral(page);
    await expect(cancel).toContainText('F8');
    await expect(handsfree).toContainText('F9');
    await cancel.click();
    await waitForCaptureReady(cancel);
    await page.keyboard.press('Control+F10');
    await expect(page.getByRole('alert').filter({ hasText: 'Choose a single key without modifiers' })).toBeVisible();
    expect((await session.invoke('get_all_settings')).linux_cancel_key).toBe('F8');
    // Escape is assignable; cancelling capture uses the explicit outside click.
    await cancel.click();
    await waitForCaptureReady(cancel);
    await page.keyboard.press('Escape');
    await expect.poll(async () => (await session.invoke('get_all_settings')).linux_cancel_key).toBe('Escape');
    await handsfree.click();
    await waitForCaptureReady(handsfree);
    await page.getByText('Switch to hands-free', { exact: true }).click();
    await expect(handsfree).toContainText('F9');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  } finally {
    await session.invoke('save_setting', { key: 'linux_cancel_key', value: previous.linux_cancel_key ?? 'Escape' });
    await session.invoke('save_setting', { key: 'linux_handsfree_key', value: previous.linux_handsfree_key ?? 'Space' });
  }
});
