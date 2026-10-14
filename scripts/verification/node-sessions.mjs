import { randomUUID } from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { root, artifact } from './identity.mjs';
import { run } from './process.mjs';
import { sessionIdentityMismatch, startOwnedSession } from './session.mjs';
import { summarizeNodeTests } from './node-reporter.mjs';

function checkMetadata(expected, metadata, initial) {
  const mismatch = sessionIdentityMismatch(expected, metadata);
  if (mismatch) throw Object.assign(new Error('Owned file source identity mismatch'), { identityMismatch: mismatch });
  if (metadata.maxRuns !== 30 || !Number.isSafeInteger(metadata.runs)
    || metadata.runs < 0 || metadata.runs > 30 || (initial && metadata.runs !== 0)) {
    throw new Error('Owned file requires a fresh default-30 session');
  }
}

// File-local interaction contracts stay on one worker. Independent files get
// separate default-30 workers; no retry, counter reset or quota override.
export async function runNodeSessions(files, { identity, start, execute, metadata, live = false }) {
  if (!files.length || new Set(files).size !== files.length
    || files.some(file => !/^[A-Za-z0-9_-]+\.test\.mjs$/.test(file))) {
    throw new TypeError('Expected unique owned-session test files');
  }
  const result = { status: 'passed', tests: [], checks: [], artifacts: [], sessions: [] };
  for (const file of files) {
    const shard = { file, status: 'failed' };
    result.sessions.push(shard);
    let session;
    try {
      session = await start(`verify-${randomUUID()}`, file);
      checkMetadata(identity, session.metadata, true);
      shard.identity = { commit: session.metadata.commit, fingerprint: session.metadata.fingerprint };
      shard.before = { runs: session.metadata.runs, maxRuns: session.metadata.maxRuns };
      const executed = await execute(file, session);
      result.artifacts.push(...(executed.artifacts ?? []));
      const rows = executed.events?.tests;
      if (!Array.isArray(rows) || rows.some(row => row.file !== file)) {
        throw new Error('Owned file reporter is missing or contains another file');
      }
      result.tests.push(...rows);
      const summary = summarizeNodeTests(executed.events, [file], { live });
      shard.status = executed.process.status === 'passed' ? summary.status : 'failed';
      shard.exitCode = executed.process.exitCode;
      shard.timedOut = executed.process.reason === 'Check timed out';
      if (file === 'session.test.mjs') {
        if (!Array.isArray(executed.suite?.checks) || !executed.suite.checks.length
          || sessionIdentityMismatch(identity, executed.suite.identity)) {
          throw new Error('Session verification records are missing or stale');
        }
        result.checks.push(...executed.suite.checks);
        if (executed.suite.checks.some(check => check.status === 'failed')) shard.status = 'failed';
      }
    } catch (error) {
      shard.status = 'failed';
      shard.reason = 'Owned file startup, execution or reporting failed';
      if (error.startupFailure) shard.startupFailure = error.startupFailure;
      if (error.identityMismatch) shard.identityMismatch = error.identityMismatch;
    } finally {
      if (session) {
        try {
          const after = await metadata(session);
          checkMetadata(identity, after, false);
          shard.after = { runs: after.runs, maxRuns: after.maxRuns };
        } catch (error) {
          shard.status = 'failed';
          shard.reason = 'Owned file final readiness or source identity failed';
          if (error.identityMismatch) shard.identityMismatch = error.identityMismatch;
        } finally {
          try { await session.stop(); }
          catch {
            shard.status = 'failed';
            shard.reason = 'Could not stop owned file worker';
          }
        }
      }
    }
    if (shard.status === 'failed') result.status = 'failed';
    else if (shard.status !== 'passed' && result.status !== 'failed') result.status = 'incomplete';
  }
  return result;
}

export async function runOwnedNodeSuite({ identity, fixtures, directory, synthetic = true, live = false }) {
  const files = (await fs.readdir(path.join(root, 'tests/dev-session'))).filter(file => file.endsWith('.test.mjs')).sort();
  const result = await runNodeSessions(files, {
    identity, live,
    start: (id) => startOwnedSession({ id, fixtures, directory, synthetic }),
    metadata: async (session) => {
      const response = await fetch(new URL('/__verenu_dev/session', session.access.localAccessUrl), { headers: { Authorization: `Bearer ${session.access.token}` }, signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error('Owned file readiness request failed');
      return response.json();
    },
    execute: async (file, session) => {
      const name = `session-tests-${file.slice(0, -9)}`;
      const nodeReport = path.join(directory, `${name}.json`);
      const env = { ...process.env, VERENU_SESSION_ACCESS_FILE: session.accessFile, VERENU_DEV_REQUIRE_LIVE: live ? '1' : '0' };
      const tested = await run(process.execPath, ['--test', '--test-concurrency=1', '--test-reporter=spec', '--test-reporter=./scripts/verification/node-reporter.mjs', '--test-reporter-destination=stdout', `--test-reporter-destination=${nodeReport}`, `tests/dev-session/${file}`], { directory, name, env });
      const events = await fs.readFile(nodeReport, 'utf8').then(JSON.parse).catch(() => null);
      const suite = file === 'session.test.mjs'
        ? await fs.readFile(path.join(session.directory, 'verification.json'), 'utf8').then(JSON.parse).catch(() => null)
        : null;
      return { process: tested, events, suite, artifacts: [artifact(tested.log)] };
    },
  });
  return { ...result, files };
}
