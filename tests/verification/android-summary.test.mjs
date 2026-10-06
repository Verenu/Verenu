import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizeAndroidReports } from '../../scripts/verification/android-summary.mjs';
const report = (attributes = '') => `<testsuite name="com.verenu.app.FocusTest" tests="2" skipped="0" failures="0" errors="0" ${attributes}>`;
test('Android reports require every native suite and actual unskipped cases', () => {
  const expected = ['com.verenu.app.FocusTest'];
  assert.equal(summarizeAndroidReports([report()], expected).status, 'passed');
  for (const reports of [[], ['invalid'], [report().replace('tests="2"', 'tests="0"')], [report().replace('skipped="0"', 'skipped="1"')]]) assert.equal(summarizeAndroidReports(reports, expected).status, 'incomplete');
  assert.equal(summarizeAndroidReports([report()], [...expected, 'com.verenu.app.MissingTest']).status, 'incomplete');
  assert.equal(summarizeAndroidReports([report().replace('failures="0"', 'failures="1"')], expected).status, 'failed');
});
