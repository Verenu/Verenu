import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createNativeSession } from '../../scripts/verification/native-session.mjs';
import { replacePromptArgument } from '../../scripts/verification/prompt.mjs';
import { ownedSessionStartupState, startOwnedSession } from '../../scripts/verification/session.mjs';

test('native WebDriver session creation retries transient HTTP errors', async () => {
  let attempts = 0;
  const id = await createNativeSession(4444, {
    deadlineMs: 1_000,
    sleep: async () => {},
    fetchImpl: async () => {
      attempts++;
      return attempts === 1
        ? { ok: false, status: 500, json: async () => ({ value: { message: 'window is still starting' } }) }
        : { ok: true, status: 200, json: async () => ({ value: { sessionId: 'synthetic-session' } }) };
    },
  });

  assert.equal(id, 'synthetic-session');
  assert.equal(attempts, 2);
});

test('owned session requires an artifact directory', async () => {
  await assert.rejects(
    startOwnedSession({ id: 'missing-directory' }),
    { name: 'TypeError', message: 'startOwnedSession requires a directory path' },
  );
});

test('owned session rejects unsafe IDs before constructing a session path', async () => {
  for (const id of ['../escape', 'has spaces', '', 'x'.repeat(81)]) {
    await assert.rejects(
      startOwnedSession({ id, directory: '/tmp/unused-session' }),
      { name: 'TypeError', message: 'startOwnedSession requires a valid session ID' },
    );
  }
});

test('owned session startup fails fast for a stopped manifest owned by its launcher', () => {
  assert.equal(ownedSessionStartupState({ launcherPid: 42, status: 'starting' }, 42), 'starting');
  assert.equal(ownedSessionStartupState({ launcherPid: 42, status: 'ready' }, 42), 'ready');
  assert.equal(ownedSessionStartupState({ launcherPid: 42, status: 'stopped' }, 42), 'stopped');
  assert.equal(ownedSessionStartupState({ launcherPid: 24, status: 'stopped' }, 42), null);
});

test('dev-session rejects invalid synthetic seed arguments before creating a session', async () => {
  const script = fileURLToPath(new URL('../../scripts/dev-session.mjs', import.meta.url));
  const stateRoot = path.join(os.homedir(), '.local', 'state', 'verenu', 'dev-sessions');
  for (const args of [['--synthetic-seed', '--seed-dir'], ['--synthetic-seed', '--seed-dir', '/tmp/private-seed']]) {
    const id = `invalid-seed-${randomUUID()}`;
    const result = spawnSync(process.execPath, [script, '--id', id, ...args], { encoding: 'utf8', timeout: 5_000 });
    assert.equal(result.status, 1);
    assert.match(result.stderr, args.at(-1) === '--seed-dir' ? /--seed-dir requires a value/ : /--synthetic-seed cannot copy installed data/);
    await assert.rejects(fs.access(path.join(stateRoot, id)), { code: 'ENOENT' });
  }
});

test('agent prompt replacement preserves JavaScript replacement tokens literally', () => {
  const prompt = 'Budget $$5, shell $HOME, regex $1, match $&';
  assert.equal(replacePromptArgument('prefix {prompt} suffix', prompt), `prefix ${prompt} suffix`);
});
