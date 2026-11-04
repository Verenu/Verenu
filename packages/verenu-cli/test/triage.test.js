import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import { join } from 'node:path';
import { checks } from '../src/checks/index.js';
import { fix } from '../src/fixes/index.js';
import { bundle } from '../src/bundle.js';
import { report } from '../src/report.js';
import { nativeChecks } from '../src/checks/native.js';
import { createHash } from 'node:crypto';

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
  for (const name of ['cache', 'provider-keys', 'terms', 'network', 'logs']) assert.equal(by[name].status, 'warn', name);
  assert.equal(by.models.status, 'warn');
  assert.ok(['warn', 'pass'].includes(by.keyring.status));
  assert.ok(['warn', 'pass'].includes(by.launch.status));
  assert.match(report(results), /FAIL settings/);
  assert.doesNotMatch(bundle([...results, {name: 'secret', status: 'pass', message: 'private transcript'}]), /private transcript|secret/);
});

test('invalid settings reset preserves the original and valid settings are refused', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  await writeFile(join(dataDir, 'settings.json'), '{broken');
  const backup = await fix(dataDir, 'reset-settings');
  assert.equal(await readFile(join(backup, 'settings.json'), 'utf8'), '{broken');
  assert.equal(await readFile(join(dataDir, 'settings.json'), 'utf8'), '{}\n');
  await assert.rejects(fix(dataDir, 'reset-settings'), /valid/);
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

test('native diagnostics detect executable, keyring, provider and partial model failures', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  await mkdir(join(dataDir, 'models'));
  await writeFile(join(dataDir, 'models', 'model.partial'), 'partial');
  await mkdir(join(dataDir, 'logs'));
  await writeFile(join(dataDir, 'logs', 'app.log'), 'Secret Service unavailable\npanicked at startup');
  const run = () => ({status: 0});
  const first = Object.fromEntries((await nativeChecks({ dataDir, settings: { transcription_provider: 'groq' }, platform: 'linux', env: { VERENU_BINARY: process.execPath }, run })).map(r => [r.name, r]));
  assert.equal(first.launch.status, 'fail');
  assert.equal(first.keyring.status, 'fail');
  assert.equal(first.models.status, 'fail');
  assert.equal(first['provider-keys'].status, 'warn');
  await writeFile(join(dataDir, 'logs', 'app.log'), 'normal startup');
  await rm(join(dataDir, 'models', 'model.partial'));
  const second = Object.fromEntries((await nativeChecks({ dataDir, settings: { transcription_provider: 'groq' }, platform: 'linux', env: { VERENU_BINARY: process.execPath }, run })).map(r => [r.name, r]));
  assert.equal(second.launch.status, 'pass');
  assert.equal(second.keyring.status, 'pass');
  assert.equal(second.models.status, 'pass');
  assert.equal(second['provider-keys'].status, 'pass');
  const missing = Object.fromEntries((await nativeChecks({ dataDir, settings: {transcription_provider: 'groq'}, platform: 'linux', env: {VERENU_BINARY: '/missing'}, run: (cmd, args) => ({status: args[0] === 'lookup' ? 1 : 0}) })).map(r => [r.name, r]));
  assert.equal(missing.launch.status, 'warn');
  assert.equal(missing['provider-keys'].status, 'fail');
});

test('model manifest hash and size mismatches are reported', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const dir = join(dataDir, 'models');
  await mkdir(dir);
  await writeFile(join(dir, 'model.bin'), 'model');
  await writeFile(join(dir, 'manifest.json'), JSON.stringify({ files: [{ name: 'model.bin', size: 5, sha256: createHash('sha256').update('model').digest('hex') }] }));
  const check = async () => (await nativeChecks({dataDir, env: {}, platform: 'other'})).find(r => r.name === 'models');
  assert.match((await check()).message, /1 manifest artifacts verified/);
  await writeFile(join(dir, 'model.bin'), 'wrong');
  assert.equal((await check()).status, 'fail');
});
