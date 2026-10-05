import path from 'node:path';

const menuIds = new Set(['history-retention-menu', 'transcription-mode-menu']);
const geometryChecks = new Set([
  'left-edge', 'right-viewport', 'top-panel', 'bottom-panel', 'left-panel', 'right-panel',
]);

function safeMenuGeometry(errors = []) {
  if (!Array.isArray(errors)) return null;
  for (const error of errors) {
    if (typeof error.message !== 'string' || error.message.length > 2048) continue;
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
      // Malformed diagnostics are omitted from shared reports.
    }
  }
  return null;
}

function safeAssertionLine(errors = [], expectedFile) {
  if (!Array.isArray(errors)) return null;
  const error = errors.find((item) => item.location
    && typeof item.location.file === 'string'
    && path.basename(item.location.file) === expectedFile
    && Number.isSafeInteger(item.location.line)
    && item.location.line > 0);
  return error?.location.line ?? null;
}

// Shared reports retain only allowlisted geometry and source line diagnostics;
// they never copy arbitrary assertion text, private traces, screenshots, or logs.
export function summarizePlaywrightFailures(report) {
  const failures = [];

  function visit(suite, inheritedFile = '') {
    const suiteFile = suite.file || inheritedFile;
    for (const spec of suite.specs ?? []) {
      const file = path.basename(spec.file || suiteFile || 'unknown');
      for (const test of spec.tests ?? []) {
        const failedResult = test.results
          ?.filter((result) => ['failed', 'timedOut', 'interrupted'].includes(result.status))
          .at(-1);
        if (!failedResult) continue;
        const failure = {
          project: test.projectName || 'unknown',
          file,
          line: Number.isSafeInteger(spec.line) ? spec.line : null,
          title: spec.title || 'Unnamed test',
        };
        const geometry = safeMenuGeometry(failedResult.errors);
        if (geometry) failure.menuGeometry = geometry;
        const assertionLine = safeAssertionLine(failedResult.errors, file);
        if (assertionLine) failure.assertionLine = assertionLine;
        failures.push(failure);
      }
    }
    for (const child of suite.suites ?? []) visit(child, suiteFile);
  }

  for (const suite of report?.suites ?? []) visit(suite);
  return failures.sort((left, right) =>
    left.project.localeCompare(right.project)
      || left.file.localeCompare(right.file)
      || (left.line ?? 0) - (right.line ?? 0)
      || left.title.localeCompare(right.title));
}
