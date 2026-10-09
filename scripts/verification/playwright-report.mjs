import path from 'node:path';
import { summarizePlaywrightReport } from './playwright-summary.mjs';

// Console diagnostics use the same privacy boundary as uploaded reports.
export function summarizePlaywrightFailures(report) {
  return summarizePlaywrightReport(report).tests
    .filter(test => test.diagnostics?.length)
    .map(({ project, file, line, title, menuGeometry, assertionLine, diagnostics }) => ({
      project, file: file ? path.posix.basename(file) : 'unknown', line, title,
      ...(menuGeometry ? { menuGeometry } : {}),
      ...(assertionLine ? { assertionLine } : {}),
      diagnostics,
    }))
    .sort((left, right) => left.project.localeCompare(right.project)
      || left.file.localeCompare(right.file)
      || (left.line ?? 0) - (right.line ?? 0)
      || left.title.localeCompare(right.title));
}
