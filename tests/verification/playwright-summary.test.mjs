import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { playwrightSummaryChecks, readPlaywrightReport, summarizePlaywrightReport } from '../../scripts/verification/playwright-summary.mjs';

test('expected failures, empty results and successful retries are not clean passes', () => {
  for (const results of [[], [{ status: 'failed' }], [{ status: 'failed' }, { status: 'passed' }]]) {
    assert.equal(summarizePlaywrightReport({ suites: [{ specs: [spec({ results })] }] }).status, 'failed');
  }
});

function spec({ title = 'saves the setting', file = 'tests/browser/settings.spec.mjs', line = 17, status = 'expected', results = [{ status: 'passed' }], projectName = 'desktop' } = {}) {
  return {
    title,
    file,
    line,
    tests: [{ projectName, status, results }],
  };
}

test('Playwright summary walks nested suites and keeps only safe case metadata', () => {
  const report = {
    errors: [{ message: 'PRIVATE_ERROR detail https://example.test/?token=private' }],
    suites: [{
      title: 'settings.spec.mjs',
      file: 'tests/browser/settings.spec.mjs',
      suites: [{ title: 'Settings persistence', specs: [spec()] }],
    }],
  };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });

  assert.equal(summary.availability, 'available');
  assert.equal(summary.status, 'passed');
  assert.deepEqual(summary.tests, [{
    project: 'desktop', file: 'tests/browser/settings.spec.mjs', line: 17,
    title: 'Settings persistence › saves the setting', status: 'passed', retryCount: 0,
  }]);
  assert.equal(JSON.stringify(summary).includes('PRIVATE_ERROR'), false);
  assert.equal(JSON.stringify(summary).includes('private'), false);
  assert.deepEqual(playwrightSummaryChecks(summary).map(({ name, status }) => ({ name, status })), [
    { name: 'Real-session Playwright browser flows', status: 'passed' },
  ]);
});

test('unexpected, interrupted, and flaky outcomes become failed checks without error text', () => {
  const report = {
    suites: [{
      title: 'settings.spec.mjs', file: 'tests/browser/settings.spec.mjs',
      specs: [
        spec({ title: 'unexpected result', status: 'unexpected', results: [{ status: 'failed', error: { message: 'SECRET_ERROR' } }] }),
        spec({ title: 'interrupted result', status: 'interrupted', results: [{ status: 'interrupted' }] }),
        spec({ title: 'retried result', status: 'flaky', results: [{ status: 'failed' }, { status: 'passed', retry: 1 }] }),
        spec({ title: 'intentionally skipped', status: 'skipped', results: [] }),
      ],
    }],
  };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
  const checks = playwrightSummaryChecks(summary);

  assert.equal(summary.status, 'failed');
  assert.deepEqual(summary.counts, { total: 4, passed: 0, failed: 3, skipped: 1, retries: 1 });
  assert.deepEqual(summary.tests.map((row) => row.status), ['failed', 'failed', 'failed', 'skipped']);
  assert.equal(summary.tests[2].retryCount, 1);
  assert.equal(checks.filter((check) => check.status === 'failed').length, 4);
  assert.equal(JSON.stringify(summary).includes('SECRET_ERROR'), false);
  assert.equal(JSON.stringify(checks).includes('SECRET_ERROR'), false);
});

test('static titles redact URLs, credential-shaped strings, and email addresses', () => {
  const report = { suites: [{ specs: [spec({ title: 'calls https://example.test/?key=private with sk-proj-abcdefgh1234 and test@example.com' })] }] };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });

  assert.equal(summary.tests[0].title, 'calls [url] with [redacted] and [redacted]');
});

test('failed dropdown cases expose only allowlisted geometry and assertion lines', () => {
  const report = {
    suites: [{
      file: 'tests/browser/settings-dropdowns.spec.mjs',
      specs: [{
        title: 'menus stay in the viewport',
        line: 34,
        tests: [{
          projectName: 'phone',
          status: 'unexpected',
          results: [{
            status: 'failed',
            errors: [{
              message: 'private assertion value\nMENU_GEOMETRY:{"menuId":"transcription-mode-menu","check":"right-panel","bounds":{"x":8,"y":12,"width":374,"height":240,"privateText":"do-not-share"},"content":{"x":16,"y":24,"width":358,"height":700},"viewport":{"width":390,"height":844},"privateText":"do-not-share"}',
              location: { file: '/runner/work/app/tests/browser/settings-dropdowns.spec.mjs', line: 80, column: 5 },
            }],
          }],
        }],
      }],
    }],
  };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });

  assert.deepEqual(summary.tests, [{
    project: 'phone', file: 'tests/browser/settings-dropdowns.spec.mjs', line: 34,
    title: 'menus stay in the viewport', status: 'failed', retryCount: 0,
    assertionLine: 80,
    menuGeometry: {
      check: 'right-panel', menuId: 'transcription-mode-menu',
      bounds: { x: 8, y: 12, width: 374, height: 240 },
      content: { x: 16, y: 24, width: 358, height: 700 },
      viewport: { width: 390, height: 844 },
    },
  }]);
  assert.equal(JSON.stringify(summary).includes('private'), false);
  assert.equal(JSON.stringify(summary).includes('do-not-share'), false);
});

