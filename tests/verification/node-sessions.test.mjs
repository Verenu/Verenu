import test from 'node:test';
import assert from 'node:assert/strict';
import { runNodeSessions } from '../../scripts/verification/node-sessions.mjs';
import { summarizeNodeTests, summarizeNodeFailure } from '../../scripts/verification/node-reporter.mjs';
import { basicCases, mentionCases } from '../dev-session/helpers/basic-cases.mjs';

const identity = { worktree: '/synthetic/worktree', commit: 'synthetic', fingerprint: 'a'.repeat(64) };
function fixture(overrides = {}) {
  const order = [];
  const workers = [];
  return { order, workers, options: {
    identity,
    start: async (id, file) => {
      order.push(`start:${file}`);
      const worker = { file, id, metadata: { ...identity, runs: 0, maxRuns: 30 }, stop: async () => { order.push(`stop:${file}`); } };
      workers.push(worker);
      return worker;
    },
    execute: async (file) => {
      order.push(`execute:${file}`);
      return { process: { status: 'passed', exitCode: 0 }, events: { tests: [{ name: 'required', file, status: 'passed' }] }, suite: { identity, checks: [{ name: 'same-session interactions', status: 'passed' }] } };
    },
    metadata: async (worker) => { order.push(`metadata:${worker.file}`); return { ...worker.metadata, runs: 3 }; },
    ...overrides,
  } };
}

test('file workers start fresh, retain internal interactions, capture records, and stop sequentially', async () => {
  const f = fixture();
  const result = await runNodeSessions(['one.test.mjs', 'session.test.mjs'], f.options);
  assert.equal(result.status, 'passed');
  assert.deepEqual(f.order, ['start:one.test.mjs', 'execute:one.test.mjs', 'metadata:one.test.mjs', 'stop:one.test.mjs', 'start:session.test.mjs', 'execute:session.test.mjs', 'metadata:session.test.mjs', 'stop:session.test.mjs']);
  assert.notEqual(f.workers[0].id, f.workers[1].id);
  assert.deepEqual(result.checks, [{ name: 'same-session interactions', status: 'passed' }]);
  assert.ok(result.sessions.every(shard => shard.before.runs === 0 && shard.after.runs === 3 && shard.after.maxRuns === 30 && shard.identity.fingerprint === identity.fingerprint));
});

test('runner exceptions stop their worker, preserve failure and execute remaining files once', async () => {
  const f = fixture();
  const execute = f.options.execute;
  f.options.execute = async (file) => { if (file === 'one.test.mjs') throw new Error('private runner detail'); return execute(file); };
  const result = await runNodeSessions(['one.test.mjs', 'two.test.mjs'], f.options);
  assert.equal(result.status, 'failed');
  assert.equal(f.order.filter(step => step === 'stop:one.test.mjs').length, 1);
  assert.equal(f.order.filter(step => step === 'execute:two.test.mjs').length, 1);
  assert.equal(JSON.stringify(result).includes('private runner detail'), false);
  assert.deepEqual(summarizeNodeTests(result, ['one.test.mjs', 'two.test.mjs']).missingFiles, ['one.test.mjs']);
});

test('startup rejection retains diagnostics and does not retry or run a missing worker', async () => {
  const f = fixture({ start: async () => { throw Object.assign(new Error('private startup'), { startupFailure: { manifestState: 'stopped' } }); } });
  const result = await runNodeSessions(['one.test.mjs'], f.options);
  assert.equal(result.status, 'failed');
  assert.deepEqual(result.sessions[0].startupFailure, { manifestState: 'stopped' });
  assert.deepEqual(f.order, []);
});

test('missing and foreign reports fail closed and still stop the owned worker', async () => {
  for (const events of [null, { tests: [] }, { tests: [{ name: 'wrong', file: 'outside.test.mjs', status: 'passed' }] }]) {
    const f = fixture({ execute: async () => ({ process: { status: 'passed', exitCode: 0 }, events }) });
    const result = await runNodeSessions(['one.test.mjs'], f.options);
    assert.notEqual(result.status, 'passed');
    assert.equal(f.order.at(-1), 'stop:one.test.mjs');
  }
});

