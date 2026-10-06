import test from 'node:test';
import assert from 'node:assert/strict';
import reporter, { summarizeNodeFailure, summarizeNodeTests } from '../../scripts/verification/node-reporter.mjs';

test('Node reports preserve failures and skips without private output', async () => {
  const events = [
    { type: 'test:pass', data: { name: 'saved', file: '/private/tests/dev-session/session.test.mjs', details: {} } },
    { type: 'test:pass', data: { name: 'optional', file: '/private/tests/dev-session/session.test.mjs', skip: 'private reason', details: {} } },
    { type: 'test:fail', data: { name: 'recovery', file: '/private/tests/dev-session/recovery.test.mjs', details: { error: {
      code: 'ERR_TEST_FAILURE',
      cause: {
        name: 'AssertionError', code: 'ERR_ASSERTION', actual: 'PRIVATE_DICTATED_TEXT', expected: 'OTHER_PRIVATE_TEXT',
        stack: 'AssertionError [ERR_ASSERTION]: private\n    at /private/tests/dev-session/recovery.test.mjs:42:9',
      },
    } } } },
    { type: 'test:pass', data: { name: 'unfinished', file: '/private/tests/dev-session/recovery.test.mjs', todo: true, details: {} } },
  ];
  let output = '';
  for await (const chunk of reporter((async function* () { yield* events; })())) output += chunk;
  const report = JSON.parse(output);
  assert.deepEqual(report.tests.map(row => row.status), ['passed', 'skipped', 'failed', 'skipped']);
  assert.equal(output.includes('private'), false);
  assert.equal(output.includes('PRIVATE_DICTATED_TEXT'), false);
  assert.deepEqual(report.tests[2].failure, {
    type: 'AssertionError', code: 'ERR_ASSERTION', location: { file: 'recovery.test.mjs', line: 42 },
  });
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

test('failed session reports include named failures from expected files only', () => {
  const summary = summarizeNodeTests({ tests: [
    { name: 'Synthetic regression case', file: 'pipeline.test.mjs', status: 'failed', failure: { type: 'AssertionError', location: { file: 'pipeline.test.mjs', line: 52 } } },
    { name: 'live corpus', file: 'session.test.mjs', status: 'skipped' },
    { name: 'outside test', file: 'private.test.mjs', status: 'failed' },
  ] }, ['pipeline.test.mjs', 'session.test.mjs']);
  const failure = summarizeNodeFailure(summary, ['pipeline.test.mjs', 'session.test.mjs'], {
    processStatus: 'failed', exitCode: 1,
  });
  assert.deepEqual(failure, {
    status: 'failed',
    process: { status: 'failed', exitCode: 1 },
    counts: { total: 2, passed: 0, failed: 1, skipped: 1 },
    failedFiles: ['pipeline.test.mjs'],
    failedTests: [{ name: 'Synthetic regression case', file: 'pipeline.test.mjs', failure: { type: 'AssertionError', location: { file: 'pipeline.test.mjs', line: 52 } } }],
    skippedFiles: ['session.test.mjs'],
    missingFiles: [],
  });
  assert.equal(JSON.stringify(failure).includes('SYNTHETIC_PRIVATE_DETAIL'), false);
  assert.equal(JSON.stringify(failure).includes('private.test.mjs'), false);
});