test('successful test results do not erase a failed browser process', () => {
  const summary = summarizePlaywrightReport({ suites: [{ specs: [spec()] }] }, {
    processStatus: 'failed', exitCode: 1, processReason: 'Process ended with SIGABRT', projectRoot: '/repo',
  });
  const checks = playwrightSummaryChecks(summary);

  assert.equal(summary.status, 'failed');
  assert.deepEqual(summary.process, { status: 'failed', exitCode: 1, reason: 'Playwright process exited with code 1.' });
  assert.equal(summary.reason, 'Playwright process exited with code 1.');
  assert.equal(checks.filter((check) => check.status === 'failed').length, 1);
  assert.equal(JSON.stringify(summary).includes('SIGABRT'), false);
});

test('failed process with missing or invalid JSON gets a generic check and no invented cases', () => {
  for (const availability of ['missing', 'invalid']) {
    const summary = summarizePlaywrightReport(null, {
      availability, processStatus: 'failed', processReason: 'Check timed out', projectRoot: '/repo',
    });
    assert.equal(summary.availability, availability);
    assert.equal(summary.status, 'failed');
    assert.deepEqual(summary.tests, []);
    assert.equal(summary.reason, 'Playwright process timed out.');
    assert.deepEqual(playwrightSummaryChecks(summary), [{
      name: 'Real-session Playwright browser flows', status: 'failed', reason: 'Playwright process timed out.',
    }]);
  }
});

test('missing reports and reports with no test cases fail closed even if the process exits zero', () => {
  const missing = summarizePlaywrightReport(null, { availability: 'missing', processStatus: 'passed' });
  const empty = summarizePlaywrightReport({ suites: [] }, { processStatus: 'passed' });

  assert.equal(missing.status, 'failed');
  assert.equal(missing.reason, 'Structured Playwright report is missing or invalid.');
  assert.equal(empty.status, 'failed');
  assert.equal(empty.reason, 'Playwright report contains no test cases.');
});

test('all-skipped and partially skipped browser suites cannot verify a task', () => {
  for (const specs of [[spec({ status: 'skipped', results: [] })], [spec(), spec({ status: 'skipped', results: [] })]]) {
    const summary = summarizePlaywrightReport({ suites: [{ specs }] });
    assert.equal(summary.status, 'incomplete');
    assert.ok(playwrightSummaryChecks(summary).some(row => row.status === 'incomplete'));
  }
});

test('each required viewport project must appear in the executed report', () => {
  const report = { suites: [{ specs: [spec()] }] };
  const summary = summarizePlaywrightReport(report, { expectedProjects: ['desktop', 'phone'] });
  assert.equal(summary.status, 'incomplete');
  assert.deepEqual(summary.missingProjects, ['phone']);
  report.suites[0].specs.push(spec({ projectName: 'phone' }));
  assert.equal(summarizePlaywrightReport(report, { expectedProjects: ['desktop', 'phone'] }).status, 'passed');
});

test('report reader identifies missing and malformed JSON without returning raw contents', async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-playwright-report-'));
  try {
    const missing = await readPlaywrightReport(path.join(directory, 'missing.json'));
    assert.deepEqual(missing, { availability: 'missing', report: null });

    const malformedPath = path.join(directory, 'malformed.json');
    await fs.writeFile(malformedPath, '{"secret":"PRIVATE_VALUE"');
    const malformed = await readPlaywrightReport(malformedPath);
    assert.deepEqual(malformed, { availability: 'invalid', report: null });
    assert.equal(JSON.stringify(malformed).includes('PRIVATE_VALUE'), false);

    const reportPath = path.join(directory, 'valid.json');
    await fs.writeFile(reportPath, JSON.stringify({ suites: [] }));
    const valid = await readPlaywrightReport(reportPath);
    assert.deepEqual(valid, { availability: 'available', report: { suites: [] } });
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});

test('case paths stay repository-relative and invalid report shapes do not create cases', () => {
  const report = {
    suites: [{
      suites: [{ title: 'outside file', file: '/etc/passwd', specs: [spec({ file: '/etc/passwd', line: -3 })] }],
      specs: [spec({ title: 'escaped file', file: 'tests/browser/../../../../etc/passwd' }), null, { title: 'invalid', tests: 'not-an-array' }],
    }],
  };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });

  assert.equal(summary.tests.length, 2);
  assert.deepEqual(summary.tests.map(({ file, line }) => [file, line]), [[null, 17], [null, null]]);
  assert.equal(summary.tests[0].status, 'passed');
  assert.equal(summary.tests[0].project, 'desktop');

  const relative = summarizePlaywrightReport({ suites: [{ specs: [spec({ file: 'settings.spec.mjs' })] }] }, { projectRoot: '/repo' });
  assert.equal(relative.tests[0].file, 'tests/browser/settings.spec.mjs');
});
