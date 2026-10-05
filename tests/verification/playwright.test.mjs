import test from 'node:test';
import assert from 'node:assert/strict';
import { failedPlaywrightChecks } from '../../scripts/verification/playwright.mjs';

test('CI reports identify nested failures by project without exposing errors or attachments', () => {
  const report = { suites: [{ title: 'settings.spec.mjs', suites: [{ title: 'Contexts', specs: [{ title: 'saves a Context', tests: [
    { projectName: 'desktop', results: [{ status: 'passed' }] },
    { projectName: 'phone', results: [{ status: 'timedOut', error: { message: 'private session credential' }, attachments: [{ path: 'private trace' }] }] },
  ] }] }] }] };
  assert.deepEqual(failedPlaywrightChecks(report), [{
    name: 'Playwright phone: settings.spec.mjs > Contexts > saves a Context', status: 'failed', reason: 'timedOut',
  }]);
});

test('a successful retry retains the failed attempt in CI diagnostics', () => {
  assert.equal(failedPlaywrightChecks({ suites: [{ specs: [{ title: 'reloads', tests: [{ results: [{ status: 'failed' }, { status: 'passed' }] }] }] }] })[0].status, 'failed');
});

test('successful and skipped results produce no failed checks', () => {
  assert.deepEqual(failedPlaywrightChecks({ suites: [{ specs: [{ tests: [{ results: [{ status: 'passed' }, { status: 'skipped' }] }] }] }] }), []);
  assert.deepEqual(failedPlaywrightChecks({}), []);
});
