import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import http from 'node:http';
import { spawnSync } from 'node:child_process';
import { connect, detectedHarnesses, installSkill, invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';
import { muteForRoute } from '../../Agent-Skills/verenu-control/scripts/audio-policy.mjs';

test('skill installation requires consent and an explicit selection of detected harnesses', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-agent-install-'));
  try {
    await fs.mkdir(path.join(home, '.codex'));
    await fs.mkdir(path.join(home, '.claude'));
    assert.deepEqual((await detectedHarnesses(home)).map((h) => h.id), ['codex', 'claude']);
    await assert.rejects(installSkill(['codex'], { home }), /consent/);
    await assert.rejects(installSkill([], { home, permanent: true }), /Choose/);
    await assert.rejects(installSkill(['grok'], { home, permanent: true }), /not detected/);
    const [destination] = await installSkill(['codex'], { home, permanent: true });
    assert.match(await fs.readFile(path.join(destination, 'SKILL.md'), 'utf8'), /name: verenu-control/);
    assert.match(await fs.readFile(path.join(destination, 'scripts/verenu-agent.mjs'), 'utf8'), /export async function invoke/);
    await assert.rejects(fs.stat(path.join(home, '.claude/skills/verenu-control')), /ENOENT/);
    await assert.rejects(installSkill(['codex'], { home, permanent: true }), /already exists/);
    await installSkill(['codex'], { home, permanent: true, replace: true });
    const installed = await import(pathToFileURL(path.join(destination, 'scripts/verenu-agent.mjs')).href);
    await installed.installSkill(['codex', 'claude'], { home, permanent: true, replace: true });
    assert.equal(await fs.readFile(path.join(home, '.claude/skills/verenu-control/SKILL.md'), 'utf8'), await fs.readFile(path.join(destination, 'SKILL.md'), 'utf8'));
  } finally { await fs.rm(home, { recursive: true, force: true }); }
});

test('CLI refuses malformed or absent local grants before making a request', async () => {
  await assert.rejects(invoke('describe', {}, {}), /No agent grant/);
  await assert.rejects(invoke('describe', {}, { VERENU_AGENT_TOKEN: 'a'.repeat(64), VERENU_AGENT_PORT: 'https://example.com' }), /Set VERENU_AGENT_TOKEN/);
});

test('environment grants cannot be confused with approval grants', async () => {
  const env = { VERENU_AGENT_TOKEN: 'a'.repeat(64), VERENU_AGENT_PORT: '17421' };
  await assert.rejects(connect(env), /Unset the environment grant/);
  for (const command of ['status', 'disconnect']) {
    const result = spawnSync(process.execPath, ['Agent-Skills/verenu-control/scripts/verenu-agent.mjs', command], {
      env: { ...process.env, ...env }, encoding: 'utf8',
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Revoke environment grants/);
  }
});

test('connection creates a private session grant and uses it for later requests', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-agent-connection-'));
  let token;
  const server = http.createServer(async (req, res) => {
    let input = '';
    for await (const chunk of req) input += chunk;
    res.setHeader('content-type', 'application/json');
    if (req.url === '/v1/authorize') {
      token = JSON.parse(input).token;
      res.end(JSON.stringify({ code: 'A1B2C3', expires_in_seconds: 120 }));
    } else if (req.url === '/v1/invoke' && req.headers.authorization === `Bearer ${token}`) {
      res.end(JSON.stringify({ version: 1, result: 'connected' }));
    } else { res.writeHead(401); res.end(JSON.stringify({ error: 'Unauthorized' })); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const grantFile = path.join(home, 'grant.json');
  const env = { VERENU_APP_DATA_DIR_OVERRIDE: home, VERENU_AGENT_GRANT_FILE: grantFile };
  try {
    await fs.writeFile(path.join(home, 'agent-port'), String(server.address().port));
    assert.deepEqual(await connect(env), { code: 'A1B2C3', expires_in_seconds: 120 });
    assert.match(token, /^[a-f0-9]{64}$/);
    if (process.platform !== 'win32') assert.equal((await fs.stat(grantFile)).mode & 0o077, 0);
    assert.equal(await invoke('describe', {}, env), 'connected');
    await assert.rejects(connect(env), /EEXIST/);
    if (process.platform !== 'win32') {
      await fs.chmod(grantFile, 0o644);
      await assert.rejects(invoke('describe', {}, env), /private regular file/);
    }
  } finally {
    await new Promise((resolve) => server.close(resolve));
    await fs.rm(home, { recursive: true, force: true });
  }
});

test('CLI reports HTTP failures and malformed success responses without mistaking them for JSON errors', async () => {
  let status = 413;
  let body = 'request body too large';
  const server = http.createServer((_req, res) => { res.writeHead(status); res.end(body); });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const env = { VERENU_AGENT_TOKEN: 'a'.repeat(64), VERENU_AGENT_PORT: String(server.address().port) };
  try {
    await assert.rejects(invoke('describe', {}, env), /HTTP 413/);
    status = 429; body = JSON.stringify({ error: 'Agent API busy' });
    await assert.rejects(invoke('describe', {}, env), /HTTP 429: Agent API busy/);
    status = 200; body = 'not JSON';
    await assert.rejects(invoke('describe', {}, env), /invalid JSON/);
    body = 'null';
    await assert.rejects(invoke('describe', {}, env), /Unsupported agent API response/);
  } finally { await new Promise((resolve) => server.close(resolve)); }
});

test('audio policy uses explicit portable rules, prioritizes unmute and leaves unknown routes unchanged', () => {
  const policy = { muteOutput: 'speakers', unmuteOutput: 'headphones' };
  assert.equal(muteForRoute({ output: 'Speakers', activePort: 'headphones' }, policy), false);
  assert.equal(muteForRoute({ output: 'USB Speakers', activePort: 'lineout' }, policy), true);
  assert.equal(muteForRoute({ output: 'Unknown', activePort: 'lineout' }, policy), null);
  assert.equal(muteForRoute({ output: 'Speakers', activePort: 'speakers' }), null);
});
