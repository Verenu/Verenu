import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, open } from 'node:fs/promises';
import os from 'node:os';
import { join } from 'node:path';
import { checks } from '../src/checks/index.js';
import { fix, stalePidFiles } from '../src/fixes/index.js';
import { bundle } from '../src/bundle.js';
import { report } from '../src/report.js';
import { nativeChecks } from '../src/checks/native.js';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { generate } from '../scripts/generate-model-manifest.js';

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

test('extracted model entries and extraction marker are checked', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
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

test('SQLite checkpoint requires typed confirmation and backs up the database', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const db = join(dataDir, 'verenu.db');
  assert.equal(spawnSync('sqlite3', [db, 'CREATE TABLE example (id INTEGER); INSERT INTO example VALUES (1);']).status, 0);
  const original = await readFile(db);
  await assert.rejects(fix(dataDir, 'checkpoint-db'), /Typed yes/);
  assert.deepEqual(await readFile(db), original);
  const backup = await fix(dataDir, 'checkpoint-db', { typedYes: true });
  assert.deepEqual(await readFile(join(backup, 'verenu.db')), original);
  assert.equal(spawnSync('sqlite3', [db, 'PRAGMA integrity_check;'], {encoding: 'utf8'}).stdout.trim(), 'ok');
});

test('checkpoint backs up and truncates a real unheld WAL', {timeout: 15000}, async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  const db = join(dataDir, 'verenu.db');
  const child = spawn('sqlite3', [db], {stdio: ['pipe', 'pipe', 'pipe']});
  t.after(() => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); });
  await new Promise((resolve, reject) => {
    let output = '';
    child.once('error', reject);
    child.stdout.on('data', data => { output += data; if (output.includes('fixture-ready')) resolve(); });
    child.once('exit', code => reject(new Error(`SQLite fixture exited early: ${code}`)));
    child.stdin.write("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE fixture(id INTEGER); INSERT INTO fixture VALUES (7); SELECT 'fixture-ready';\n");
  });
  const ended = new Promise(resolve => child.once('exit', resolve));
  child.kill('SIGKILL');
  await ended;
  const wal = `${db}-wal`;
  const originalWal = await readFile(wal);
  assert.ok(originalWal.length > 0);
  const backup = await fix(dataDir, 'checkpoint-db', {typedYes: true});
  assert.deepEqual(await readFile(join(backup, 'verenu.db-wal')), originalWal);
  const remaining = await readFile(wal).catch(error => { if (error.code === 'ENOENT') return Buffer.alloc(0); throw error; });
  assert.equal(remaining.length, 0);
  assert.equal(spawnSync('sqlite3', [db, 'SELECT id FROM fixture;'], {encoding: 'utf8'}).stdout.trim(), '7');
});

test('SQLite recovery requires typed confirmation and preserves the original backup', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const db = join(dataDir, 'verenu.db');
  assert.equal(spawnSync('sqlite3', [db, 'CREATE TABLE example (id INTEGER); INSERT INTO example VALUES (1);']).status, 0);
  const original = await readFile(db);
  await assert.rejects(fix(dataDir, 'recover-db'), /Typed yes/);
  assert.deepEqual(await readFile(db), original);
  const backup = await fix(dataDir, 'recover-db', { typedYes: true });
  assert.deepEqual(await readFile(join(backup, 'verenu.db')), original);
  assert.equal(spawnSync('sqlite3', [db, 'SELECT id FROM example;'], { encoding: 'utf8' }).stdout.trim(), '1');
});

test('failed recovery keeps a corrupt original and its backup', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const db = join(dataDir, 'verenu.db');
  await writeFile(db, 'synthetic corrupt database');
  await assert.rejects(fix(dataDir, 'recover-db', {typedYes: true}));
  assert.equal(await readFile(db, 'utf8'), 'synthetic corrupt database');
  const backups = await (await import('node:fs/promises')).readdir(join(dataDir, 'triage-backups'));
  assert.equal(await readFile(join(dataDir, 'triage-backups', backups[0], 'verenu.db'), 'utf8'), 'synthetic corrupt database');
});

