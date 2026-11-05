import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import os from 'node:os';
import { readAppInfo } from '../src/appInfo.js';

const startup = version => `[2026-11-04T10:00:00Z] INFO  [verenu] verenu ${version} starting — linux x86_64, release build — verbose logging off\n`;

async function dataWithLogs(t, logs) {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-appinfo-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  await mkdir(join(dataDir, 'logs'));
  for (const [name, text] of Object.entries(logs)) await writeFile(join(dataDir, 'logs', name), text);
  return dataDir;
}

test('the app version and build come from the newest session log', async t => {
  const dataDir = await dataWithLogs(t, {
    'session-20260101T000000.000Z-aaaa-0000.log': startup('0.20.0'),
    'session-20261104T100000.000Z-bbbb-0000.log': startup('0.21.0-beta.1') + 'ERROR [verenu::pipeline] private dictated words\n',
  });
  assert.deepEqual(await readAppInfo(dataDir), {
    version: '0.21.0-beta.1',
    os: 'linux',
    arch: 'x86_64',
    build: 'release',
    sessionLog: 'session-20261104T100000.000Z-bbbb-0000.log',
  });
});

test('only the four startup fields are returned, never other log text', async t => {
  const dataDir = await dataWithLogs(t, { 'session-1-0000.log': startup('0.21.0') + 'secret dictated text\n' });
  const info = await readAppInfo(dataDir);
  assert.ok(!JSON.stringify(info).includes('secret dictated text'));
  assert.deepEqual(Object.keys(info).sort(), ['arch', 'build', 'os', 'sessionLog', 'version']);
});

test('a log without a startup line, or no logs at all, gives null', async t => {
  const dataDir = await dataWithLogs(t, { 'session-1-0000.log': 'INFO something else\n', 'notes.txt': startup('9.9.9') });
  assert.equal(await readAppInfo(dataDir), null);
  const empty = await mkdtemp(join(os.tmpdir(), 'verenu-appinfo-'));
  t.after(() => rm(empty, { recursive: true, force: true }));
  assert.equal(await readAppInfo(empty), null);
});
