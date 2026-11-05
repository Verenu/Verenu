import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import os from 'node:os';
import { checks, checkIds } from '../src/checks/index.js';
import { nativeChecks } from '../src/checks/native.js';
import { stalePidFiles } from '../src/checks/probes.js';
import { generate } from '../scripts/generate-model-manifest.js';

const tempDir = async (t, prefix = 'verenu-triage-') => {
  const dir = await mkdtemp(join(os.tmpdir(), prefix));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
};

// One failing fixture per check. Each fixture is applied on top of a passing baseline.
const fixtures = {
  settings: async d => writeFile(join(d, 'settings.json'), '{invalid'),
  sqlite: async d => writeFile(join(d, 'verenu.db'), 'broken database'),
  cache: async d => mkdir(join(d, 'icon-cache')),
  terms: async d => writeFile(join(d, 'terms-consent.json'), '{"accepted":false}'),
  launch: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'panicked at startup'); },
  keyring: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'Secret Service unavailable'); },
  'provider-keys': async d => writeFile(join(d, 'settings.json'), '{"transcription_provider":"groq"}'),
  models: async d => { await mkdir(join(d, 'models')); await writeFile(join(d, 'models', 'bad.partial'), 'partial'); },
  process: async d => writeFile(join(d, 'verenu.pid'), '2147483647'),
  logs: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'ERROR fixture'); },
};

test('check coverage guard', () => assert.deepEqual(Object.keys(fixtures).sort(), [...checkIds].sort()));
test('no check id names a network request', () => assert.equal(checkIds.includes('network'), false));

for (const [id, makeFailure] of Object.entries(fixtures)) {
  test(`${id}: passing and failure fixtures`, async t => {
    const dataDir = await tempDir(t, 'verenu-check-');
    await writeFile(join(dataDir, 'settings.json'), '{"transcription_provider":"local"}');
    await writeFile(join(dataDir, 'terms-consent.json'), '{"accepted":true}');
    assert.equal(spawnSync('sqlite3', [join(dataDir, 'verenu.db'), 'CREATE TABLE fixture (id INTEGER);']).status, 0);
    let failing = false;
    const options = {
      dataDir,
      platform: 'linux',
      env: { VERENU_BINARY: process.execPath },
      run: (_cmd, args) => ({ status: failing && id === 'provider-keys' && args[0] === 'lookup' ? 1 : 0, stdout: '', stderr: '' }),
    };
    const pass = await checks(options);
    assert.deepEqual(pass.map(r => r.name).sort(), [...checkIds].sort());
    assert.equal(pass.find(r => r.name === id).status, 'pass');
    await makeFailure(dataDir);
    failing = true;
    const failed = await checks(options);
    assert.notEqual(failed.find(r => r.name === id).status, 'pass');
  });
}

test('checks report missing and corrupt data without writing to it', async t => {
  const dataDir = await tempDir(t);
  await writeFile(join(dataDir, 'settings.json'), '{bad');
  await writeFile(join(dataDir, 'verenu.db'), 'invalid');
  await mkdir(join(dataDir, 'icon-cache'));
  await mkdir(join(dataDir, 'models'));
  await writeFile(join(dataDir, 'models', 'incomplete'), 'fixture');
  await mkdir(join(dataDir, 'logs'));
  await writeFile(join(dataDir, 'logs', 'app.log'), 'ERROR private transcript');
  const before = await readFile(join(dataDir, 'settings.json'), 'utf8');
  const results = await checks({ dataDir, env: {} });
  const by = Object.fromEntries(results.map(r => [r.name, r]));
  assert.equal(by.settings.status, 'fail');
  assert.notEqual(by.sqlite.status, 'pass');
  for (const name of ['cache', 'terms', 'logs']) assert.equal(by[name].status, 'warn', name);
  assert.equal(by.models.status, 'warn');
  assert.equal(await readFile(join(dataDir, 'settings.json'), 'utf8'), before);
  assert.ok(!JSON.stringify(results).includes('private transcript'), 'results must not carry log text');
});

test('native diagnostics detect executable, keyring, provider and partial model failures', async t => {
  const dataDir = await tempDir(t);
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

test('stale PID files are reported and not removed', { skip: process.platform !== 'linux' }, async t => {
  const dataDir = await tempDir(t);
  await writeFile(join(dataDir, 'verenu.pid'), '2147483647');
  assert.deepEqual((await stalePidFiles(dataDir)).map(p => p.name), ['verenu.pid']);
  const processCheck = (await nativeChecks({ dataDir, settings: {}, platform: 'linux', env: {}, run: () => ({status: 0}) })).find(r => r.name === 'process');
  assert.equal(processCheck.status, 'warn');
  assert.match(processCheck.message, /PID file/);
  assert.equal(await readFile(join(dataDir, 'verenu.pid'), 'utf8'), '2147483647');
});

test('model manifest hash and size mismatches are reported', async t => {
  const dataDir = await tempDir(t);
  const dir = join(dataDir, 'models');
  await mkdir(dir);
  await writeFile(join(dir, 'model.bin'), 'model');
  await writeFile(join(dir, 'manifest.json'), JSON.stringify({ files: [{ name: 'model.bin', size: 5, sha256: createHash('sha256').update('model').digest('hex') }] }));
  const check = async () => (await nativeChecks({dataDir, env: {}, platform: 'other'})).find(r => r.name === 'models');
  assert.match((await check()).message, /1 manifest artifacts verified/);
  await writeFile(join(dir, 'model.bin'), 'wrong');
  assert.equal((await check()).status, 'fail');
});

test('extracted model entries and extraction marker are checked', async t => {
  const dataDir = await tempDir(t);
  const root = join(dataDir, 'models', 'stt', 'sense-voice-int8');
  await mkdir(root, {recursive: true});
  await writeFile(join(root, '.verenu-extracted'), 'sense-voice');
  await writeFile(join(root, 'model.int8.onnx'), 'fixture');
  await writeFile(join(root, 'tokens.txt'), 'fixture');
  const check = async () => (await nativeChecks({dataDir, env: {}, platform: 'other'})).find(r => r.name === 'models');
  assert.notEqual((await check()).status, 'fail');
  await rm(join(root, '.verenu-extracted'));
  assert.match((await check()).message, /marker/);
  await rm(join(root, 'tokens.txt'));
  assert.equal((await check()).status, 'fail');
});

test('generated model manifest matches Rust source', async () => {
  assert.equal(await readFile(new URL('../src/model-manifest.json', import.meta.url), 'utf8'), await generate());
});
