import test from 'node:test';
import assert from 'node:assert/strict';
import reporter, { summarizeNodeTests } from '../../scripts/verification/node-reporter.mjs';

test('Node reports preserve failures and skips without private output', async () => {
  const events = [
    { type: 'test:pass', data: { name: 'saved', file: '/private/tests/dev-session/session.test.mjs', details: {} } },
    { type: 'test:pass', data: { name: 'optional', file: '/private/tests/dev-session/session.test.mjs', skip: 'private reason', details: {} } },
    { type: 'test:fail', data: { name: 'recovery', file: '/private/tests/dev-session/recovery.test.mjs', details: { error: 'SECRET_ERROR' } } },
    { type: 'test:pass', data: { name: 'unfinished', file: '/private/tests/dev-session/recovery.test.mjs', todo: true, details: {} } },
  ];
  let output = '';
  for await (const chunk of reporter((async function* () { yield* events; })())) output += chunk;
  const report = JSON.parse(output);
  assert.deepEqual(report.tests.map(row => row.status), ['passed', 'skipped', 'failed', 'skipped']);
  assert.equal(output.includes('private'), false);
  assert.equal(output.includes('SECRET_ERROR'), false);
  assert.equal(summarizeNodeTests(report, ['session.test.mjs', 'recovery.test.mjs']).status, 'failed');
});

test('missing files, unexpected skips and all-skipped suites remain incomplete', () => {
  const row = { name: 'required', file: 'session.test.mjs', status: 'passed' };
  assert.equal(summarizeNodeTests({ tests: [row] }, ['session.test.mjs', 'hotkey-chords.test.mjs']).status, 'incomplete');
  assert.equal(summarizeNodeTests({ tests: [{ ...row, status: 'skipped' }] }, ['session.test.mjs']).status, 'incomplete');
  const live = { ...row, status: 'skipped', name: 'live synthetic corpus reaches providers, Context rules, events, and exact new history' };
  assert.equal(summarizeNodeTests({ tests: [row, live] }, ['session.test.mjs']).status, 'passed');
  assert.equal(summarizeNodeTests({ tests: [row, live] }, ['session.test.mjs'], { live: true }).status, 'incomplete');
});
