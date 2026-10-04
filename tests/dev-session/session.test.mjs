import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { sourceIdentity } from '../../scripts/verification/identity.mjs';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to the private access.json produced by npm run dev:session. These tests require a real running Rust session.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const base = new URL(access.localAccessUrl).origin;
const headers = { Authorization: `Bearer ${access.token}` };
const results = [];
const identity = sourceIdentity();
let metadata;

async function request(route, init = {}) {
  return fetch(`${base}/__verenu_dev${route}`, { ...init, headers: { ...headers, ...init.headers }, signal: AbortSignal.timeout(120_000) });
}
async function invoke(command, args = {}) {
  const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
  assert.equal(response.status, 200, `Native command failed: ${command}`);
  return response.json();
}
function check(name, run) {
  test(name, { timeout: 180_000 }, async (t) => {
    const row = { name, status: 'failed' }; results.push(row);
    await run(t, row);
    if (row.status !== 'skipped') row.status = 'passed';
  });
}
after(async () => {
  await fs.writeFile(path.join(path.dirname(accessFile), 'verification.json'), JSON.stringify({ schemaVersion: 2, identity, session: metadata ? { id: metadata.id, branch: metadata.branch, commit: metadata.commit, fingerprint: metadata.fingerprint, transport: metadata.transport, privateHistory: metadata.privateHistory } : null, checkedAt: new Date().toISOString(), checks: results, native: { status: 'not-tested', reason: 'Browser verification does not establish hotkeys, native insertion, OS permissions, or pill placement.' } }, null, 2), { mode: 0o600 });
});

