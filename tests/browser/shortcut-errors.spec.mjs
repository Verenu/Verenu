import { test, expect } from './fixtures.mjs';

const failure = {
  id: 'dictation', requested: 'Ctrl + Super', active: null, codes: [],
  note: 'Global shortcuts are unavailable. The desktop portal could not identify Verenu. Reinstall Verenu and restart the app. Verenu will retry automatically. Details: Creating shortcut session: An app id is required',
};
const working = { ...failure, active: 'Ctrl + Super', codes: ['ControlLeft', 'MetaLeft'], note: null };

// Browser sessions disable OS shortcuts. Deliver the real event contract
// through the Rust bridge; the native probe covers actual portal registration.
async function publish(page, status) {
  await page.evaluate(async (item) => {
    const { emit } = await import('/src/lib/tauri.ts');
    await emit('verenu:shortcuts-changed', [item]);
  }, status);
}

test('shortcut failure remains visible and ready recovery restores dictation guidance', async ({ page, readySpeech }) => {
  await expect(page.getByRole('heading', { name: /Hold Ctrl.*Super.*to dictate/ })).toBeVisible();
  await publish(page, failure);
  await expect(page.getByRole('heading', { name: 'Dictation shortcut unavailable' })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'An app id is required' })).toBeVisible();
  await expect(page.getByText('Hold Unavailable', { exact: false })).toHaveCount(0);
  // A saved-hotkey notification must not advertise an unregistered shortcut.
  await page.evaluate(async () => {
    const { emit } = await import('/src/lib/tauri.ts');
    await emit('verenu:hotkey-changed', ['ControlLeft', 'MetaLeft']);
  });
  await expect(page.getByRole('heading', { name: 'Dictation shortcut unavailable' })).toBeVisible();
  await page.evaluate(async () => {
    const { emit } = await import('/src/lib/tauri.ts');
    await emit('open-flow:open-settings-section', 'general');
  });
  await expect(page.getByRole('button', { name: 'Change dictation hotkey' })).toBeVisible();
  await publish(page, failure);
  await expect(page.getByRole('alert').filter({ hasText: 'An app id is required' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await publish(page, working);
  await expect(page.getByRole('alert').filter({ hasText: 'An app id is required' })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: /Hold Ctrl.*Super.*to dictate/ })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'An app id is required' })).toHaveCount(0);
});

test('shortcut recovery keeps setup guidance while speech is not ready', async ({ page, incompleteSpeech }) => {
  await expect(page.getByRole('heading', { name: 'Finish dictation setup' })).toBeVisible();
  await publish(page, failure);
  await expect(page.getByRole('heading', { name: 'Dictation shortcut unavailable' })).toBeVisible();
  await page.evaluate(async () => {
    const { emit } = await import('/src/lib/tauri.ts');
    await emit('open-flow:open-settings-section', 'general');
  });
  await expect(page.getByRole('button', { name: 'Change dictation hotkey' })).toBeVisible();
  await publish(page, working);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Finish dictation setup' })).toBeVisible();
  await expect(page.getByRole('heading', { name: /Hold Ctrl.*Super.*to dictate/ })).toHaveCount(0);
  await expect(page.getByText('does not support speech recognition')).toBeVisible();
});
