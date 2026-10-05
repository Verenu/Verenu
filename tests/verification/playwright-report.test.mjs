import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizePlaywrightFailures } from '../../scripts/verification/playwright-report.mjs';

test('Playwright summaries retain failed desktop and phone case names without assertion data', () => {
  const report = {
    suites: [{
      file: 'tests/browser/api-keys.spec.mjs',
      specs: [{
        title: 'API key actions never expose the inactive label',
        line: 3,
        tests: [
          { projectName: 'desktop', results: [{ status: 'failed', errors: [{ message: 'private assertion value' }] }] },
          { projectName: 'phone', results: [{ status: 'timedOut', errors: [{ message: 'private assertion value' }] }] },
          { projectName: 'desktop', results: [{ status: 'passed' }] },
        ],
      }],
      suites: [{
        file: 'tests/browser/models.spec.mjs',
        specs: [{
          title: 'Model state survives reload',
          line: 19,
          tests: [{ projectName: 'desktop', results: [{ status: 'passed' }, { status: 'failed' }] }],
        }],
      }],
    }],
  };

  assert.deepEqual(summarizePlaywrightFailures(report), [
    { project: 'desktop', file: 'api-keys.spec.mjs', line: 3, title: 'API key actions never expose the inactive label' },
    { project: 'desktop', file: 'models.spec.mjs', line: 19, title: 'Model state survives reload' },
    { project: 'phone', file: 'api-keys.spec.mjs', line: 3, title: 'API key actions never expose the inactive label' },
  ]);
});

test('Playwright summaries omit passing, skipped, and missing results', () => {
  assert.deepEqual(summarizePlaywrightFailures({
    suites: [{ specs: [{ title: 'Passing test', tests: [{ results: [{ status: 'passed' }] }] }] }],
  }), []);
  assert.deepEqual(summarizePlaywrightFailures(null), []);
});