test('each worker rejects stale identities, nonzero initial counters and changed limits', async () => {
  for (const wrong of [{ fingerprint: 'b'.repeat(64) }, { runs: 1 }, { maxRuns: 100 }]) {
    const f = fixture();
    const start = f.options.start;
    f.options.start = async (...args) => { const worker = await start(...args); Object.assign(worker.metadata, wrong); return worker; };
    const result = await runNodeSessions(['one.test.mjs'], f.options);
    assert.equal(result.status, 'failed');
    assert.equal(f.order.includes('execute:one.test.mjs'), false);
    assert.equal(f.order.at(-1), 'stop:one.test.mjs');
  }
});

test('final identity changes, malformed readiness and shutdown errors cannot become passes', async () => {
  for (const wrong of [{ fingerprint: 'b'.repeat(64) }, { runs: 31 }, { maxRuns: 100 }]) {
    const f = fixture({ metadata: async worker => ({ ...worker.metadata, ...wrong }) });
    assert.equal((await runNodeSessions(['one.test.mjs'], f.options)).status, 'failed');
    assert.equal(f.order.at(-1), 'stop:one.test.mjs');
  }
  const f = fixture();
  const start = f.options.start;
  f.options.start = async (...args) => ({ ...await start(...args), stop: async () => { throw new Error('stop failed'); } });
  assert.equal((await runNodeSessions(['one.test.mjs'], f.options)).status, 'failed');
});

test('missing session verification records fail closed before worker shutdown', async () => {
  const f = fixture({ execute: async file => ({ process: { status: 'passed', exitCode: 0 }, events: { tests: [{ name: 'required', file, status: 'passed' }] } }) });
  assert.equal((await runNodeSessions(['session.test.mjs'], f.options)).status, 'failed');
  assert.equal(f.order.at(-1), 'stop:session.test.mjs');
});

test('aggregate retains named failures, assertion locations, timeouts and allowed optional skip', async () => {
  const f = fixture({ execute: async file => ({ process: { status: file === 'one.test.mjs' ? 'failed' : 'passed', exitCode: file === 'one.test.mjs' ? 1 : 0, reason: file === 'one.test.mjs' ? 'Check timed out' : undefined }, events: { tests: [{ name: 'required', file, status: file === 'one.test.mjs' ? 'failed' : 'passed', ...(file === 'one.test.mjs' ? { failure: { type: 'AssertionError', code: 'ERR_ASSERTION', location: { file, line: 12 } } } : {}) }] } }) });
  const result = await runNodeSessions(['one.test.mjs', 'two.test.mjs'], f.options);
  assert.equal(result.status, 'failed');
  assert.equal(result.sessions[0].timedOut, true);
  const summary = summarizeNodeTests(result, ['one.test.mjs', 'two.test.mjs']);
  assert.deepEqual(summarizeNodeFailure(summary, ['one.test.mjs', 'two.test.mjs'], { processStatus: 'failed', exitCode: 1 }).failedTests[0].failure.location, { file: 'one.test.mjs', line: 12 });
  const optional = fixture({ execute: async file => ({ process: { status: 'passed', exitCode: 0 }, events: { tests: [{ name: 'required', file, status: 'passed' }, { name: 'live synthetic corpus reaches providers, Context rules, events, and exact new history', file, status: 'skipped' }] } }) });
  assert.equal((await runNodeSessions(['two.test.mjs'], optional.options)).status, 'passed');
  optional.options.live = true;
  assert.equal((await runNodeSessions(['two.test.mjs'], optional.options)).status, 'incomplete');
});

test('Basic groups retain 36 unique scenarios and reject duplicate file discovery', async () => {
  assert.equal(basicCases.length, 30);
  assert.equal(mentionCases.length, 6);
  assert.equal(new Set([...basicCases, ...mentionCases].map(([enabled, raw]) => JSON.stringify([enabled, raw]))).size, 36);
  await assert.rejects(runNodeSessions(['one.test.mjs', 'one.test.mjs'], fixture().options), TypeError);
});