test('database actions refuse an active holder without changing data', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  const db = join(dataDir, 'verenu.db');
  assert.equal(spawnSync('sqlite3', [db, 'CREATE TABLE example (id INTEGER);']).status, 0);
  const original = await readFile(db);
  const handle = await open(db, 'r');
  try {
    for (const id of ['checkpoint-db', 'recover-db']) await assert.rejects(fix(dataDir, id, {typedYes: true}), /active holders/);
    assert.deepEqual(await readFile(db), original);
  } finally { await handle.close(); }
});

test('recovery refuses sidecars without changing the database', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  const db = join(dataDir, 'verenu.db');
  assert.equal(spawnSync('sqlite3', [db, 'CREATE TABLE example (id INTEGER);']).status, 0);
  const original = await readFile(db);
  await writeFile(`${db}-wal`, 'fixture');
  await assert.rejects(fix(dataDir, 'recover-db', {typedYes: true}), /sidecars/);
  assert.deepEqual(await readFile(db), original);
  assert.equal(await readFile(`${db}-wal`, 'utf8'), 'fixture');
});

test('model cleanup backs up a known artifact with a checksum mismatch', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  const models = join(dataDir, 'models');
  await mkdir(models);
  await writeFile(join(models, 'ggml-small.bin'), 'invalid fixture');
  const backup = await fix(dataDir, 'clear-partial-models');
  assert.equal(await readFile(join(backup, 'models', 'ggml-small.bin'), 'utf8'), 'invalid fixture');
  await assert.rejects(readFile(join(models, 'ggml-small.bin')));
});

test('settings reset restores the newest valid backed-up settings', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  const previous = join(dataDir, 'triage-backups', '100-old');
  await mkdir(previous, { recursive: true });
  await writeFile(join(previous, 'settings.json'), '{"volume":42}');
  await writeFile(join(dataDir, 'settings.json'), '{bad');
  await fix(dataDir, 'reset-settings');
  assert.equal(await readFile(join(dataDir, 'settings.json'), 'utf8'), '{"volume":42}');
});

test('stale PID cleanup preserves the file in backup', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  await writeFile(join(dataDir, 'verenu.pid'), '2147483647');
  assert.equal((await stalePidFiles(dataDir)).length, 1);
  const backup = await fix(dataDir, 'clear-stale-pids');
  assert.equal(await readFile(join(backup, 'verenu.pid'), 'utf8'), '2147483647');
  await assert.rejects(readFile(join(dataDir, 'verenu.pid')));
});

test('partial model cleanup moves only incomplete artifacts to backup', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  await mkdir(join(dataDir, 'models'));
  await writeFile(join(dataDir, 'models', 'file.partial'), 'partial');
  await writeFile(join(dataDir, 'models', 'good'), 'complete');
  const backup = await fix(dataDir, 'clear-partial-models');
  assert.equal(await readFile(join(backup, 'models', 'file.partial'), 'utf8'), 'partial');
  assert.equal(await readFile(join(dataDir, 'models', 'good'), 'utf8'), 'complete');
  await assert.rejects(readFile(join(dataDir, 'models', 'file.partial')));
});

test('noninteractive invocation declines fixes by default', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  await writeFile(join(dataDir, 'settings.json'), '{invalid');
  const cli = fileURLToPath(new URL('../bin/verenu.js', import.meta.url));
  const invoked = spawnSync(process.execPath, [cli, 'triage', '--data-dir', dataDir, '--json'], {encoding: 'utf8', timeout: 15000});
  assert.equal(invoked.status, 0);
  assert.equal(await readFile(join(dataDir, 'settings.json'), 'utf8'), '{invalid');
  assert.deepEqual(JSON.parse(invoked.stdout).actions, []);
});

test('--yes does not approve transcript recovery without typed confirmation', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, {recursive: true, force: true}));
  await writeFile(join(dataDir, 'verenu.db'), 'broken fixture');
  const cli = fileURLToPath(new URL('../bin/verenu.js', import.meta.url));
  const invoked = spawnSync(process.execPath, [cli, 'triage', '--data-dir', dataDir, '--json', '--yes'], {encoding: 'utf8', timeout: 15000});
  assert.equal(invoked.status, 0);
  assert.equal(await readFile(join(dataDir, 'verenu.db'), 'utf8'), 'broken fixture');
  assert.equal(JSON.parse(invoked.stdout).actions.some(action => action.fix === 'recover-db'), false);
});
