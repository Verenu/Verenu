// Shared reports contain classifications, never excerpts of browser errors.
const matchers = new Set([
  'toBe', 'toEqual', 'toStrictEqual', 'toBeTruthy', 'toBeFalsy', 'toContain',
  'toBeGreaterThan', 'toBeGreaterThanOrEqual', 'toBeLessThan', 'toBeLessThanOrEqual',
  'toBeVisible', 'toBeHidden', 'toBeEnabled', 'toBeDisabled', 'toBeChecked',
  'toHaveText', 'toContainText', 'toHaveValue', 'toHaveCount', 'toHaveAttribute',
  'toHaveClass', 'toHaveCSS', 'toHaveScreenshot', 'toMatchSnapshot', 'toHaveURL',
  'toHaveTitle', 'toBeInViewport', 'toMatch', 'toHaveLength',
]);
const failedStates = new Set(['failed', 'timedOut', 'interrupted']);

function errorSummary(error, status) {
  const firstLine = typeof error?.message === 'string'
    ? error.message.slice(0, 2048).split('\n', 1)[0].replace(/\u001b\[[0-9;]*m/g, '') : '';
  const matcher = firstLine.match(/^(?:Error: )?expect\([^\r\n]*\)\.(?:not\.)?([A-Za-z]+)\(/)?.[1];
  if (matchers.has(matcher)) return `Assertion failed: ${matcher}. Values and call log withheld.`;
  if (status === 'timedOut' || /^TimeoutError:/.test(firstLine)) return 'Operation timed out. Error text withheld.';
  if (status === 'interrupted') return 'Execution interrupted. Error text withheld.';
  return 'Failure details withheld or unavailable.';
}

export function safeFailureAttempts(test, file, testPointer) {
  if (!Array.isArray(test?.results)) return [];
  return test.results.flatMap((result, resultIndex) => {
    if (!failedStates.has(result?.status)) return [];
    const errors = Array.isArray(result.errors) && result.errors.length ? result.errors : result.error ? [result.error] : [];
    const summaries = [...new Set((errors.length ? errors : [null]).map(error => errorSummary(error, result.status)))];
    const location = errors.find(error => {
      const candidate = error?.location?.file;
      if (!file || typeof candidate !== 'string') return false;
      const normalized = candidate.replaceAll('\\', '/');
      return !normalized.split('/').includes('..')
        && (normalized === file || normalized.endsWith(`/${file}`))
        && Number.isSafeInteger(error.location.line) && error.location.line > 0;
    })?.location;
    // Pointers identify attachments in the private JSON report without copying
    // their names, paths, bodies, URLs, or any trace contents into CI artifacts.
    const artifacts = Array.isArray(result.attachments) ? result.attachments.flatMap((attachment, index) =>
      attachment?.name === 'trace' && attachment.contentType === 'application/zip'
        ? [{ kind: 'trace', report: 'playwright.json', pointer: `${testPointer}/results/${resultIndex}/attachments/${index}`, visibility: 'private' }]
        : []) : [];
    return [{
      resultIndex, status: result.status, summaries,
      ...(location ? { location: { file, line: location.line,
        ...(Number.isSafeInteger(location.column) && location.column > 0 ? { column: location.column } : {}) } } : {}),
      artifacts,
    }];
  });
}
