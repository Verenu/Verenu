import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to this worktree session access.json.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const base = access.localAccessUrl.split('#')[0].replace(/\/$/, '') + '/__verenu_dev';
async function invoke(command, args = {}) {
  const response = await fetch(`${base}/invoke`, { method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
  assert.equal(response.status, 200, `Command failed: ${command}`);
  return response.json();
}

test('custom provider editor persists, offers task models, renames, and removes selected references', { timeout: 120_000 }, async () => {
  const browser = await chromium.launch({ headless: true });
  const before = await invoke('get_all_settings');
  const keys = ['custom_providers', 'transcription_default_model', 'cleanup_default_model', 'transcription_fallback_models', 'cleanup_fallback_models', 'transcription_provider', 'cleanup_provider', 'transcription_model', 'cleanup_model', 'transcription_models_by_provider', 'cleanup_models_by_provider', 'advanced_model_ui'];
  let id;
  try {
    const page = await browser.newPage({ viewport: { width: 1320, height: 860 } });
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.goto(access.localAccessUrl);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Providers', exact: true }).click();
    await page.getByRole('button', { name: 'Create custom provider', exact: true }).click();
    const picker = page.getByRole('group', { name: 'Choose a provider preset' });
    await picker.getByRole('searchbox', { name: 'Search provider presets' }).fill('ollama');
    await picker.getByRole('button', { name: /^Mistral/ }).waitFor({ state: 'detached' });
    await picker.getByRole('searchbox', { name: 'Search provider presets' }).fill('scratch');
    await picker.getByRole('button', { name: /^Start from scratch/ }).click();
    const editor = page.getByRole('form', { name: 'Custom provider editor' });
    await editor.getByLabel('Name', { exact: true }).fill('Fixture endpoint');
    await editor.getByLabel(/^Base URL/).fill('http://localhost:8000/v1');
    await editor.getByLabel(/^Transcription model IDs/).fill('vendor/speech');
    await editor.getByLabel(/^Cleanup model IDs/).fill('vendor/chat');
    await editor.getByRole('switch', { name: 'Custom provider requires API key' }).click();
    await page.waitForTimeout(400);
    const evidence = path.join(path.dirname(accessFile), 'custom-provider-evidence');
    await fs.mkdir(evidence, { recursive: true });
    await editor.screenshot({ path: path.join(evidence, 'desktop-editor.png') });
    await page.setViewportSize({ width: 390, height: 844 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await editor.screenshot({ path: path.join(evidence, 'phone-editor.png') });
    await editor.getByRole('button', { name: 'Save provider', exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(evidence, 'phone-editor-actions.png') });
    await editor.getByRole('button', { name: 'Save provider', exact: true }).click();
    await page.getByRole('status').filter({ hasText: 'Provider saved' }).waitFor();
    const saved = await invoke('get_all_settings');
    const p = saved.custom_providers.find(p => p.name === 'Fixture endpoint');
    assert.ok(p); id = p.id;
    assert.deepEqual(p.cleanup_models, ['vendor/chat']);
    assert.equal(p.requires_key, false);
    assert.equal('api_key' in p, false);
    await page.setViewportSize({ width: 1320, height: 860 });
    await page.getByRole('button', { name: 'Models', exact: true }).click();
    // The advanced panel owns the model picker.
    await invoke('save_setting', { key: 'advanced_model_ui', value: true });
    await page.reload();
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Models', exact: true }).click();
    const transcriptionTile = page.locator('[data-setting-target="models-transcription"]');
    await transcriptionTile.getByRole('button', { name: /Change model/ }).click();
    await page.getByText('Fixture endpoint', { exact: true }).first().waitFor();
    await page.getByText('vendor/speech', { exact: true }).first().waitFor();
    await page.getByRole('dialog').screenshot({ path: path.join(evidence, 'desktop-picker.png') });
    assert.equal(await page.getByText('vendor/chat', { exact: true }).count(), 0);
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Providers', exact: true }).click();
    const section = page.getByRole('region', { name: 'Custom providers' });
    await section.getByRole('button', { name: /^Edit/ }).click();
    await page.getByLabel('Name', { exact: true }).fill('Renamed endpoint');
    await page.getByRole('button', { name: 'Save provider', exact: true }).click();
    await page.getByRole('status').filter({ hasText: 'Provider saved' }).waitFor();
    assert.equal((await invoke('get_all_settings')).custom_providers.find(p => p.id === id).name, 'Renamed endpoint');
    await invoke('save_setting', { key: 'transcription_default_model', value: `${id}/vendor/speech` });
    await invoke('save_setting', { key: 'transcription_fallback_models', value: ['groq/whisper-large-v3', `${id}/vendor/other`] });
    await section.getByRole('button', { name: 'Remove', exact: true }).click();
    await section.getByRole('button', { name: 'Remove provider', exact: true }).click();
    await page.getByRole('status').filter({ hasText: 'Provider removed' }).waitFor();
    const removed = await invoke('get_all_settings');
    assert.equal(removed.custom_providers.some(p => p.id === id), false);
    assert.equal(removed.transcription_default_model, 'groq/whisper-large-v3');
    assert.deepEqual(removed.transcription_fallback_models, []);
    assert.deepEqual(errors, []);
  } finally {
    for (const key of keys) {
      if (before[key] !== undefined && before[key] !== null) await invoke('save_setting', { key, value: before[key] });
    }
    await browser.close();
  }
});
