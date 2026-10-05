// Keep CI diagnostics limited to test names and result states. Raw errors,
// traces, page URLs, and attachments can contain session credentials.
export function failedPlaywrightChecks(report) {
  const checks = [];
  function visit(suite, parents) {
    const titles = [...parents, suite.title].filter(Boolean);
    for (const spec of suite.specs || []) {
      for (const test of spec.tests || []) {
        const failed = (test.results || []).filter((result) => ['failed', 'timedOut', 'interrupted'].includes(result.status));
        if (!failed.length) continue;
        checks.push({
          name: `Playwright ${test.projectName || 'default'}: ${[...titles, spec.title].filter(Boolean).join(' > ')}`,
          status: 'failed',
          reason: failed.map((result) => result.status).join(', '),
        });
      }
    }
    for (const child of suite.suites || []) visit(child, titles);
  }
  for (const suite of report.suites || []) visit(suite, []);
  return checks;
}
