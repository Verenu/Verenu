import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import os from 'node:os';
import { checks, checkIds } from '../src/checks/index.js';

const fixtures = {
  settings: async d => writeFile(join(d, 'settings.json'), '{invalid'),
  sqlite: async d => writeFile(join(d, 'verenu.db'), 'broken database'),
  cache: async d => mkdir(join(d, 'icon-cache')),
  terms: async d => writeFile(join(d, 'terms-consent.json'), '{"accepted":false}'),
  network: async () => {},
  launch: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'panicked at startup'); },
  keyring: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'Secret Service unavailable'); },
  'provider-keys': async d => writeFile(join(d, 'settings.json'), '{"transcription_provider":"groq"}'),
  models: async d => { await mkdir(join(d, 'models')); await writeFile(join(d, 'models', 'bad.partial'), 'partial'); },
  process: async d => writeFile(join(d, 'verenu.pid'), '2147483647'),
  logs: async d => { await mkdir(join(d, 'logs')); await writeFile(join(d, 'logs', 'app.log'), 'ERROR fixture'); },
};

test('check coverage guard', () => assert.deepEqual(Object.keys(fixtures).sort(), [...checkIds].sort()));
for (const [id, makeFailure] of Object.entries(fixtures)) {
  test(`${id}: passing and failure fixtures`, async t => {
    const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-check-'));
    t.after(() => rm(dataDir, {recursive: true, force: true}));
    await writeFile(join(dataDir, 'settings.json'), '{"transcription_provider":"local"}');
    await writeFile(join(dataDir, 'terms-consent.json'), '{"accepted":true}');
    assert.equal(spawnSync('sqlite3', [join(dataDir, 'verenu.db'), 'CREATE TABLE fixture (id INTEGER);']).status, 0);
    let failing = false;
    const options = { dataDir, platform: 'linux', env: { VERENU_BINARY: process.execPath, VERENU_CONVEX_URL: 'https://example.com' }, fetcher: async () => { if (failing && id === 'network') throw Error('offline'); return {ok: true}; }, run: (_cmd, args) => ({ status: failing && id === 'provider-keys' && args[0] === 'lookup' ? 1 : 0, stdout: '', stderr: '' }) };
    const pass = await checks(options);
    assert.deepEqual(pass.map(r => r.name).sort(), [...checkIds].sort());
    assert.equal(pass.find(r => r.name === id).status, 'pass');
    await makeFailure(dataDir);
    failing = true;
    const failed = await checks(options);
    assert.notEqual(failed.find(r => r.name === id).status, 'pass');
  });
}
