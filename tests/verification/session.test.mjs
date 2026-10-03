import assert from 'node:assert/strict';
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

test('agent prompt replacement preserves JavaScript replacement tokens literally', () => {
  const prompt = 'Budget $$5, shell $HOME, regex $1, match $&';
  assert.equal(replacePromptArgument('prefix {prompt} suffix', prompt), `prefix ${prompt} suffix`);
});
