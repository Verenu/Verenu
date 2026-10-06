import { safeText } from './playwright-summary.mjs';

function nodeFailure(error, file) {
  let cause = error;
  for (let depth = 0; depth < 4 && cause?.cause; depth++) {
    if (cause.code === 'ERR_ASSERTION' || cause.name === 'AssertionError') break;
    cause = cause.cause;
  }
  const code = typeof cause?.code === 'string' && /^[A-Z0-9_]{1,64}$/.test(cause.code) ? cause.code : undefined;
  const name = ['AssertionError', 'Error', 'TypeError', 'RangeError', 'SyntaxError'].includes(cause?.name)
    ? cause.name
    : 'Error';
  const escapedFile = file.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = typeof cause?.stack === 'string'
    ? cause.stack.match(new RegExp(`(?:^|[/\\\\])tests[/\\\\]dev-session[/\\\\](${escapedFile}):(\\d+)(?::\\d+)?`, 'm'))
    : null;
  return {
    type: name,
    ...(code ? { code } : {}),
    ...(match ? { location: { file, line: Number(match[2]) } } : {}),
  };
}

// Node's reporter stream keeps skipped cases visible without publishing errors,
// invocation arguments, console output, or private session paths.
export default async function* reporter(events) {
  const tests = [];
  for await (const event of events) {
    if (!['test:pass', 'test:fail'].includes(event.type) || event.data.details?.type === 'suite') continue;
    const file = event.data.file?.replaceAll('\\', '/').split('/tests/dev-session/')[1] || null;
    const row = {
      name: safeText(event.data.name),
      file,
      status: event.data.skip || event.data.todo ? 'skipped' : event.type === 'test:fail' ? 'failed' : 'passed',
    };
    if (row.status === 'failed' && file) row.failure = nodeFailure(event.data.details?.error, file);
    tests.push(row);
  }
  yield JSON.stringify({ tests }, null, 2);
}

export function summarizeNodeTests(report, expectedFiles, { live = false } = {}) {
  const tests = Array.isArray(report?.tests) ? report.tests : [];
  const missingFiles = expectedFiles.filter(file => !tests.some(row => row.file === file));
  const failed = tests.some(row => row.status === 'failed');
  const incomplete = !tests.length || missingFiles.length > 0 || tests.some(row => {
    if (row.status === 'passed') return false;
    return !(row.status === 'skipped' && !live && row.name === 'live synthetic corpus reaches providers, Context rules, events, and exact new history');
  });
  return { status: failed ? 'failed' : incomplete ? 'incomplete' : 'passed', tests, missingFiles };
}

export function summarizeNodeFailure(summary, expectedFiles, { processStatus, exitCode, timedOut = false } = {}) {
  const tests = Array.isArray(summary?.tests) ? summary.tests : [];
  const allowedFiles = new Set(expectedFiles);
  const scopedTests = tests.filter(row => allowedFiles.has(row.file));
  const filesFor = (status) => [...new Set(scopedTests
    .filter(row => row.status === status && allowedFiles.has(row.file))
    .map(row => row.file))].sort();
  return {
    status: ['passed', 'failed', 'incomplete'].includes(summary?.status) ? summary.status : 'incomplete',
    process: {
      status: processStatus === 'passed' ? 'passed' : 'failed',
      ...(Number.isInteger(exitCode) ? { exitCode } : {}),
      ...(timedOut ? { timedOut: true } : {}),
    },
    counts: {
      total: scopedTests.length,
      passed: scopedTests.filter(row => row.status === 'passed').length,
      failed: scopedTests.filter(row => row.status === 'failed').length,
      skipped: scopedTests.filter(row => row.status === 'skipped').length,
    },
    failedFiles: filesFor('failed'),
    failedTests: scopedTests.filter(row => row.status === 'failed').map(row => ({
      name: row.name,
      file: row.file,
      ...(row.failure ? { failure: row.failure } : {}),
    })),
    skippedFiles: filesFor('skipped'),
    missingFiles: (Array.isArray(summary?.missingFiles) ? summary.missingFiles : []).filter(file => allowedFiles.has(file)).sort(),
  };
}
