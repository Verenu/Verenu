import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { chromium } from 'playwright';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to a real dev session access.json.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const session = await (await fetch(new URL('/__verenu_dev/session', access.localAccessUrl), {
  headers: { Authorization: `Bearer ${access.token}` },
})).json();

async function invoke(command, args = {}) {
  const response = await fetch(new URL('/__verenu_dev/invoke', access.localAccessUrl), {
    method: 'POST',
    headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, args }),
  });
  assert.equal(response.status, 200, `${command} failed`);
  return response.json();
}

test('hotkey capture saves the whole held chord on release and keeps rejected bindings', { timeout: 90_000 }, async () => {
  const original = await invoke('get_setting', { key: 'hotkey' });
  const modifiers = ['ControlLeft', 'AltLeft', 'ShiftLeft', 'MetaLeft'];
  let codes;
  for (const trigger of ['KeyK', 'F6', 'F7', 'F8', 'F9']) {
    const candidate = [...modifiers, trigger];
    if (await invoke('check_hotkey', { keys: candidate })) { codes = candidate; break; }
  }
  assert.ok(codes, 'A five-key test chord must be available');
  const browser = await chromium.launch({ headless: true });
  try {
    for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport, reducedMotion: 'reduce' });
      const page = await context.newPage();
      try {
        await page.goto(access.localAccessUrl);
        await page.getByRole('button', { name: /^Dev tests/ }).waitFor();
        await page.evaluate(async () => {
          const { emit } = await import('/src/lib/tauri.ts');
          await emit('open-flow:open-settings-section', 'general');
        });
        const button = page.getByRole('button', { name: 'Change dictation hotkey' });
        await button.waitFor();
        let before = await invoke('get_setting', { key: 'hotkey' });
        if (session.platform === 'linux') {
          for (const side of ['Left', 'Right']) {
            for (const superFirst of [false, true]) {
              await button.click();
              await page.locator('.keybind-btn.recording').waitFor();
              await page.evaluate(({ side, superFirst }) => {
                const codes = superFirst ? [`OS${side}`, 'ControlLeft'] : ['ControlLeft', `OS${side}`];
                for (const code of codes) {
                  window.dispatchEvent(new KeyboardEvent('keydown', { code, ctrlKey: code === 'ControlLeft' || !superFirst, bubbles: true }));
                }
                window.dispatchEvent(new KeyboardEvent('keyup', { code: `OS${side}`, ctrlKey: true, bubbles: true }));
              }, { side, superFirst });
              await page.waitForFunction(() => !document.querySelector('.keybind-btn')?.classList.contains('saving'));
              const expected = superFirst ? [`Meta${side}`, 'ControlLeft'] : ['ControlLeft', `Meta${side}`];
              assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), expected, 'WebKit OS codes must save as canonical Meta codes');
              assert.equal(await page.locator('#hotkey-help[role="alert"]').count(), 0);
            }
          }
          await page.reload();
          await page.getByRole('button', { name: /^Dev tests/ }).waitFor();
          await page.evaluate(async () => {
            const { emit } = await import('/src/lib/tauri.ts');
            await emit('open-flow:open-settings-section', 'general');
          });
          await button.waitFor();
          assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), ['MetaRight', 'ControlLeft']);
          await page.waitForFunction(() => document.querySelector('.keybind-btn')?.textContent?.trim() === 'Super + Ctrl');
          assert.equal((await button.innerText()).trim(), 'Super + Ctrl');
          before = await invoke('get_setting', { key: 'hotkey' });
        }
        await button.click();
        await page.locator('.keybind-btn.recording').waitFor();
        for (const code of codes) await page.keyboard.down(code);
        assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), before, 'Keydown must not save a partial chord');
        await page.keyboard.up(codes[0]);
        for (const code of codes.slice(1)) await page.keyboard.up(code);
        await page.waitForFunction(() => !document.querySelector('.keybind-btn')?.classList.contains('saving'));
        assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), codes);
        assert.equal((await button.innerText()).split(' + ').length, 5);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);

        await button.click();
        await page.locator('.keybind-btn.recording').waitFor();
        await page.keyboard.press('Escape');
        await button.waitFor();
        assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), codes, 'Escape must cancel without changing the binding');

        await button.click();
        await page.locator('.keybind-btn.recording').waitFor();
        const copyCodes = session.platform === 'macos' ? ['AltLeft', 'MetaLeft', 'KeyC'] : ['ControlLeft', 'AltLeft', 'KeyC'];
        for (const code of copyCodes) await page.keyboard.down(code);
        for (const code of [...copyCodes].reverse()) await page.keyboard.up(code);
        await page.locator('#hotkey-help[role="alert"]').waitFor();
        assert.match(await page.locator('#hotkey-help').innerText(), /already assigned/);
        assert.deepEqual(await invoke('get_setting', { key: 'hotkey' }), codes, 'The copy shortcut must remain reserved');
      } finally { await context.close(); }
    }
  } finally {
    await browser.close();
    await invoke('save_setting', { key: 'hotkey', value: original ?? (session.platform === 'macos' ? ['AltLeft', 'Space'] : ['ControlLeft', 'MetaLeft']) });
  }
});
