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
    const screenshots = process.env.VERENU_T3_SCREENSHOT_DIR;
    for (const viewport of [{ width: 1320, height: 860 }, { width: 390, height: 844 }]) {
      const context = await browser.newContext({ viewport });
      await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: base });
      const page = await context.newPage();
      try {
        await page.goto(session.access.localAccessUrl);
        await page.locator('[data-debug-id="nav.settings"]').click();
        await page.locator('[data-debug-id="settings.integrations"]').click();
        await page.waitForFunction(() => document.querySelector('[aria-label="T3 Code integration"]')?.getAttribute('aria-busy') === 'false');
        const panel = page.getByRole('region', { name: 'T3 Code integration', exact: true });
        const countButton = panel.getByRole('button', { name: '3 skills', exact: true });
        const actions = panel.getByRole('button', { name: 'T3 Code actions', exact: true });
        const before = await invoke('get_t3_skills');
        assert.equal(before.skills.length, 3);
        assert.equal(await panel.getByRole('button', { name: 'Connect T3 Code', exact: true }).count(), 0);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
        if (screenshots) {
          await fs.mkdir(screenshots, { recursive: true });
          await page.screenshot({ path: path.join(screenshots, `integrations-${viewport.width}.png`) });
        }

        await countButton.click();
        const dialog = page.getByRole('dialog', { name: 'Shared T3 skills', exact: true });
        await dialog.waitFor();
        await dialog.evaluate(async node => {
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
          await Promise.all(node.getAnimations().map(animation => animation.finished.catch(() => {})));
        });
        const list = dialog.getByRole('list', { name: 'Shared T3 skill names', exact: true });
        const tokens = async () => (await list.locator('code').allTextContents()).map(text => text.trim());
        assert.deepEqual(await tokens(), ['$babysit-pr', '$pr-babysit', '$skill-designer']);
        const dialogText = await dialog.innerText();
        assert.ok(!dialogText.includes('Private description sentinel'));
        assert.ok(!dialogText.includes('Create and improve skills.'));
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
        if (viewport.width <= 560) {
          const sheet = await dialog.boundingBox();
          assert.ok(sheet.width >= viewport.width - 1, 'bottom sheet spans the viewport width');
          assert.ok(sheet.y + sheet.height >= viewport.height - 1, 'bottom sheet is docked to the bottom edge');
        }
        if (screenshots) await page.screenshot({ path: path.join(screenshots, `t3-skills-dialog-${viewport.width}.png`) });

        const search = dialog.getByRole('searchbox', { name: 'Search skill names', exact: true });
        await search.fill('designer');
        assert.deepEqual(await tokens(), ['$skill-designer']);
        await search.fill('sentinel');
        assert.deepEqual(await tokens(), []);
        assert.ok((await dialog.innerText()).includes('No skill names match “sentinel”.'));
        await search.fill('$BABY');
        assert.deepEqual(await tokens(), ['$babysit-pr', '$pr-babysit']);
        await search.fill('');
        assert.equal((await tokens()).length, 3);

        await dialog.getByRole('button', { name: 'Copy $skill-designer', exact: true }).click();
        await dialog.getByRole('status').filter({ hasText: 'Copied $skill-designer.' }).waitFor();
        await page.waitForFunction(async () => (await navigator.clipboard.readText()) === '$skill-designer', null, { timeout: 5000 });

        await page.keyboard.press('Escape');
        await dialog.waitFor({ state: 'detached' });
        assert.equal(await countButton.evaluate(node => node === document.activeElement), true);
        assert.ok(await panel.isVisible());

        await actions.click();
        const menu = page.getByRole('menu', { name: 'T3 Code actions', exact: true });
        await menu.waitFor();
        assert.deepEqual((await menu.getByRole('menuitem').allTextContents()).map(text => text.trim()), ['Pull now', 'Reconnect', 'Disconnect']);
        await page.keyboard.press('Escape');
        await menu.waitFor({ state: 'detached' });
        assert.equal(await actions.getAttribute('aria-expanded'), 'false');
        assert.ok(await panel.isVisible());

        await actions.click();
        await menu.getByRole('menuitem', { name: 'Reconnect', exact: true }).click();
        const pairing = panel.locator('#t3-pairing-link');
        await pairing.waitFor();
        assert.equal(await pairing.evaluate(node => node === document.activeElement), true);
        assert.ok((await panel.innerText()).includes('Your current connection stays active until the new link connects.'));
        assert.equal(await countButton.count(), 1);
        await pairing.fill('https://synthetic.invalid/pair#token=synthetic');
        await panel.getByRole('button', { name: 'Connect T3 Code', exact: true }).click();
        await panel.getByRole('alert').waitFor();
        assert.equal(await pairing.inputValue(), '');
        const afterFailedPairing = await invoke('get_t3_skills');
        assert.equal(afterFailedPairing.connection.id, before.connection.id);
        assert.deepEqual(afterFailedPairing.skills, before.skills);
        await panel.getByRole('button', { name: 'Cancel', exact: true }).click();
        await pairing.waitFor({ state: 'detached' });
        assert.equal(await countButton.isVisible(), true);
        assert.ok(!(await panel.innerText()).includes('Using saved skills'), 'A failed replacement pairing is not a failed skill pull');

        await actions.click();
        await menu.getByRole('menuitem', { name: 'Disconnect', exact: true }).click();
        const confirm = page.getByRole('dialog', { name: 'Disconnect T3 Code?', exact: true });
        await confirm.waitFor();
        const confirmText = await confirm.innerText();
        assert.ok(confirmText.includes('native credential store'));
        assert.ok(confirmText.includes('deletes the 3 imported skill names'));
        await page.keyboard.press('Escape');
        await confirm.waitFor({ state: 'detached' });
        assert.equal(await actions.evaluate(node => node === document.activeElement), true);
        assert.ok(await panel.isVisible());
        assert.equal((await invoke('get_t3_skills')).connection.id, before.connection.id);
      } finally { await context.close(); }
    }
    await browser.close(); browser = undefined;
    for (const [process, enabled, expected] of [['t3-code', true, true], ['com.monke.t3code.grokbot', true, true], ['synthetic-other', true, false], ['t3-code', false, false]]) {
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
    cleanupContent = 'Use $BABYSIT-PR?';
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
    cleanupContent = 'Use ($BABYSIT-PR)';
    const beforeWrapped = new Set((await invoke('get_recent')).map(row => row.id));
    const wrappedCursor = (await (await request('/events?after=0')).json()).cursor;
    const wrappedResponse = await request(`/audio?context=${context.id}&process=com.t3tools.T3Code`, { method: 'POST', body: audio });
    assert.equal(wrappedResponse.status, 200);
    const wrappedOutput = await wrappedResponse.json();
    assert.equal(wrappedOutput.pipeline, 'production');
    assert.equal(wrappedOutput.text, 'Use ( $babysit-pr ) ');
    const wrappedRows = (await invoke('get_recent')).filter(row => !beforeWrapped.has(row.id));
    assert.equal(wrappedRows.length, 1);
    assert.equal(wrappedRows[0].clean_text, wrappedOutput.text);
    const wrappedEvents = await (await request(`/events?after=${wrappedCursor}`)).json();
    assert.ok(wrappedEvents.events.some(event => event.event === 'verenu:transcribed' && event.payload === wrappedOutput.text));
    assert.equal(requests.at(-1).evidence, true);
    assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
    phrase = 'Can you mention the HOME variable?';
    cleanupContent = 'Mention $HOME';
    const beforeVariable = new Set((await invoke('get_recent')).map(row => row.id));
    const variableCursor = (await (await request('/events?after=0')).json()).cursor;
    const variableResponse = await request(`/audio?context=${context.id}&process=com.t3tools.T3Code`, { method: 'POST', body: audio });
    assert.equal(variableResponse.status, 200);
    const variableOutput = await variableResponse.json();
    assert.equal(variableOutput.pipeline, 'production');
    assert.equal(variableOutput.text, 'Mention $HOME');
    const variableRows = (await invoke('get_recent')).filter(row => !beforeVariable.has(row.id));
    assert.equal(variableRows.length, 1);
    assert.equal(variableRows[0].clean_text, variableOutput.text);
    const variableEvents = await (await request(`/events?after=${variableCursor}`)).json();
    assert.ok(variableEvents.events.some(event => event.event === 'verenu:transcribed' && event.payload === variableOutput.text));
    assert.equal(requests.at(-1).evidence, true);
    assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
    phrase = 'Can you set it to dollar HOME?';
    cleanupContent = 'Set it to $HOME';
    const beforeDollarVariable = new Set((await invoke('get_recent')).map(row => row.id));
    const dollarVariableCursor = (await (await request('/events?after=0')).json()).cursor;
    const dollarVariableResponse = await request(`/audio?context=${context.id}&process=com.t3tools.T3Code`, { method: 'POST', body: audio });
    assert.equal(dollarVariableResponse.status, 200);
    const dollarVariableOutput = await dollarVariableResponse.json();
    assert.equal(dollarVariableOutput.pipeline, 'production');
    assert.equal(dollarVariableOutput.text, 'Set it to $HOME');
    const dollarVariableRows = (await invoke('get_recent')).filter(row => !beforeDollarVariable.has(row.id));
    assert.equal(dollarVariableRows.length, 1);
    assert.equal(dollarVariableRows[0].clean_text, dollarVariableOutput.text);
    const dollarVariableEvents = await (await request(`/events?after=${dollarVariableCursor}`)).json();
    assert.ok(dollarVariableEvents.events.some(event => event.event === 'verenu:transcribed' && event.payload === dollarVariableOutput.text));
    assert.equal(requests.at(-1).evidence, true);
    assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
    phrase = 'Please prepare the quarterly report.';
    cleanupContent = 'Use $BABYSIT-PR';
    failCleanup = false;
    const listedSelection = await request(`/audio?context=${context.id}&process=t3-code`, { method: 'POST', body: audio });
    assert.equal(listedSelection.status, 200);
    assert.equal((await listedSelection.json()).text, 'Use $babysit-pr ');
    assert.equal(requests.at(-1).evidence, true);
    assert.ok(!requests.at(-1).body.includes('Private description sentinel'));
  } finally {
    if (browser) await browser.close();
    if (session) await session.stop();
    await new Promise(resolve => server.close(resolve));
    if (previousSeed === undefined) delete process.env.VERENU_DEV_SEED_DIR; else process.env.VERENU_DEV_SEED_DIR = previousSeed;
  }
});
