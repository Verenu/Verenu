import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, writeFile, rm, symlink, chmod, mkdir, copyFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { connect, invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';

const instance = '80e46383-dfaf-4be9-8761-0b60477c9300';

async function fixture(t, { restartDuringAuthorization = false } = {}) {
  const directory = await mkdtemp(path.join(tmpdir(), 'verenu-agent-cli-'));
  const sessionFile = path.join(directory, 'agent-session.json');
  let currentInstance = instance;
  let token;
  let approved = false;
  const server = createServer(async (req, res) => {
    res.setHeader('content-type', 'application/json');
    if (req.url === '/v1/status') return res.end(JSON.stringify({ version: 1, instance: currentInstance }));
    if (req.url === '/v1/authorize') {
      let body = '';
      for await (const chunk of req) body += chunk;
      token = JSON.parse(body).token;
      if (restartDuringAuthorization) currentInstance = '90e46383-dfaf-4be9-8761-0b60477c9300';
      return res.end(JSON.stringify({ code: '123456', expires_in_seconds: 120 }));
    }
    if (req.url === '/v1/invoke' && approved && req.headers.authorization === `Bearer ${token}`) return res.end(JSON.stringify({ version: 1, result: { ok: true } }));
    res.writeHead(401).end(JSON.stringify({ error: 'Unauthorized' }));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  await writeFile(sessionFile, JSON.stringify({ version: 1, port: server.address().port, instance }), { mode: 0o600 });
  t.after(async () => { await new Promise((resolve) => server.close(resolve)); await rm(directory, { recursive: true, force: true }); });
  return { directory, sessionFile, env: { VERENU_APP_DATA_DIR_OVERRIDE: directory, VERENU_AGENT_GRANT_FILE: path.join(directory, 'grant.json') }, approve: () => { approved = true; }, restart: () => { currentInstance = '90e46383-dfaf-4be9-8761-0b60477c9300'; } };
}

test('approval is required and the agent retains its own grant', async (t) => {
  const f = await fixture(t);
  assert.equal((await connect(f.env)).code, '123456');
  await assert.rejects(invoke('describe', {}, f.env), /Unauthorized/);
  f.approve();
  assert.deepEqual(await invoke('describe', {}, f.env), { ok: true });
});

test('cleans up the grant if Verenu restarts during authorization', async (t) => {
  const f = await fixture(t, { restartDuringAuthorization: true });
  await assert.rejects(connect(f.env), /restarted/);
  await assert.rejects(invoke('describe', {}, f.env), /ENOENT/);
});

test('rejects a restarted instance on the same port', async (t) => {
  const f = await fixture(t);
  await connect(f.env);
  f.approve();
  f.restart();
  await assert.rejects(invoke('describe', {}, f.env), /restarted/);
});

test('rejects non-private and linked discovery files on Unix', { skip: process.platform === 'win32' }, async (t) => {
  const f = await fixture(t);
  await chmod(f.sessionFile, 0o644);
  await assert.rejects(connect(f.env), /not private/);
  await rm(f.sessionFile);
  await symlink(path.join(f.directory, 'other'), f.sessionFile);
  await assert.rejects(connect(f.env));
});

test('does not fall back from a partial explicit grant', async (t) => {
  const f = await fixture(t);
  await assert.rejects(invoke('describe', {}, { ...f.env, VERENU_AGENT_TOKEN: 'a'.repeat(64) }), /Set VERENU_AGENT_TOKEN/);
});

test('finds the normal profile and rejects multiple live profiles', { skip: process.platform !== 'linux' }, async (t) => {
  const standard = await fixture(t);
  const other = await fixture(t);
  const home = await mkdtemp(path.join(tmpdir(), 'verenu-agent-home-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const defaultDir = path.join(home, '.config', 'Verenu');
  const xdg = path.join(home, 'other-config');
  const xdgDir = path.join(xdg, 'Verenu');
  await mkdir(defaultDir, { recursive: true });
  await mkdir(xdgDir, { recursive: true });
  await copyFile(standard.sessionFile, path.join(defaultDir, 'agent-session.json'));
  const env = { HOME: home, XDG_CONFIG_HOME: xdg, VERENU_AGENT_GRANT_FILE: path.join(home, 'grant.json') };
  await connect(env);
  await rm(env.VERENU_AGENT_GRANT_FILE);
  await copyFile(other.sessionFile, path.join(xdgDir, 'agent-session.json'));
  await assert.rejects(connect(env), /More than one Verenu profile/);
});
