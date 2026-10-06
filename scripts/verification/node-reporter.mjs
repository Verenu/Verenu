import { safeText } from './playwright-summary.mjs';

// Node's reporter stream keeps skipped cases visible without publishing errors,
// invocation arguments, console output, or private session paths.
export default async function* reporter(events) {
  const tests = [];
  for await (const event of events) {
    if (!['test:pass', 'test:fail'].includes(event.type) || event.data.details?.type === 'suite') continue;
    tests.push({
      name: safeText(event.data.name),
      file: event.data.file?.replaceAll('\\', '/').split('/tests/dev-session/')[1] || null,
      status: event.data.skip || event.data.todo ? 'skipped' : event.type === 'test:fail' ? 'failed' : 'passed',
    });
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
