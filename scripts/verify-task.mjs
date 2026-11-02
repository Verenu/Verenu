#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { root, sourceIdentity, changedFiles, artifact } from './verification/identity.mjs';
import { requirements, nativeRequirements, evaluate, incompleteUnlessFailed } from './verification/policy.mjs';
import { beginShutdown, run } from './verification/process.mjs';
import { matchesTaskSessionMode, matchesTaskSessionPartition, taskSessionArguments, taskSessionCoverage, taskSessionPartitions } from './verification/task-session-plan.mjs';

for (const [signal, code] of [['SIGINT', 130], ['SIGTERM', 143]]) {
  process.on(signal, () => { void beginShutdown(4000).finally(() => process.exit(code)); });
}

const args = process.argv.slice(2);
function option(name, fallback) {
  const index = args.indexOf(name);
  if (index < 0) return fallback;
  if (!args[index + 1] || args[index + 1].startsWith('--')) throw new Error(`${name} needs a value`);
  return args[index + 1];
}
if (args.includes('--help')) {
  console.log('npm run verify:task -- [--task private-task.json] [--evidence report.json] [--require session,native] [--base master] [--report PATH] [--inspect-only]');
  console.log('A changed task without acceptance criteria stays incomplete; provide --task to record observable outcomes.');
  process.exit(0);
}
const identity = sourceIdentity();
const taskPath = option('--task', null);
const task = JSON.parse(taskPath ? fs.readFileSync(taskPath, 'utf8') : '{}');
const files = changedFiles(option('--base', 'master'));
const required = requirements(files, [...(task.require || []), ...option('--require', '').split(',').filter(Boolean)]);
const native = nativeRequirements(files, task.native || []);
if (native.length && !required.includes('native-integration')) required.push('native-integration');
const directory = path.join(root, 'test-results', `task-${randomUUID()}`);
const reportPath = path.resolve(option('--report', path.join(directory, 'verification.json')));
const records = [];
const supplied = option('--evidence', null);
if (supplied) {
  const evidence = JSON.parse(fs.readFileSync(supplied, 'utf8'));
  for (const record of evidence.records || []) {
    if (!record.artifacts?.length || !record.observed) record.status = incompleteUnlessFailed(record.status);
    if (record.category === 'inspection' && (!record.inspected || !record.artifacts?.length)) record.status = incompleteUnlessFailed(record.status);
    for (const item of record.artifacts || []) {
      try { if (artifact(item.path).sha256 !== item.sha256) record.status = incompleteUnlessFailed(record.status); }
      catch { record.status = incompleteUnlessFailed(record.status); }
    }
    if (['session', 'pipeline'].includes(record.category)) {
      try {
        const report = record.sessionReport;
        const detail = JSON.parse(fs.readFileSync(report.path, 'utf8'));
        if (artifact(report.path).sha256 !== report.sha256 || detail.status !== 'verified'
          || detail.identity?.fingerprint !== identity.fingerprint || detail.identity?.worktree !== identity.worktree
          || !matchesTaskSessionPartition(detail.partition, record.partition)
          || !matchesTaskSessionMode(detail, record.category)) record.status = incompleteUnlessFailed(record.status);
        for (const item of detail.artifacts || []) {
          if (artifact(item.path).sha256 !== item.sha256) record.status = incompleteUnlessFailed(record.status);
        }
      } catch { record.status = incompleteUnlessFailed(record.status); }
    }
    records.push(record);
  }
}
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const commands = {
  static: [[npm, ['run', 'check']], [npm, ['run', 'build']]],
  unit: [[npm, ['run', 'test:unit']], [npm, ['run', 'test:verification']]],
  rust: [['cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml']], ['cargo', ['clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--features', 'dev-session,native-testing', '--', '-D', 'warnings']]],
  renderer: [[npm, ['test', '--', '--suite', 'ui,accessibility,state,performance', '--strict', '--json-report', path.join(directory, 'renderer.json')]]],
  session: taskSessionPartitions().map((partition, index) => [process.execPath, ['scripts/test-owned-session.mjs', ...taskSessionArguments(partition), '--report', path.join(directory, `session-${index}.json`)]]),
  pipeline: taskSessionPartitions().map((partition, index) => [process.execPath, ['scripts/test-owned-session.mjs', '--live', ...taskSessionArguments(partition), '--report', path.join(directory, `pipeline-${index}.json`)]]),
  native: [[npm, ['run', 'test:native:webview', '--', '--report', path.join(directory, 'native.json')]]],
  'native-integration': [[npm, ['run', 'test:native:fixtures', '--', '--report', path.join(directory, 'native-integration.json')]]],
  migration: [['cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', 'data::db', '--lib']]],
  ci: [[npm, ['run', 'test:ci']]],
  android: [[npm, ['run', 'test:android']]],
};
if (!args.includes('--inspect-only')) {
  for (let index = records.length - 1; index >= 0; index--) {
    if (records[index].category !== 'native-integration' && commands[records[index].category]?.length) records.splice(index, 1);
  }
}
if (!args.includes('--inspect-only')) {
  for (const category of required) {
    for (const [index, [command, argv]] of (commands[category] || []).entries()) {
      console.log(`Checking ${category} ${index + 1}...`);
      const result = await run(command, argv, { directory, name: `${category}-${index}` });
      let status = result.status;
      const childReport = path.join(directory, ['session', 'pipeline'].includes(category) ? `${category}-${index}.json` : `${category}.json`);
      let detail;
      if (['session', 'pipeline', 'native', 'native-integration'].includes(category)) {
        try {
          detail = JSON.parse(fs.readFileSync(childReport, 'utf8'));
          if (result.exitCode === 2 && detail.status === 'incomplete') status = 'incomplete';
          if (detail.identity?.fingerprint !== identity.fingerprint || detail.identity?.worktree !== identity.worktree) status = incompleteUnlessFailed(status);
          else if (detail.status !== 'verified') status = detail.status === 'failed' ? 'failed' : incompleteUnlessFailed(status);
          if (['session', 'pipeline'].includes(category) && (!matchesTaskSessionPartition(detail.partition, taskSessionPartitions()[index]) || !matchesTaskSessionMode(detail, category))) status = incompleteUnlessFailed(status);
          if (detail.checks?.some((row) => row.status === 'failed')) status = 'failed';
        } catch { status = result.status === 'failed' ? 'failed' : 'incomplete'; }
      }
      if (category === 'renderer') {
        try {
          const renderer = JSON.parse(fs.readFileSync(path.join(directory, 'renderer.json'), 'utf8'));
          if (renderer.tests.some((row) => row.status === 'failed')) status = 'failed';
          else if (renderer.tests.some((row) => row.status !== 'passed' || row.regression_status === 'flaky')) status = incompleteUnlessFailed(status);
        } catch { status = result.status === 'failed' ? 'failed' : 'incomplete'; }
      }
      records.push({ ...result, category, status, observed: `${category} check ${index + 1} ${status}`, fingerprint: identity.fingerprint, worktree: identity.worktree, artifacts: [artifact(result.log), ...(detail ? [artifact(childReport)] : [])], reason: detail?.reason || result.reason, ...(['session', 'pipeline'].includes(category) ? { partition: detail?.partition, ...(detail ? { sessionReport: artifact(childReport) } : {}) } : {}), ...(detail?.scope ? { scope: detail.scope, platform: detail.platform } : {}) });
    }
  }
}
for (const category of ['session', 'pipeline']) {
  if (required.includes(category) && !taskSessionCoverage(records.filter(record => record.category === category))) {
    records.push({ category, status: 'incomplete', fingerprint: identity.fingerprint, worktree: identity.worktree, reason: 'Session partition coverage is missing or duplicated' });
  }
}
if (sourceIdentity().fingerprint !== identity.fingerprint) records.push({ category: required[0] || 'static', status: 'incomplete', reason: 'Source changed while checks ran' });
const outcome = evaluate(required, records, identity, task.acceptance || [], native);
if (!task.acceptance?.length && required.length) {
  outcome.issues.push({ category: 'acceptance', status: 'incomplete', reason: 'Define task acceptance criteria with --task and supply observed outcomes' });
  if (outcome.status === 'verified') outcome.status = 'incomplete';
}
fs.mkdirSync(path.dirname(reportPath), { recursive: true });
fs.writeFileSync(reportPath, JSON.stringify({ schemaVersion: 1, ...outcome, identity, changedFiles: files, required, native, acceptance: task.acceptance || [], records, checkedAt: new Date().toISOString() }, null, 2), { mode: 0o600 });
console.log(`Task verification: ${outcome.status}. Report: ${reportPath}`);
for (const issue of outcome.issues) console.log(`${issue.category}: ${issue.reason}`);
process.exitCode = outcome.status === 'verified' ? 0 : outcome.status === 'failed' ? 1 : 2;
