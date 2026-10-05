import path from 'node:path';

// Keep uploaded CI summaries useful without copying private traces, screenshots,
// assertion values, or browser logs into the shared report artifact.
export function summarizePlaywrightFailures(report) {
  const failures = [];

  function visit(suite, inheritedFile = '') {
    const suiteFile = suite.file || inheritedFile;
    for (const spec of suite.specs ?? []) {
      const file = path.basename(spec.file || suiteFile || 'unknown');
      for (const test of spec.tests ?? []) {
        const result = test.results?.at(-1);
        if (!result || !['failed', 'timedOut', 'interrupted'].includes(result.status)) continue;
        failures.push({
          project: test.projectName || 'unknown',
          file,
          line: Number.isSafeInteger(spec.line) ? spec.line : null,
          title: spec.title || 'Unnamed test',
        });
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
