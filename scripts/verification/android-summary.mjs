export function summarizeAndroidReports(reports, expectedSuites) {
  const suites = reports.map(xml => {
    const tag = xml.match(/<testsuite\b[^>]*>/)?.[0] || '';
    const attribute = name => tag.match(new RegExp(`\\b${name}="([^"]*)"`))?.[1];
    return { name: attribute('name'), tests: Number(attribute('tests')), skipped: Number(attribute('skipped')), failures: Number(attribute('failures')), errors: Number(attribute('errors')) };
  });
  const missing = expectedSuites.filter(name => !suites.some(suite => suite.name === name));
  const executed = suites.reduce((total, suite) => total + suite.tests - suite.skipped, 0);
  const failed = suites.some(suite => suite.failures > 0 || suite.errors > 0);
  const incomplete = !suites.length || missing.length > 0 || !Number.isFinite(executed) || executed <= 0 || suites.some(suite => !Number.isFinite(suite.tests) || suite.tests <= 0 || suite.skipped !== 0 || !Number.isFinite(suite.failures) || !Number.isFinite(suite.errors));
  return { status: failed ? 'failed' : incomplete ? 'incomplete' : 'passed', executed, missing };
}
