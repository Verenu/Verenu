#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { incompleteUnlessFailed } from './verification/policy.mjs';
import { summarizePlaywrightFailures } from './verification/playwright-report.mjs';
import { beginShutdown, run } from './verification/process.mjs';
import { playwrightSummaryChecks, readPlaywrightReport, summarizePlaywrightReport } from './verification/playwright-summary.mjs';
import { startOwnedSession, invokeSession } from './verification/session.mjs';
import { summarizeNodeFailure, summarizeNodeTests } from './verification/node-reporter.mjs';
import { runOwnedNodeSuite } from './verification/node-sessions.mjs';
import { sessionPartition } from './verification/session-partition.mjs';

const args = process.argv.slice(2);
for (const [signal, code] of [['SIGINT', 130], ['SIGTERM', 143]]) {
  process.on(signal, () => { void beginShutdown().finally(() => process.exit(code)); });
}
const partition = sessionPartition(args);
const started = performance.now();
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
const report = { schemaVersion: 1, identity, partition, live: args.includes('--live'), status: 'incomplete', checks: [], artifacts: [], timing: {}, checkedAt: new Date().toISOString() };
let session;
try {
  const fixtures = path.join(directory, 'audio');
  const generated = await run(process.execPath, ['scripts/dev-audio-fixtures.mjs', '--out', fixtures], { directory, name: 'fixtures', timeout: 120_000 });
  assert.equal(generated.status, 'passed', 'Synthetic audio generation failed');
  const id = `verify-${randomUUID()}`;
  const synthetic = !args.includes('--live');
  if (partition.suite !== 'browser') {
    const nodes = await runOwnedNodeSuite({ identity, fixtures, directory, synthetic, live: args.includes('--live'), partition });
    report.artifacts.push(...nodes.artifacts);
    report.nodeSessions = nodes.sessions;
    const nodeSummary = summarizeNodeTests({ tests: nodes.tests }, nodes.files, { live: args.includes('--live') });
    if (nodes.status !== 'passed' || nodeSummary.status !== 'passed') {
      if (nodes.status === 'failed') nodeSummary.status = 'failed';
      report.nodeFailure = summarizeNodeFailure(nodeSummary, nodes.files, {
        processStatus: nodes.status === 'failed' ? 'failed' : 'passed',
        exitCode: nodes.status === 'failed' ? 1 : 0,
        timedOut: nodes.sessions.some(shard => shard.timedOut),
      });
      if (nodes.status !== 'failed') throw Object.assign(new Error('Owned-session cases were skipped or missing'), { verificationStatus: 'incomplete' });
      throw new Error('Real-session regression failed');
    }
    report.node = nodeSummary;
    report.checks.push({ name: 'Every owned-session test file executed without unexpected skips', status: report.node.status });
    report.checks.push(...nodes.checks);
  }
  if (partition.suite !== 'node') {
    console.log(`Owned browser: starting ${partition.project} ${partition.index}/${partition.count}`);
    const browserStarted = performance.now();
    session = await startOwnedSession({ id, fixtures, directory, synthetic: true });
    report.timing.browserStartupMs = Math.round(performance.now() - browserStarted);
    const env = { ...process.env, VERENU_SESSION_ACCESS_FILE: session.accessFile, VERENU_DEV_REQUIRE_LIVE: args.includes('--live') ? '1' : '0' };
    const browserEnv = args.includes('--update-snapshots') ? { ...env, VERENU_SNAPSHOT_SOURCE_FINGERPRINT: identity.fingerprint } : env;
    const playwright = await run(process.execPath, [playwrightCli, 'test', '--config', 'tests/browser/playwright.config.mjs', ...(partition.project === 'all' ? [] : [`--project=${partition.project}`]), ...(partition.count === 1 ? [] : [`--shard=${partition.index}/${partition.count}`]), ...(args.includes('--update-snapshots') ? ['--update-snapshots=all'] : [])], { directory, name: 'playwright', env: browserEnv });
    report.timing.playwrightMs = playwright.durationMs;
    report.artifacts.push(artifact(playwright.log));
    const browserReport = await readPlaywrightReport(path.join(session.directory, 'playwright.json'));
    const browserSummary = summarizePlaywrightReport(browserReport.report, {
      availability: browserReport.availability,
      processStatus: playwright.status,
      exitCode: playwright.exitCode,
      processReason: playwright.reason,
      expectedProjects: partition.project === 'all' ? ['desktop', 'phone'] : [partition.project],
    });
    const failureDiagnostics = summarizePlaywrightFailures(browserReport.report);
    report.playwright = { ...browserSummary, failureDiagnostics };
    report.checks.push(...playwrightSummaryChecks(browserSummary));
    const failedTests = browserSummary.tests.filter((test) => test.status === 'failed');
    if (failureDiagnostics.length) {
      const cases = failureDiagnostics.map(({ project, file, line, title, menuGeometry, assertionLine }) => {
        const source = file ? `${file}:${line ?? '?'}` : 'unknown source';
        const assertion = assertionLine ? ` assertion:${assertionLine}` : '';
        const geometry = menuGeometry ? ` geometry:${JSON.stringify(menuGeometry)}` : '';
        return `${project}: ${source} ${title}${assertion}${geometry}`;
      }).join('; ');
      console.error(`Real-session Playwright failure diagnostics: ${cases}`);
    }
    if (browserSummary.status !== 'passed') {
      throw Object.assign(new Error(failedTests.length
        ? `Real-session Playwright flows failed in ${failedTests.length} case(s).`
        : browserSummary.reason || 'Real-session Playwright flows failed.'), { verificationStatus: browserSummary.status });
    }
    const restartStarted = performance.now();
    const context = await invokeSession(session, 'create_context', { name: 'Synthetic restart', contextualFormattingOverride: null });
    const initialLegacy = (await invokeSession(session, 'get_all_settings')).legacy_features_enabled === true;
    await invokeSession(session, 'save_setting', { key: 'legacy_features_enabled', value: !initialLegacy });
    const previousThemes = await invokeSession(session, 'get_setting', { key: 'custom_themes' });
    const restartThemes = [{ id: 'synthetic-restart', name: 'Synthetic restart', palette: { background: '#182633', foreground: '#eeeeee' }, accent: '#5ca6e8' }];
    await invokeSession(session, 'save_setting', { key: 'custom_themes', value: restartThemes });
    await session.stop(); session = undefined;
    session = await startOwnedSession({ id, fixtures, directory, synthetic: true });
    assert.ok((await invokeSession(session, 'get_contexts')).some((row) => row.id === context.id && row.name === 'Synthetic restart'));
    assert.equal((await invokeSession(session, 'get_all_settings')).legacy_features_enabled, !initialLegacy);
    assert.deepEqual((await invokeSession(session, 'get_all_settings')).custom_themes, restartThemes);
    await invokeSession(session, 'save_setting', { key: 'custom_themes', value: previousThemes });
    await invokeSession(session, 'save_setting', { key: 'legacy_features_enabled', value: initialLegacy });
    await invokeSession(session, 'delete_context', { contextId: context.id });
    report.checks.push({ name: 'Context and settings survive real backend restart', status: 'passed' });
    report.checks.push({ name: 'Named custom themes survive real backend restart', status: 'passed' });
    report.timing.persistenceMs = Math.round(performance.now() - restartStarted);
    console.log(`Owned browser: ${partition.project} passed (${report.timing.playwrightMs} ms tests)`);
  }
  report.status = report.checks.some((row) => row.status === 'failed') ? 'failed' : 'verified';
  if (args.includes('--live') && report.checks.some((row) => row.status === 'skipped')) report.status = incompleteUnlessFailed(report.status);
  if (sourceIdentity().fingerprint !== identity.fingerprint) { report.status = incompleteUnlessFailed(report.status); report.reason = 'Source changed during verification'; }
} catch (error) {
  report.status = error.verificationStatus === 'incomplete' ? 'incomplete' : 'failed'; report.reason = error.message;
  if (error.startupFailure) report.startupFailure = error.startupFailure;
  if (error.identityMismatch) report.identityMismatch = error.identityMismatch;
} finally {
  if (session) {
    try { await session.stop(); }
    catch (error) {
      report.status = 'failed';
      report.reason = [report.reason, `Could not stop owned session: ${error.message}`].filter(Boolean).join('; ');
    }
  }
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  report.timing.totalMs = Math.round(performance.now() - started);
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Real-session verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
