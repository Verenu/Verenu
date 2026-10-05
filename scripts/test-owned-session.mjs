#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { incompleteUnlessFailed } from './verification/policy.mjs';
import { run } from './verification/process.mjs';
import { playwrightSummaryChecks, readPlaywrightReport, summarizePlaywrightReport } from './verification/playwright-summary.mjs';
import { startOwnedSession, invokeSession } from './verification/session.mjs';

const args = process.argv.slice(2);
const require = createRequire(import.meta.url);
const playwrightCli = require.resolve('@playwright/test/cli');
const reportIndex = args.indexOf('--report');
const directory = path.join(root, 'test-results', `session-${randomUUID()}`);
const reportArgument = reportIndex >= 0 ? args[reportIndex + 1] : undefined;
if (reportIndex >= 0 && (!reportArgument || reportArgument.startsWith('--'))) {
  throw new Error('--report requires a file path');
}
const reportPath = reportIndex < 0 ? path.join(directory, 'verification.json') : path.resolve(reportArgument);
const identity = sourceIdentity();
const report = { schemaVersion: 1, identity, status: 'incomplete', checks: [], artifacts: [], checkedAt: new Date().toISOString() };
let session;
try {
  const fixtures = path.join(directory, 'audio');
  const generated = await run(process.execPath, ['scripts/dev-audio-fixtures.mjs', '--out', fixtures], { directory, name: 'fixtures', timeout: 120_000 });
  assert.equal(generated.status, 'passed', 'Synthetic audio generation failed');
  const id = `verify-${randomUUID()}`;
  const synthetic = !args.includes('--live');
  session = await startOwnedSession({ id, fixtures, directory, synthetic });
  const env = { ...process.env, VERENU_SESSION_ACCESS_FILE: session.accessFile, VERENU_DEV_REQUIRE_LIVE: args.includes('--live') ? '1' : '0' };
  const tested = await run(process.execPath, ['--test', 'tests/dev-session/session.test.mjs'], { directory, name: 'session-tests', env });
  report.artifacts.push(artifact(tested.log));
  assert.equal(tested.status, 'passed', 'Real-session regression failed');
  const suite = JSON.parse(await fs.readFile(path.join(session.directory, 'verification.json'), 'utf8'));
  report.checks.push(...suite.checks);
  const playwright = await run(process.execPath, [playwrightCli, 'test', '--config', 'tests/browser/playwright.config.mjs'], { directory, name: 'playwright', env });
  report.artifacts.push(artifact(playwright.log));
  const browserReport = await readPlaywrightReport(path.join(session.directory, 'playwright.json'));
  const browserSummary = summarizePlaywrightReport(browserReport.report, {
    availability: browserReport.availability,
    processStatus: playwright.status,
    exitCode: playwright.exitCode,
    processReason: playwright.reason,
  });
  report.playwright = browserSummary;
  report.checks.push(...playwrightSummaryChecks(browserSummary));
  const failedTests = browserSummary.tests.filter((test) => test.status === 'failed');
  if (failedTests.length) {
    const cases = failedTests.map(({ project, file, line, title, menuGeometry, assertionLine }) => {
      const source = file ? `${file}:${line ?? '?'}` : 'unknown source';
      const assertion = assertionLine ? ` assertion:${assertionLine}` : '';
      const geometry = menuGeometry ? ` geometry:${JSON.stringify(menuGeometry)}` : '';
      return `${project}: ${source} ${title}${assertion}${geometry}`;
    }).join('; ');
    console.error(`Real-session Playwright failures: ${cases}`);
  }
  if (browserSummary.status !== 'passed') {
    throw new Error(failedTests.length
      ? `Real-session Playwright flows failed in ${failedTests.length} case(s).`
      : browserSummary.reason || 'Real-session Playwright flows failed.');
  }
  report.checks.push({ name: 'Real UI settings save/reload and invalid Context recovery at desktop and phone widths', status: 'passed' });
  const context = await invokeSession(session, 'create_context', { name: 'Synthetic restart', contextualFormattingDisabled: false });
  const initialLegacy = (await invokeSession(session, 'get_all_settings')).legacy_features_enabled === true;
  await invokeSession(session, 'save_setting', { key: 'legacy_features_enabled', value: !initialLegacy });
  const previousThemes = await invokeSession(session, 'get_setting', { key: 'custom_themes' });
  const restartThemes = [{ id: 'synthetic-restart', name: 'Synthetic restart', palette: { background: '#182633', foreground: '#eeeeee' }, accent: '#5ca6e8' }];
  await invokeSession(session, 'save_setting', { key: 'custom_themes', value: restartThemes });
  await session.stop(); session = undefined;
  session = await startOwnedSession({ id, fixtures, directory, synthetic });
  assert.ok((await invokeSession(session, 'get_contexts')).some((row) => row.id === context.id && row.name === 'Synthetic restart'));
  assert.equal((await invokeSession(session, 'get_all_settings')).legacy_features_enabled, !initialLegacy);
  assert.deepEqual((await invokeSession(session, 'get_all_settings')).custom_themes, restartThemes);
  await invokeSession(session, 'save_setting', { key: 'custom_themes', value: previousThemes });
  await invokeSession(session, 'save_setting', { key: 'legacy_features_enabled', value: initialLegacy });
  await invokeSession(session, 'delete_context', { contextId: context.id });
  report.checks.push({ name: 'Context and settings survive real backend restart', status: 'passed' });
  report.checks.push({ name: 'Named custom themes survive real backend restart', status: 'passed' });
  report.status = report.checks.some((row) => row.status === 'failed') ? 'failed' : 'verified';
  if (args.includes('--live') && report.checks.some((row) => row.status === 'skipped')) report.status = incompleteUnlessFailed(report.status);
  if (sourceIdentity().fingerprint !== identity.fingerprint) { report.status = incompleteUnlessFailed(report.status); report.reason = 'Source changed during verification'; }
} catch (error) {
  report.status = 'failed'; report.reason = error.message;
} finally {
  if (session) {
    try { await session.stop(); }
    catch (error) {
      report.status = 'failed';
      report.reason = [report.reason, `Could not stop owned session: ${error.message}`].filter(Boolean).join('; ');
    }
  }
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Real-session verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
