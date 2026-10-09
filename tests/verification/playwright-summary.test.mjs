import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { playwrightSummaryChecks, readPlaywrightReport, summarizePlaywrightReport } from '../../scripts/verification/playwright-summary.mjs';
import { summarizePlaywrightFailures } from '../../scripts/verification/playwright-report.mjs';
import { safeFailureAttempts } from '../../scripts/verification/playwright-diagnostics.mjs';

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

  assert.deepEqual(summary.tests.map(({ diagnostics, ...metadata }) => metadata), [{
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

test('failed attempts retain safe assertion summaries, source locations and private trace pointers after retries', () => {
  const privateValues = ['PRIVATE_RECORD', 'CLIPBOARD_VALUE', 'ENV_SECRET', 'sk-proj-abcdefgh1234', 'github_pat_abcdefgh1234', 'person@example.test'];
  const secret = privateValues.join(' ');
  const failed = spec({ status: 'flaky', results: [{
    status: 'failed',
    errors: [{ message: `\u001b[31mError: expect('${secret}').toHaveText(expected) failed\u001b[0m\nExpected: ${secret}\nReceived: https://localhost/?token=PRIVATE_TOKEN`,
      location: { file: '/private/session/tests/browser/settings.spec.mjs', line: 29, column: 7 },
      stack: secret, snippet: secret }],
    attachments: [
      { name: 'screenshot', contentType: 'image/png', body: secret, path: secret },
      { name: 'trace', contentType: 'application/zip', path: `/private/${secret}/trace.zip`, body: secret },
    ],
  }, { status: 'passed' }] });
  const report = { errors: [{ message: secret }], suites: [{ suites: [{ specs: [failed] }] }] };
  const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
  const expected = [{
    resultIndex: 0, status: 'failed',
    summaries: ['Assertion failed: toHaveText. Values and call log withheld.'],
    location: { file: 'tests/browser/settings.spec.mjs', line: 29, column: 7 },
    artifacts: [{ kind: 'trace', report: 'playwright.json',
      pointer: '/suites/0/suites/0/specs/0/tests/0/results/0/attachments/1', visibility: 'private' }],
  }];
  assert.equal(summary.status, 'failed');
  assert.equal(summary.tests[0].retryCount, 1);
  assert.equal(summary.tests[0].assertionLine, 29);
  assert.deepEqual(summary.tests[0].diagnostics, expected);
  assert.deepEqual(playwrightSummaryChecks(summary)[1].diagnostics, expected);
  const failures = summarizePlaywrightFailures(report);
  assert.deepEqual(failures[0].diagnostics, expected);
  const pointer = expected[0].artifacts[0].pointer;
  assert.equal(pointer.split('/').slice(1).reduce((value, key) => value[key], report).name, 'trace');
  const shared = JSON.stringify({ summary, checks: playwrightSummaryChecks(summary), failures });
  for (const value of [...privateValues, 'PRIVATE_TOKEN', '/private/', 'image/png']) assert.equal(shared.includes(value), false);
});

test('unknown errors are withheld, invalid locations and attachments are omitted, and skips remain incomplete', () => {
  const results = [{ status: 'failed', errors: [null,
    { message: 'Error: expect(private).toLeakSecret(PRIVATE_VALUE)', location: { file: '/private/other/settings.spec.mjs', line: 9 } },
    { message: 'ENV_PASSWORD=PRIVATE_ENV\nclipboard: PRIVATE_CLIPBOARD', location: { file: 'tests/browser/../browser/settings.spec.mjs', line: 10 } },
  ], attachments: [null, { name: 'PRIVATE_NAME', contentType: 'application/zip', path: 'PRIVATE_PATH' }] }];
  const summary = summarizePlaywrightReport({ suites: [{ specs: [spec({ status: 'unexpected', results }),
    spec({ status: 'skipped', results: [] }), spec({ results: [] })] }] }, { projectRoot: '/repo' });
  assert.equal(summary.status, 'failed');
  assert.deepEqual(summary.counts, { total: 3, passed: 0, failed: 2, skipped: 1, retries: 0 });
  assert.deepEqual(summary.tests[0].diagnostics, [{ resultIndex: 0, status: 'failed',
    summaries: ['Failure details withheld or unavailable.'], artifacts: [] }]);
  assert.equal(Object.hasOwn(summary.tests[1], 'diagnostics'), false);
  assert.equal(Object.hasOwn(summary.tests[2], 'diagnostics'), false);
  assert.equal(JSON.stringify(summary).includes('PRIVATE_'), false);
  const skipped = summarizePlaywrightReport({ suites: [{ specs: [spec({ status: 'skipped', results: [] })] }] });
  assert.equal(skipped.status, 'incomplete');
  assert.equal(playwrightSummaryChecks(skipped)[1].status, 'incomplete');
});

test('timeout and interrupted attempts retain fixed summaries without copying single errors', () => {
  for (const [status, message] of [['timedOut', 'Operation timed out. Error text withheld.'], ['interrupted', 'Execution interrupted. Error text withheld.']]) {
    const summary = summarizePlaywrightReport({ suites: [{ specs: [spec({ status: 'unexpected', results: [{ status, error: { message: 'PRIVATE_ERROR' } }] })] }] });
    assert.equal(summary.status, 'failed');
    assert.deepEqual(summary.tests[0].diagnostics[0].summaries, [message]);
    assert.equal(JSON.stringify(summary).includes('PRIVATE_ERROR'), false);
  }
});

test('unsafe spec paths never reach summary, checks, or failure diagnostic locations', () => {
  for (const directory of [
    'https://private.example.test/?token=SYNTHETIC_TOKEN',
    'sk-proj-SYNTHETIC_123456789', 'sk_SYNTHETIC_123456789',
    'ghp_SYNTHETIC_123456789', 'github_pat_SYNTHETIC_123456789',
    'person@example.test', '\u001b[31mSYNTHETIC_CONTROL\u202e',
    'SYNTHETIC_QUERY?token=secret', 'SYNTHETIC_FRAGMENT#secret',
  ]) {
    const file = `tests/browser/${directory}/settings.spec.mjs`;
    const candidate = file.replace('https://', 'https:/');
    const report = { suites: [{ specs: [spec({ file, status: 'unexpected', results: [{
      status: 'failed', errors: [{ message: 'Error: expect(locator).toBeVisible() failed',
        location: { file: candidate, line: 9 } }],
    }] })] }] };
    const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
    const failures = summarizePlaywrightFailures(report);
    assert.equal(summary.status, 'failed');
    assert.equal(summary.tests[0].file, null);
    assert.equal(Object.hasOwn(summary.tests[0].diagnostics[0], 'location'), false);
    assert.equal(failures[0].file, 'unknown');
    const shared = JSON.stringify({ summary, checks: playwrightSummaryChecks(summary), failures });
    for (const value of ['SYNTHETIC_', 'private.example.test', 'person@example.test', '\u001b', '\u202e']) {
      assert.equal(shared.includes(value), false);
    }
    assert.deepEqual(summary.tests[0].diagnostics[0].summaries,
      ['Assertion failed: toBeVisible. Values and call log withheld.']);
    assert.equal(Object.hasOwn(safeFailureAttempts(report.suites[0].specs[0].tests[0], candidate, '/suites/0/specs/0/tests/0')[0], 'location'), false);
  }
});

test('safe nested and absolute source paths retain normalized locations', () => {
  const file = 'tests/browser/nested/settings.spec.mjs';
  const summary = summarizePlaywrightReport({ suites: [{ specs: [spec({
    file: `/repo/${file}`, status: 'unexpected', results: [{ status: 'failed', errors: [{
      message: 'Error: expect(locator).toBeVisible() failed',
      location: { file: 'C:\\runner\\tests\\browser\\nested\\settings.spec.mjs', line: 9, column: 3 },
    }] }],
  })] }] }, { projectRoot: '/repo' });
  assert.equal(summary.tests[0].file, file);
  assert.deepEqual(summary.tests[0].diagnostics[0].location, { file, line: 9, column: 3 });
});

test('ordinary sk suffixes retain source and assertion diagnostics', () => {
  for (const file of [
    'tests/browser/task-flow.spec.mjs', 'tests/browser/mask-entry.spec.mjs',
    'tests/browser/risk-review.spec.mjs', 'tests/browser/task-workflow-long/settings.spec.mjs',
    'tests/browser/sk-ui.spec.mjs',
  ]) {
    const report = { suites: [{ specs: [spec({ file, status: 'unexpected', results: [{
      status: 'failed', errors: [{ message: 'Error: expect(locator).toBeVisible() failed',
        location: { file: `/repo/${file}`, line: 9, column: 3 } }],
    }] })] }] };
    const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
    assert.equal(summary.tests[0].file, file);
    assert.deepEqual(summary.tests[0].diagnostics[0].location, { file, line: 9, column: 3 });
    assert.deepEqual(summarizePlaywrightFailures(report)[0].diagnostics[0].location,
      { file, line: 9, column: 3 });
  }
});

test('credential-shaped spec filenames cannot expose source locations', () => {
  for (const name of ['sk-proj-SYNTHETIC_123456789', 'sk_SYNTHETIC_123456789', 'ghp_SYNTHETIC_123456789', 'github_pat_SYNTHETIC_123456789']) {
    const file = `tests/browser/nested/${name}.spec.mjs`;
    const report = { suites: [{ specs: [spec({ file, status: 'unexpected', results: [{
      status: 'failed', error: { message: 'PRIVATE_ERROR', location: { file, line: 9 } },
    }] })] }] };
    const summary = summarizePlaywrightReport(report, { projectRoot: '/repo' });
    assert.equal(summary.tests[0].file, null);
    assert.equal(Object.hasOwn(summary.tests[0].diagnostics[0], 'location'), false);
    assert.equal(JSON.stringify({ summary, failures: summarizePlaywrightFailures(report) }).includes('SYNTHETIC_'), false);
  }
});
