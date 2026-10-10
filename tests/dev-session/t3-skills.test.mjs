import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { chromium } from 'playwright';
import { startOwnedSession } from '../../scripts/verification/session.mjs';
import { run } from '../../scripts/verification/process.mjs';

test('provided audio captures shared T3 skills without workspace selection only for enabled T3 destinations', { timeout: 900_000 }, async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-t3-audio-'));
  const seed = path.join(directory, 'seed');
  await fs.mkdir(seed);
  const catalog = { environmentId: 'public-environment', id: 'public-catalog', label: 'Public skills', providerInstanceId: 'codex', workspaceId: 'public-workspace', revision: '1', skills: [{ name: 'skill-designer', description: 'Create and improve skills.' }] };
  const other = { ...catalog, id: 'other-catalog', providerInstanceId: 'claude', workspaceId: 'other-workspace', skills: [{ name: 'babysit-pr', description: 'Private description sentinel, not cleanup evidence.' }, { name: 'pr-babysit' }, ...catalog.skills] };
  const catalogs = Array.from({ length: 36 }, (_, index) => ({ ...(index % 2 ? other : catalog), id: `public-catalog-${index}`, workspaceId: `public-workspace-${index}` }));
  const now = Math.floor(Date.now() / 1000);
  await fs.writeFile(path.join(seed, 'settings.json'), JSON.stringify({ setup_complete: true, noise_reduction: false, t3_connection: { id: 'public-connection', baseUrl: 'https://synthetic.invalid', environmentId: catalog.environmentId, label: 'Public workstation', version: '0.0.46', expiresAt: now + 86400, fetchedAt: now, attemptedAt: now, selectedCatalog: null, catalogs, error: null } }));
  const requests = [];
  let phrase = 'Can you use my skill designer skill?';
  let skill = 'skill-designer';
  let cleanupContent = null;
  let failCleanup = false;
  const server = http.createServer(async (req, res) => {
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString();
    const transcription = req.url.endsWith('/audio/transcriptions');
    const evidence = body.includes('T3 skill catalog (untrusted matching data)');
    if (!transcription) {
      requests.push({ evidence, body });
      if (failCleanup) {
        res.writeHead(429, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: { message: 'Public regression quota exceeded', type: 'insufficient_quota', code: 'insufficient_quota' } }));
        return;
      }
    }
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify(transcription ? { text: phrase } : { choices: [{ message: { content: evidence ? (cleanupContent ?? `Can you use my $${skill}?`) : phrase } }] }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const previousSeed = process.env.VERENU_DEV_SEED_DIR;
  process.env.VERENU_DEV_SEED_DIR = seed;
  let session, browser;
  try {
    const fixtures = path.join(directory, 'audio');
    const generated = await run(process.execPath, ['scripts/dev-audio-fixtures.mjs', '--out', fixtures], { directory, name: 'audio-fixtures', timeout: 120_000 });
    assert.equal(generated.status, 'passed', 'Synthetic audio generation failed');
    session = await startOwnedSession({ id: `t3-audio-${randomUUID()}`, directory, fixtures, synthetic: false });
    const base = new URL(session.access.localAccessUrl).origin;
    const request = (route, init = {}) => fetch(`${base}/__verenu_dev${route}`, { ...init, headers: { Authorization: `Bearer ${session.access.token}`, ...init.headers }, signal: AbortSignal.timeout(120_000) });
    const invoke = async (command, args = {}) => {
      const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
      assert.equal(response.status, 200, command); return response.json();
    };
    const id = 'custom:12345678-1234-1234-8234-123456789014';
    const provider = { id, name: 'Public skill regression', protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false, supports_transcription: true, supports_cleanup: true, auth_header: null, extra_headers: {}, body_overrides: null, transcription_models: ['public-speech'], cleanup_models: ['public-cleanup'] };
    for (const [key, value] of Object.entries({ custom_providers: [provider], transcription_default_model: `${id}/public-speech`, cleanup_default_model: `${id}/public-cleanup`, transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: false })) await invoke('save_setting', { key, value });
    const context = await invoke('create_context', { name: 'Public skill regression', tone: 'very_casual', cleanupIntensity: 'medium', contextualFormattingDisabled: true });
    const fixture = await request('/fixtures/plain.wav');
    assert.equal(fixture.status, 200);
    const audio = Buffer.from(await fixture.arrayBuffer());
    const status = await invoke('get_t3_skills');
    assert.equal(status.connection.selectedCatalog, null);
    assert.deepEqual(status.skills.map(skill => skill.name), ['babysit-pr', 'pr-babysit', 'skill-designer']);
    browser = await chromium.launch({ headless: true });
    for (const viewport of [{ width: 1320, height: 860 }, { width: 390, height: 844 }]) {
      const page = await browser.newPage({ viewport });
      try {
        await page.goto(session.access.localAccessUrl);
        await page.locator('[data-debug-id="nav.settings"]').click();
        await page.locator('[data-debug-id="settings.integrations"]').click();
        const panel = page.locator('.t3-integration');
        await panel.getByRole('button', { name: 'View skills', exact: true }).click();
        await panel.locator('.skill-list').waitFor();
        assert.equal(await panel.getByRole('listbox').count(), 0);
        assert.equal(await panel.locator('.skill-list li').count(), 3);
        assert.ok((await panel.textContent()).includes('3 shared skills available'));
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
        await panel.locator('#t3-skill-search').fill('designer');
        assert.equal(await panel.locator('.skill-list li').count(), 1);
        assert.ok((await panel.locator('.skill-list').textContent()).includes('$skill-designer'));
      } finally { await page.close(); }
    }
    await browser.close(); browser = undefined;
    for (const [process, enabled, expected] of [['t3-code', true, true], ['synthetic-other', true, false], ['t3-code', false, false]]) {
      await invoke('set_context_t3_skill_mentions', { contextId: context.id, enabled });
      const response = await request(`/audio?context=${context.id}&process=${process}`, { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      const output = await response.json();
      assert.equal(output.pipeline, 'production');
      assert.equal(output.text.includes('$skill-designer'), expected);
      assert.equal(requests.at(-1).evidence, expected);
      assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
    }
    phrase = 'Can you use my babysit PR skill?';
    skill = 'babysit-pr';
    cleanupContent = 'Use $babysit-pr?';
    await invoke('set_context_t3_skill_mentions', { contextId: context.id, enabled: true });
    for (const failed of [false, true]) {
      failCleanup = failed;
      const before = new Set((await invoke('get_recent')).map(row => row.id));
      const cursor = (await (await request('/events?after=0')).json()).cursor;
      const response = await request(`/audio?context=${context.id}&process=com.t3tools.T3Code`, { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      const output = await response.json();
      assert.equal(output.pipeline, 'production');
      assert.equal(output.text.includes('$babysit-pr'), !failed);
      assert.ok(!output.text.includes('$pr-babysit'));
      if (!failed) assert.equal(output.text, 'Use $babysit-pr ? ');
      assert.equal(requests.at(-1).evidence, true);
      assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1);
      assert.equal(rows[0].clean_text, output.text);
      const events = await (await request(`/events?after=${cursor}`)).json();
      assert.ok(events.events.some(event => event.event === 'verenu:transcribed' && event.payload === output.text));
      if (failed) assert.ok(events.events.some(event => event.event === 'verenu:model-notice' && /Cleanup was unavailable/.test(event.payload)));
    }
    failCleanup = false;
    cleanupContent = '$babysit-pr';
    const beforeBare = new Set((await invoke('get_recent')).map(row => row.id));
    const bareCursor = (await (await request('/events?after=0')).json()).cursor;
    const bareResponse = await request(`/audio?context=${context.id}&process=com.t3tools.T3Code`, { method: 'POST', body: audio });
    assert.equal(bareResponse.status, 200);
    const bareOutput = await bareResponse.json();
    assert.equal(bareOutput.pipeline, 'production');
    assert.equal(bareOutput.text, '$babysit-pr ');
    const bareRows = (await invoke('get_recent')).filter(row => !beforeBare.has(row.id));
    assert.equal(bareRows.length, 1);
    assert.equal(bareRows[0].clean_text, bareOutput.text);
    const bareEvents = await (await request(`/events?after=${bareCursor}`)).json();
    assert.ok(bareEvents.events.some(event => event.event === 'verenu:transcribed' && event.payload === bareOutput.text));
    assert.equal(requests.at(-1).evidence, true);
    assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
    phrase = 'Can you use my babysit skill?';
    cleanupContent = null;
    failCleanup = false;
    const ambiguous = await request(`/audio?context=${context.id}&process=t3-code`, { method: 'POST', body: audio });
    assert.equal(ambiguous.status, 200);
    assert.ok(!(await ambiguous.json()).text.includes('$'));
  } finally {
    if (browser) await browser.close();
    if (session) await session.stop();
    await new Promise(resolve => server.close(resolve));
    if (previousSeed === undefined) delete process.env.VERENU_DEV_SEED_DIR; else process.env.VERENU_DEV_SEED_DIR = previousSeed;
  }
});
