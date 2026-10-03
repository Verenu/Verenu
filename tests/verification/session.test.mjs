import assert from 'node:assert/strict';
import test from 'node:test';
import { createNativeSession } from '../../scripts/verification/native-session.mjs';
import { startOwnedSession } from '../../scripts/verification/session.mjs';

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
