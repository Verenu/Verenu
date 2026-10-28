import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, writeFile, rename, rm, symlink, chmod, mkdir, copyFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { connect, invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';

const token = 'a'.repeat(64);
const instance = '80e46383-dfaf-4be9-8761-0b60477c9300';

async function fixture(t, onPair) {
  const directory = await mkdtemp(path.join(tmpdir(), 'verenu-agent-cli-'));
  const sessionFile = path.join(directory, 'agent-session.json');
  let port;
  async function publish(status, grant, instanceId = instance) {
    const next = path.join(directory, 'next.json');
    await writeFile(next, JSON.stringify({ version: 1, port, instance: instanceId, status, ...(grant ? { token: grant } : {}) }), { mode: 0o600 });
    await rename(next, sessionFile);
  }
  const server = createServer(async (req, res) => {
    if (req.url === '/v1/status') {
      res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ version: 1, instance }));
    } else if (req.url === '/v1/pair') {
      await onPair(publish);
      res.writeHead(202).end();
    } else if (req.url === '/v1/invoke' && req.headers.authorization === `Bearer ${token}`) {
      res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ version: 1, result: { ok: true } }));
    } else res.writeHead(401, { 'content-type': 'application/json' }).end('{}');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  port = server.address().port;
  await publish('idle');
  t.after(async () => { await new Promise((resolve) => server.close(resolve)); await rm(directory, { recursive: true, force: true }); });
  return { env: { VERENU_APP_DATA_DIR_OVERRIDE: directory }, publish, sessionFile, directory };
}

test('connect waits for approval and later commands reuse the approved grant', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, async (publish) => { await publish('pending'); setTimeout(() => void publish('approved', token), 15); });
  await assert.rejects(invoke('describe', {}, f.env), /not approved/);
  await connect(f.env, { timeoutMs: 500, pollMs: 5 });
  assert.deepEqual(await invoke('describe', {}, f.env), { ok: true });
  await connect(f.env, { timeoutMs: 500, pollMs: 5 });
});

test('declined approval never grants access', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, async (publish) => { await publish('denied'); });
  await assert.rejects(connect(f.env, { timeoutMs: 500, pollMs: 5 }), /declined/);
  await assert.rejects(invoke('describe', {}, f.env), /not approved/);
});

test('does not accept a restarted instance using the same port', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, async (publish) => { await publish('approved', token, '90e46383-dfaf-4be9-8761-0b60477c9300'); });
  await assert.rejects(connect(f.env, { timeoutMs: 500, pollMs: 5 }), /restarted during approval/);
});

test('rejects non-private or linked session files on Unix', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, async () => {});
  await chmod(f.sessionFile, 0o644);
  await assert.rejects(invoke('describe', {}, f.env), /not private/);
  await rm(f.sessionFile);
  await symlink(path.join(f.directory, 'other'), f.sessionFile);
  await assert.rejects(invoke('describe', {}, f.env));
});

test('does not silently fall back from a partial explicit grant', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t, async () => {});
  await f.publish('approved', token);
  await assert.rejects(invoke('describe', {}, { ...f.env, VERENU_AGENT_TOKEN: token }), /Invalid explicit agent grant/);
});

test('finds the normal live profile when XDG points elsewhere and rejects ambiguity', { skip: process.platform !== 'linux' }, async (t) => {
  const standard = await fixture(t, async () => {});
  const other = await fixture(t, async () => {});
  await standard.publish('approved', token);
  await other.publish('approved', token);
  const home = await mkdtemp(path.join(tmpdir(), 'verenu-agent-home-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const defaultDir = path.join(home, '.config', 'Verenu');
  const xdg = path.join(home, 'other-config');
  const xdgDir = path.join(xdg, 'Verenu');
  await mkdir(defaultDir, { recursive: true });
  await mkdir(xdgDir, { recursive: true });
  await copyFile(standard.sessionFile, path.join(defaultDir, 'agent-session.json'));
  const env = { HOME: home, XDG_CONFIG_HOME: xdg };
  assert.deepEqual(await invoke('describe', {}, env), { ok: true });
  await copyFile(other.sessionFile, path.join(xdgDir, 'agent-session.json'));
  await assert.rejects(invoke('describe', {}, env), /More than one Verenu profile/);
});
