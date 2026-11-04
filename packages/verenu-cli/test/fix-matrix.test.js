import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import os from 'node:os';
import { applyFix, fixIds } from '../src/fixes/index.js';

const dbFixture = async d => {
  const db = join(d, 'verenu.db');
  assert.equal(spawnSync('sqlite3', [db, 'CREATE TABLE fixture (id INTEGER); INSERT INTO fixture VALUES (7);']).status, 0);
  return { path: db, original: await readFile(db), backup: 'verenu.db', db: true };
};
const fixtures = {
  'clear-caches': async d => { await mkdir(join(d, 'icon-cache')); const path = join(d, 'icon-cache', 'fixture'); await writeFile(path, 'cache'); return {path, original: Buffer.from('cache'), backup: 'icon-cache/fixture', removed: true}; },
  'reset-settings': async d => { const path = join(d, 'settings.json'); await writeFile(path, '{bad'); return {path, original: Buffer.from('{bad'), backup: 'settings.json', settings: true}; },
  'checkpoint-db': dbFixture,
  'recover-db': dbFixture,
  'clear-stale-pids': async d => { const path = join(d, 'verenu.pid'); await writeFile(path, '2147483647'); return {path, original: Buffer.from('2147483647'), backup: 'verenu.pid', removed: true}; },
  'clear-partial-models': async d => { await mkdir(join(d, 'models')); const path = join(d, 'models', 'model.partial'); await writeFile(path, 'partial'); return {path, original: Buffer.from('partial'), backup: 'models/model.partial', removed: true}; },
};
test('fix coverage guard', () => assert.deepEqual(Object.keys(fixtures).sort(), [...fixIds].sort()));
for (const [id, prepare] of Object.entries(fixtures)) {
  test(`${id}: declined and applied fixtures`, async t => {
    const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-fix-'));
    t.after(() => rm(dataDir, {recursive: true, force: true}));
    const fixture = await prepare(dataDir);
    assert.equal(await applyFix(dataDir, id), null);
    assert.deepEqual(await readFile(fixture.path), fixture.original);
    await assert.rejects(readdir(join(dataDir, 'triage-backups')));
    if (fixture.db) await assert.rejects(applyFix(dataDir, id, {approved: true}), /Typed yes/);
    const backup = await applyFix(dataDir, id, {approved: true, typedYes: true});
    assert.deepEqual(await readFile(join(backup, fixture.backup)), fixture.original);
    if (fixture.removed) await assert.rejects(readFile(fixture.path));
    if (fixture.settings) assert.equal(await readFile(fixture.path, 'utf8'), '{}\n');
    if (fixture.db) assert.equal(spawnSync('sqlite3', [fixture.path, 'SELECT id FROM fixture;'], {encoding: 'utf8'}).stdout.trim(), '7');
  });
}
