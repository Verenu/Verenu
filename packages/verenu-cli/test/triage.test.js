import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import { join } from 'node:path';
import { checks } from '../src/checks/index.js';
import { fix } from '../src/fixes/index.js';
import { bundle } from '../src/bundle.js';
import { report } from '../src/report.js';

test('fixtures report missing, corrupt, and error states without a network request', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  await writeFile(join(dataDir, 'settings.json'), '{bad');
  await writeFile(join(dataDir, 'verenu.db'), 'invalid');
  await mkdir(join(dataDir, 'icon-cache'));
  await mkdir(join(dataDir, 'models'));
  await writeFile(join(dataDir, 'models', 'incomplete'), 'fixture');
  await mkdir(join(dataDir, 'logs'));
  await writeFile(join(dataDir, 'logs', 'app.log'), 'ERROR private transcript');
  let calls = 0;
  const results = await checks({ dataDir, env: { VERENU_CONVEX_URL: 'https://example.com' }, fetcher: async () => { calls++; return {ok: true}; } });
  const by = Object.fromEntries(results.map(r => [r.name, r]));
  assert.equal(calls, 0);
  assert.equal(by.settings.status, 'fail');
  assert.notEqual(by.sqlite.status, 'pass');
  for (const name of ['cache', 'models', 'keyring', 'provider-keys', 'terms', 'network', 'process', 'logs']) assert.equal(by[name].status, 'warn', name);
  assert.equal(by.launch.status, 'pass');
  assert.match(report(results), /FAIL settings/);
  assert.doesNotMatch(bundle([...results, {name: 'secret', status: 'pass', message: 'private transcript'}]), /private transcript|secret/);
});

test('explicit network opt in and accepted terms permit a request', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  let calls = 0;
  const options = { dataDir, env: {VERENU_CONVEX_URL: 'https://example.com'}, fetcher: async () => { calls++; return {ok: true}; } };
  await checks({...options, checkNetwork: true});
  await writeFile(join(dataDir, 'terms-consent.json'), '{"accepted":true}');
  await checks(options);
  assert.equal(calls, 2);
});

test('cache fix backs up before moving and does not touch transcripts', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  await mkdir(join(dataDir, 'icon-cache'));
  await writeFile(join(dataDir, 'icon-cache', 'fixture'), 'cache');
  await writeFile(join(dataDir, 'verenu.db'), 'transcript fixture');
  const backup = await fix(dataDir, 'clear-caches');
  assert.equal(await readFile(join(backup, 'icon-cache', 'fixture'), 'utf8'), 'cache');
  assert.equal(await readFile(join(dataDir, 'verenu.db'), 'utf8'), 'transcript fixture');
  await assert.rejects(fix(dataDir, 'delete-transcripts'));
});