check('session uses its own real Rust backend', async () => {
  const response = await request('/session'); assert.equal(response.status, 200);
  metadata = await response.json();
  assert.equal(metadata.transport, 'rust-live');
  assert.equal(metadata.fingerprint, identity.fingerprint, 'Restart the Rust backend after source edits');
  assert.equal(metadata.worktree, identity.worktree, 'Use this worktree\'s own session');
  assert.equal(metadata.capabilities.nativeInjection, false);
  assert.equal(metadata.capabilities.credentialWrites, false);
  const settings = await invoke('get_all_settings');
  assert.equal(Object.keys(settings).some((key) => key.startsWith('api_key_')), false);
});
check('authentication, origin checks, and native command restrictions hold', async () => {
  assert.equal((await fetch(`${base}/__verenu_dev/session`)).status, 401);
  assert.equal((await request('/session', { headers: { Origin: 'https://untrusted.invalid' } })).status, 403);
  if (!metadata.capabilities.hostMicrophone) {
    assert.deepEqual(await invoke('get_microphones'), [], 'Browser-only sessions must not enumerate host microphones');
  }
  for (const command of ['save_api_key', 'delete_api_key', 'export_data', 'plugin:shell|open']) {
    const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args: {} }) });
    assert.equal(response.status, 403);
  }
});
check('Context edits persist in the real session database', async () => {
  const name = `Test ${Date.now()}`;
  const created = await invoke('create_context', { name, contextualFormattingDisabled: false });
  try {
    assert.equal((await invoke('get_contexts')).some((row) => row.id === created.id && row.name === name), true);
    await invoke('update_context', { contextId: created.id, name: `${name} edited` });
    assert.equal((await invoke('get_contexts')).some((row) => row.id === created.id && row.name.endsWith('edited')), true);
  } finally { await invoke('delete_context', { contextId: created.id }); }
});
check('local model selections and provider maps persist through the real settings validator', async () => {
  const before = await invoke('get_all_settings');
  const values = {
    cleanup_models_by_provider: {
      groq: [], openai: [], google: [], assemblyai: [], openrouter: [], xai: [],
      local: ['qwen2.5-7b-instruct'],
    },
    transcription_models_by_provider: {
      groq: [], openai: [], google: [], assemblyai: [], openrouter: [], xai: [],
      local: ['parakeet-v3'],
    },
    cleanup_default_model: 'local/qwen2.5-7b-instruct',
    transcription_default_model: 'local/parakeet-v3',
  };
  try {
    for (const [key, value] of Object.entries(values)) await invoke('save_setting', { key, value });
    const saved = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(values)) assert.deepEqual(saved[key], value);
  } finally {
    for (const key of Object.keys(values)) {
      // A fresh isolated session can have no persisted provider map yet; an
      // empty map is the equivalent default and passes the native validator.
      const value = before[key] ?? (key.endsWith('_by_provider') ? {} : null);
      await invoke('save_setting', { key, value });
    }
  }
});
check('malformed audio fails without invoking providers', async () => {
  const before = (await (await request('/session')).json()).runs;
  assert.equal((await request('/audio', { method: 'POST', body: 'not a WAV' })).status, 400);
  const after = (await (await request('/session')).json()).runs;
  assert.equal(before, after);
});
check('custom Context artwork survives native saves and duplication', async () => {
  const artwork = (kind, text) => 'custom-icon:' + JSON.stringify({ kind, text, background: '#164e63', foreground: '#38bdf8' });
  const icon = artwork('letters', 'WE');
  const created = await invoke('create_context', { name: 'Custom icon regression', icon, contextualFormattingDisabled: false });
  let duplicate;
  try {
    await invoke('update_context_color', { contextId: created.id, color: '#a855f7' });
    duplicate = await invoke('duplicate_context', { contextId: created.id });
    assert.equal(duplicate.icon, icon);
    assert.equal(duplicate.color, '#a855f7');
    const emoji = artwork('emoji', '👩🏽‍💻');
    await invoke('update_context_settings', { contextId: created.id, icon: emoji, contextualFormattingDisabled: false });
    const saved = (await invoke('get_contexts')).find(row => row.id === created.id);
    assert.equal(saved.icon, emoji);
    assert.equal(saved.color, '#a855f7');
  } finally {
    if (duplicate) await invoke('delete_context', { contextId: duplicate.id });
    await invoke('delete_context', { contextId: created.id });
  }
});
check('production audio gates reject silence and deliver backend events', async () => {
  const before = await (await request('/events?after=0')).json();
  const fixture = await request('/fixtures/silence.wav'); assert.equal(fixture.status, 200, 'Generate synthetic fixtures before running session regression');
  const response = await request('/audio', { method: 'POST', body: await fixture.arrayBuffer() });
  assert.equal(response.status, 422);
  const events = await (await request(`/events?after=${before.cursor}`)).json();
  assert.equal(events.events.some((event) => event.event === 'verenu:pill-state'), true);
});
check('desktop and phone browsers show the live session test controls', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const viewport of [{ width: 1320, height: 860 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport });
      const page = await context.newPage();
      const errors = []; page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(access.localAccessUrl);
      await page.getByRole('button', { name: /^Dev tests/ }).waitFor();
      await page.getByRole('button', { name: /^Dev tests/ }).click();
      const panel = page.getByRole('complementary', { name: 'Dev session tests' }); await panel.waitFor();
      assert.equal(await panel.getByText('Real Rust backend · isolated database · live providers').isVisible(), true);
      assert.equal(await panel.getByRole('button', { name: 'Run fixture' }).isVisible(), true);
      const box = await panel.boundingBox();
      assert.equal(box.x >= 0 && box.x + box.width <= viewport.width, true, 'Test panel must fit phone and desktop viewports');
      assert.equal((await page.evaluate(() => location.hash)).includes('session-token'), false);
      assert.equal(await page.evaluate(async () => {
        const { listen, emit } = await import('/src/lib/tauri.ts');
        let received = false;
        const stop = await listen('open-flow:open-settings-section', () => { received = true; });
        await emit('open-flow:open-settings-section', 'general');
        stop();
        return received;
      }), true, 'Frontend-emitted events must reach session listeners');
      assert.equal(errors.length, 0, `Browser has an uncaught application error: ${errors.join('; ')}`);
      await context.close();
    }
  } finally { await browser.close(); }
});
check('missing session authentication never falls back to browser mocks', async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage(); await page.goto(base);
    await page.getByRole('heading', { name: 'Connect to your dev session' }).waitFor();
    assert.equal(await page.locator('.app').count(), 0);
  } finally { await browser.close(); }
});
check('sub-app pattern Enter saves on desktop and Sub-apps stay hidden on Android', async () => {
  const browser = await chromium.launch({ headless: true });
  const createdIds = [];
  try {
    for (const viewport of [{ width: 1320, height: 860 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport, reducedMotion: 'reduce' });
      if (viewport.width < 700) {
        await context.addInitScript(() => { window.__VERENU_ANDROID__ = true; });
      }
      try {
        const page = await context.newPage();
        await page.goto(access.localAccessUrl);
        await page.getByRole('button', { name: /^Dev tests/ }).waitFor();
        if (viewport.width < 700) {
          await page.locator('.mobile-nav').getByRole('button', { name: 'Settings', exact: true }).click();
          const sections = page.getByRole('navigation', { name: 'Settings sections' });
          await sections.waitFor();
          assert.equal(await sections.getByRole('button', { name: 'Sub-apps', exact: true }).count(), 0,
            'Android Settings must hide the desktop-only Sub-apps section');
          const reopenState = await page.evaluate(async () => {
            const { appStore } = await import('/src/lib/stores.svelte.ts');
            const { emit } = await import('/src/lib/tauri.ts');
            const flush = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            appStore.settingsMobileList = false;
            appStore.settingsOpen = false;
            await flush();
            const listAfterClose = appStore.settingsMobileList;
            appStore.settingsOpen = true;
            await flush();
            const reopenedView = document.querySelector('.settings-page')?.getAttribute('data-mobile-view');
            await emit('open-flow:open-settings-section', 'general');
            await flush();
            const targetedView = document.querySelector('.settings-page')?.getAttribute('data-mobile-view');
            await emit('open-flow:open-settings-section', '');
            await flush();
            return {
              listAfterClose,
              reopenedView,
              targetedView,
              unqualifiedView: document.querySelector('.settings-page')?.getAttribute('data-mobile-view'),
            };
          });
          assert.deepEqual(reopenState, {
            listAfterClose: true,
            reopenedView: 'list',
            targetedView: 'detail',
            unqualifiedView: 'list',
          }, 'Unqualified Settings entry shows the list, while a targeted entry shows its section');
          continue;
        }
        const label = `Synthetic sub-app ${viewport.width} ${Date.now()}`;
        // Supply a public synthetic capture instead of invoking native hotkeys.
        await page.evaluate(async (label) => {
          const { contextsStore } = await import('/src/lib/contextsStore.svelte.ts');
          contextsStore.subAppSheet = { mode: 'capture', capture: {
            executable: 'fixture-app', app_name: 'Fixture app',
            window_title: label, proposed_pattern: label,
          } };
        }, label);
        const dialog = page.getByRole('dialog', { name: 'New sub-app' });
        await dialog.waitFor();
        await dialog.getByRole('textbox', { name: 'Window title text' }).press('Enter');
        await dialog.waitFor({ state: 'hidden' }).catch(async (error) => {
          await page.screenshot({ path: path.join(path.dirname(accessFile), `sub-apps-save-failure-${viewport.width}.png`) });
          throw error;
        });
        const saved = (await invoke('get_sub_apps')).find((row) => row.title_pattern === label);
        assert.ok(saved, 'Enter from the pattern input must persist a sub-app');
        createdIds.push(saved.id);
        await page.getByRole('button', { name: 'Settings', exact: true }).click();
        await page.getByRole('button', { name: 'Sub-apps', exact: true }).filter({ visible: true }).click();
        await page.locator('.settings-h').filter({ hasText: /^General$/ }).waitFor({ state: 'detached' });
        await page.getByRole('heading', { name: 'Sub-apps', exact: true, level: 2 }).waitFor();
        assert.equal(await page.getByRole('heading', { name: 'Shortcut', exact: true, level: 3 }).isVisible(), true);
        await page.getByRole('button', { name: `Remove ${saved.label}`, exact: true }).waitFor();
        const panel = page.locator('.settings-page');
        const box = await panel.boundingBox();
        assert.ok(box && box.x >= 0 && box.x + box.width <= viewport.width, 'Sub-apps settings must fit the viewport');
        await page.screenshot({ animations: 'disabled', path: path.join(path.dirname(accessFile), `sub-apps-${viewport.width}.png`) });
      } finally { await context.close(); }
    }
  } finally {
    for (const id of createdIds) await invoke('delete_sub_app', { id });
    await browser.close();
  }
});
check('live synthetic corpus reaches providers, Context rules, events, and exact new history', async (t, row) => {
  if (process.env.VERENU_DEV_REQUIRE_LIVE !== '1') {
    row.status = 'skipped'; row.reason = 'Set VERENU_DEV_REQUIRE_LIVE=1 to require paid provider verification';
    t.skip(row.reason); return;
  }
  const corpus = JSON.parse(await fs.readFile(new URL('../fixtures/dev-audio.json', import.meta.url), 'utf8'));
  const context = await invoke('create_context', { name: 'Synthetic speech', tone: 'casual', cleanupIntensity: 'medium', contextualFormattingDisabled: true });
  const snippet = await invoke('create_snippet', { trigger: 'insert the test signature', expansion: 'Synthetic signature confirmed.', instructions: '', contextId: context.id });
  const vocabulary = await invoke('create_dictionary_entry', { term: 'Verenu', mistake: 'Verenoo', contextId: context.id });
  try {
    row.cases = [];
    for (const item of corpus) {
      const beforeHistory = new Set((await invoke('get_recent')).map((entry) => entry.id));
      const beforeEvents = await (await request('/events?after=0')).json();
      const fixture = await request(`/fixtures/${item.name}.wav`); assert.equal(fixture.status, 200);
      const response = await request(`/audio?context=${context.id}&process=synthetic-fixture`, { method: 'POST', body: await fixture.arrayBuffer() });
      assert.equal(response.status, 200, `Production dictation failed for ${item.name}; inspect private redacted events`);
      const result = await response.json();
      assert.equal(result.pipeline, 'production');
      const text = result.text.toLowerCase();
      const required = item.name === 'snippet' ? ['synthetic', 'signature', 'confirmed'] : item.contains;
      assert.ok(required.every((word) => text.includes(word.toLowerCase())), `Required meaning missing in ${item.name}`);
      if (item.name === 'correction') assert.equal(/\btuesday\b/i.test(result.text), false, 'Correction retained the superseded day');
      const created = (await invoke('get_recent')).filter((entry) => !beforeHistory.has(entry.id));
      assert.equal(created.length, 1, `Fixture ${item.name} must create exactly one history row`);
      assert.equal(created[0].clean_text, result.text, 'History must contain this run\'s exact returned output');
      const events = await (await request(`/events?after=${beforeEvents.cursor}`)).json();
      assert.ok(events.events.some((event) => event.event === 'verenu:transcribed' && event.payload === result.text), 'Missing matching completion event');
      row.cases.push({ fixture: item.name, historyId: created[0].id, status: 'passed' });
    }
  } finally {
    await invoke('remove_snippet', { id: snippet.id });
    await invoke('remove_dictionary_entry', { id: vocabulary.id });
    await invoke('delete_context', { contextId: context.id });
  }
});
