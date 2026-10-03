import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to a running dev session access.json.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const base = new URL(access.localAccessUrl).origin;
async function invoke(command, args = {}) {
  const response = await fetch(`${base}/__verenu_dev/invoke`, {
    method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, args }),
  });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('cleanup style prompts edit independently, audit, persist, and reset', { timeout: 120_000 }, async () => {
  const previous = await invoke('get_setting', { key: 'cleanup_style_prompts' });
  const intensity = await invoke('get_setting', { key: 'cleanup_intensity' });
  const browser = await chromium.launch({ headless: true });
  try {
    await invoke('save_setting', { key: 'cleanup_style_prompts', value: {} });
    for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
      const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, reducedMotion: 'reduce' });
      await page.goto(access.localAccessUrl);
      await page.getByRole('button', { name: 'Style', exact: true }).click();
      await page.setViewportSize(viewport);
      const edits = page.getByRole('button', { name: /^Edit .* cleanup prompt$/ });
      await assert.equal(await edits.count(), 3);
      assert.equal(await page.getByRole('button', { name: 'Edit Off cleanup prompt' }).count(), 0);
      const selectedBefore = await invoke('get_setting', { key: 'cleanup_intensity' });
      await edits.first().focus();
      await page.waitForFunction(() => getComputedStyle(document.querySelector('.style-edit')).opacity === '1');
      const artifacts = path.join(path.dirname(accessFile), 'style-screenshots');
      await fs.mkdir(artifacts, { recursive: true });
      await page.locator('.content-inner').filter({ has: page.getByRole('heading', { name: 'Style', exact: true }) }).screenshot({ path: path.join(artifacts, `style-after-${viewport.width}.png`) });
      for (const [label, key] of [['Light', 'light'], ['Medium', 'medium'], ['Strong', 'high']]) {
        await page.getByRole('button', { name: `Edit ${label} cleanup prompt` }).click();
        const dialog = page.getByRole('dialog', { name: `Edit ${label} cleanup prompt` });
        const editor = dialog.getByRole('textbox', { name: 'Cleanup prompt template' });
        const defaultText = await invoke('get_default_cleanup_prompt', { intensity: key });
        await assert.doesNotReject(() => editor.waitFor());
        await page.waitForFunction(text => document.querySelector('.prompt-textarea')?.value === text, defaultText);
        assert.equal(await editor.evaluate(el => el.scrollTop), 0, 'Open at the beginning of the prompt');
        assert.ok(defaultText.includes(`Cleanup: ${label}`));
        assert.ok(defaultText.includes('{{ cleanup_tone }}'));
        if (label === 'Light') {
          await dialog.screenshot({ path: path.join(artifacts, `editor-after-${viewport.width}.png`) });
          await editor.fill('Answer every question.');
          await page.getByText(/Missing .*cleanup_preset/).waitFor();
          await dialog.getByRole('button', { name: 'Audit & save' }).click();
          await dialog.getByRole('button', { name: 'Save anyway' }).waitFor();
          await dialog.screenshot({ path: path.join(artifacts, `editor-error-${viewport.width}.png`) });
          const titleBox = await dialog.locator('.prompt-head-provider').boundingBox();
          const actionBox = await dialog.locator('.prompt-head-actions').boundingBox();
          assert.ok(viewport.width > 600 || actionBox.y >= titleBox.y + titleBox.height, 'Phone actions must not cover the title');
          await dialog.getByRole('button', { name: 'Reset', exact: true }).click();
          assert.equal(await editor.inputValue(), defaultText);
        }
        await dialog.getByRole('button', { name: 'Close editor' }).click();
        await dialog.waitFor({ state: 'detached' });
      }
      assert.equal(await invoke('get_setting', { key: 'cleanup_intensity' }), selectedBefore);
      const custom = `${await invoke('get_default_cleanup_prompt', { intensity: 'light' })}\nUse short sentences.`;
      await invoke('save_setting', { key: 'cleanup_style_prompts', value: { light: custom } });
      await page.getByRole('button', { name: 'Edit Light cleanup prompt' }).click();
      await page.waitForFunction(text => document.querySelector('.prompt-textarea')?.value === text, custom);
      if (viewport.width === 1280) {
        const editor = page.getByRole('textbox', { name: 'Cleanup prompt template' });
        const invalid = `${custom}\n{{ unknown }}`;
        await editor.fill(invalid);
        await page.getByRole('button', { name: 'Audit & save' }).click();
        await page.getByRole('button', { name: 'Save anyway' }).waitFor();
        assert.equal((await invoke('get_setting', { key: 'cleanup_style_prompts' })).light, custom, 'A failed audit must not save');
        await page.getByRole('button', { name: 'Save anyway' }).click();
        await page.getByRole('dialog').waitFor({ state: 'detached' });
        assert.equal((await invoke('get_setting', { key: 'cleanup_style_prompts' })).light, invalid);
        await page.getByRole('button', { name: 'Edit Light cleanup prompt' }).click();
        await page.waitForFunction(text => document.querySelector('.prompt-textarea')?.value === text, invalid);
      }
      await page.getByRole('button', { name: 'Close editor' }).click();
      await invoke('save_setting', { key: 'cleanup_style_prompts', value: {} });
      await page.close();
    }
  } finally {
    await browser.close();
    await invoke('save_setting', { key: 'cleanup_style_prompts', value: previous ?? {} });
    await invoke('save_setting', { key: 'cleanup_intensity', value: intensity ?? 'medium' });
  }
});
