import fs from 'node:fs/promises';
import path from 'node:path';
import { root } from './identity.mjs';
import { safeFailureAttempts } from './playwright-diagnostics.mjs';

const menuIds = new Set(['history-retention-menu', 'transcription-mode-menu']);
const geometryChecks = new Set([
  'left-edge', 'right-viewport', 'top-panel', 'bottom-panel', 'left-panel', 'right-panel',
]);

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export async function readPlaywrightReport(file) {
  try {
    const report = JSON.parse(await fs.readFile(file, 'utf8'));
    return isRecord(report) && Array.isArray(report.suites)
      ? { availability: 'available', report }
      : { availability: 'invalid', report: null };
  } catch (error) {
    return { availability: error.code === 'ENOENT' ? 'missing' : 'invalid', report: null };
  }
}

function relativeRepoFile(file, projectRoot) {
  if (typeof file !== 'string' || file.length === 0) return null;
  const rootPath = path.resolve(projectRoot);
  const normalized = file.replaceAll('\\', '/').replace(/^\.\//, '');
  if (!path.isAbsolute(file) && /^[A-Za-z]:\//.test(normalized)) return null;
  const testDirectory = path.join(rootPath, 'tests', 'browser');
  const isRepoRelative = normalized === 'tests/browser' || normalized.startsWith('tests/browser/');
  const base = isRepoRelative ? rootPath : testDirectory;
  const resolved = path.isAbsolute(file) ? path.resolve(file) : path.resolve(base, normalized);
  const relative = path.relative(rootPath, resolved);
  if (!relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) return null;
  return relative.split(path.sep).join('/');
}

export function safeText(value, limit = 240) {
  if (typeof value !== 'string') return '';
  return value
    .replace(/https?:\/\/\S+/gi, '[url]')
    .replace(/\b(?:Bearer\s+\S+|(?:sk[-_]|gh[pousr]_|github_pat_)[A-Za-z0-9_-]{8,})\b/gi, '[redacted]')
    .replace(/\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/gi, '[redacted]')
    .replace(/\b[^\s@]+@[^\s@]+\.[^\s@]+\b/g, '[redacted]')
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, limit);
}

function safeMenuGeometry(errors = []) {
  if (!Array.isArray(errors)) return null;
  for (const error of errors) {
    if (typeof error?.message !== 'string' || error.message.length > 2048) continue;
    const match = error.message.match(/MENU_GEOMETRY:(\{[^\r\n]+\})/);
    if (!match) continue;
    try {
      const geometry = JSON.parse(match[1]);
      const rectangle = (rect) => rect
        && ['x', 'y', 'width', 'height'].every((key) => Number.isFinite(rect[key]))
        && rect.width >= 0 && rect.height >= 0
        ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
        : null;
      const bounds = rectangle(geometry.bounds);
      const content = rectangle(geometry.content);
      if (!menuIds.has(geometry.menuId) || !geometryChecks.has(geometry.check)
        || !bounds || !content
        || !Number.isFinite(geometry.viewport?.width) || geometry.viewport.width < 0
        || !Number.isFinite(geometry.viewport?.height) || geometry.viewport.height < 0) continue;
      return {
        check: geometry.check,
        menuId: geometry.menuId,
        bounds,
        content,
        viewport: { width: geometry.viewport.width, height: geometry.viewport.height },
      };
    } catch {
      // Ignore malformed diagnostics. They may contain arbitrary page data.
    }
  }
  return null;
}

function safeAssertionLine(errors = [], expectedFile) {
  if (!Array.isArray(errors)) return null;
  const expected = typeof expectedFile === 'string' ? expectedFile.replaceAll('\\', '/').split('/').at(-1) : null;
  const error = errors.find((item) => {
    const file = item?.location?.file;
    if (typeof file !== 'string' || !expected) return false;
    return file.replaceAll('\\', '/').split('/').at(-1) === expected
      && Number.isSafeInteger(item.location.line) && item.location.line > 0;
  });
  return error?.location.line ?? null;
}

function browserTestStatus(value) {
  switch (value) {
    case 'expected':
    case 'passed':
      return 'passed';
    case 'skipped':
      return 'skipped';
    default:
      // Unexpected, flaky, interrupted, timed out, and unknown outcomes must
      // remain visible as failures in the strict session gate.
      return 'failed';
  }
}

function collectTests(report, projectRoot) {
  const tests = [];

  function walk(suite, parentTitles = [], inheritedFile = null, pointer = '') {
    if (!isRecord(suite)) return;
    const suiteFile = typeof suite.file === 'string' ? suite.file : inheritedFile;
    const suiteTitle = safeText(suite.title);
    const isFileTitle = suiteFile && suiteTitle === path.posix.basename(suiteFile.replaceAll('\\', '/'));
    const titles = suiteTitle && !isFileTitle ? [...parentTitles, suiteTitle] : parentTitles;

    if (Array.isArray(suite.specs)) {
      for (const [specIndex, spec] of suite.specs.entries()) {
        if (!isRecord(spec) || !Array.isArray(spec.tests)) continue;
        const file = relativeRepoFile(spec.file ?? suiteFile, projectRoot);
        const line = Number.isInteger(spec.line) && spec.line > 0 ? spec.line : null;
        const leaf = safeText(spec.title);
        const title = [...titles, leaf].filter(Boolean).join(' › ').slice(0, 400) || 'Untitled Playwright test';
        for (const [testIndex, test] of spec.tests.entries()) {
          if (!isRecord(test)) continue;
          const results = Array.isArray(test.results) ? test.results : [];
          const lastResult = results.filter(result => ['failed', 'timedOut', 'interrupted'].includes(result?.status)).at(-1) ?? results.at(-1);
          const row = {
            project: safeText(test.projectName, 80) || 'unknown',
            file,
            line,
            title,
            status: test.status === 'skipped' ? 'skipped'
              : results.length !== 1 || results[0].status !== 'passed' || (test.expectedStatus && test.expectedStatus !== 'passed') ? 'failed'
                : browserTestStatus(test.status),
            retryCount: Math.max(0, results.length - 1),
          };
          const menuGeometry = safeMenuGeometry(lastResult?.errors);
          if (menuGeometry) row.menuGeometry = menuGeometry;
          const assertionLine = safeAssertionLine(lastResult?.errors, spec.file ?? suiteFile);
          if (assertionLine) row.assertionLine = assertionLine;
          const diagnostics = safeFailureAttempts(test, file, `${pointer}/specs/${specIndex}/tests/${testIndex}`);
          if (diagnostics.length) row.diagnostics = diagnostics;
          tests.push(row);
        }
      }
    }

    if (Array.isArray(suite.suites)) {
      for (const [index, child] of suite.suites.entries()) walk(child, titles, suiteFile, `${pointer}/suites/${index}`);
    }
  }

  for (const [index, suite] of report.suites.entries()) walk(suite, [], null, `/suites/${index}`);
  return tests;
}

function processReason({ processStatus, exitCode, processReason: rawReason }) {
  if (processStatus === 'passed') return undefined;
  if (rawReason === 'Check timed out') return 'Playwright process timed out.';
  if (Number.isInteger(exitCode)) return `Playwright process exited with code ${exitCode}.`;
  return 'Playwright process did not complete successfully.';
}

export function summarizePlaywrightReport(report, {
  availability = isRecord(report) && Array.isArray(report.suites) ? 'available' : 'missing',
  processStatus = 'passed',
  exitCode = null,
  processReason: rawProcessReason,
  projectRoot = root,
  expectedProjects = [],
} = {}) {
  const reportAvailable = availability === 'available' && isRecord(report) && Array.isArray(report.suites);
  const tests = reportAvailable ? collectTests(report, projectRoot) : [];
  const failedCases = tests.filter((test) => test.status === 'failed');
  const failedProcess = processStatus !== 'passed';
  const missingCases = reportAvailable && tests.length === 0;
  const skippedCases = tests.filter((test) => test.status === 'skipped');
  const missingProjects = expectedProjects.filter((project) => !tests.some((test) => test.project === project));
  const status = failedProcess || failedCases.length > 0 || !reportAvailable || missingCases ? 'failed'
    : skippedCases.length || missingProjects.length ? 'incomplete' : 'passed';
  const safeProcessReason = processReason({ processStatus, exitCode, processReason: rawProcessReason });
  let reason;
  if (failedProcess) reason = safeProcessReason;
  else if (failedCases.length > 0) reason = 'One or more Playwright cases did not reach their expected outcome.';
  else if (!reportAvailable) reason = 'Structured Playwright report is missing or invalid.';
  else if (missingCases) reason = 'Playwright report contains no test cases.';
  else if (missingProjects.length) reason = `Playwright projects did not run: ${missingProjects.join(', ')}.`;
  else if (skippedCases.length) reason = 'Required Playwright cases were skipped.';

  return {
    availability: reportAvailable ? 'available' : availability === 'invalid' ? 'invalid' : 'missing',
    status,
    process: {
      status: failedProcess ? 'failed' : 'passed',
      ...(Number.isInteger(exitCode) ? { exitCode } : {}),
      ...(safeProcessReason ? { reason: safeProcessReason } : {}),
    },
    counts: {
      total: tests.length,
      passed: tests.filter((test) => test.status === 'passed').length,
      failed: failedCases.length,
      skipped: tests.filter((test) => test.status === 'skipped').length,
      retries: tests.reduce((sum, test) => sum + test.retryCount, 0),
    },
    tests,
    missingProjects,
    ...(reason ? { reason } : {}),
  };
}

export function playwrightSummaryChecks(summary) {
  const checks = [
    {
      name: 'Real-session Playwright browser flows',
      status: summary.status,
      ...(summary.reason ? { reason: summary.reason } : {}),
    },
  ];
  for (const test of summary.tests) {
    if (test.status === 'passed') continue;
    checks.push({
      name: `Playwright ${test.project}: ${test.title}`,
      status: test.status === 'skipped' ? 'incomplete' : 'failed',
      project: test.project,
      file: test.file,
      line: test.line,
      title: test.title,
      retryCount: test.retryCount,
      ...(test.diagnostics ? { diagnostics: test.diagnostics } : {}),
    });
  }
  return checks;
}
