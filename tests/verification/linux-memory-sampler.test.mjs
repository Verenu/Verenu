import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const cli = fileURLToPath(new URL('../../scripts/measure-linux-memory.mjs', import.meta.url));
const marker = 'SYNTHETIC_SAMPLER_PRIVACY_MARKER';
const linuxOnly = { skip: process.platform !== 'linux' };

async function fixture() {
  const source = `
    const { spawn } = require('node:child_process');
    const child = spawn(process.execPath, ['-e', 'console.log("ready"); setInterval(() => {}, 1000)', process.argv[1]], { stdio: ['ignore', 'pipe', 'ignore'] });
    child.stdout.once('data', () => console.log(child.pid));
    process.on('SIGTERM', () => { child.once('exit', () => process.exit(0)); child.kill('SIGTERM'); });
    setInterval(() => {}, 1000);
  `;
  const root = spawn(process.execPath, ['-e', source, marker], {
    stdio: ['ignore', 'pipe', 'ignore'],
    env: { ...process.env, SAMPLER_TEST_SECRET: marker },
  });
  try {
    const pid = await new Promise((resolve, reject) => {
      let output = '';
      const timer = setTimeout(() => finish(new Error('Fixture did not become ready')), 10000);
      const onData = data => {
        output += data;
        if (output.includes('\n')) finish(null, Number(output.trim()));
      };
      const onExit = () => finish(new Error('Fixture exited before becoming ready'));
      const onError = error => finish(error);
      function finish(error, value) {
        clearTimeout(timer);
        root.stdout.off('data', onData);
        root.off('exit', onExit);
        root.off('error', onError);
        if (error) reject(error); else resolve(value);
      }
      root.stdout.on('data', onData);
      root.once('exit', onExit);
      root.once('error', onError);
    });
    assert.ok(Number.isInteger(pid) && pid > 0);
    return { root, childPid: pid };
  } catch (error) {
    root.kill('SIGTERM');
    throw error;
  }
}

async function stop(root) {
  if (root.exitCode !== null || root.signalCode !== null) return;
  const exited = once(root, 'exit');
  root.kill('SIGTERM');
  await Promise.race([exited, delay(5000, undefined, { ref: false })]);
  if (root.exitCode === null && root.signalCode === null) {
    root.kill('SIGKILL');
    await exited;
  }
}

test('reports PSS for the root and its child without exposing command or environment data', linuxOnly, async () => {
  const { root, childPid } = await fixture();
  try {
    const { stdout, stderr } = await exec(process.execPath, [cli, '--pid', String(root.pid), '--samples', '2', '--interval-ms', '20'], { timeout: 10000 });
    assert.equal(stderr, '');
    assert.equal(stdout.includes(marker), false);
    const report = JSON.parse(stdout);
    assert.equal(report.rootPid, root.pid);
    assert.equal(report.unit, 'KiB');
    assert.equal(report.samples.length, 2);
    for (const sample of report.samples) {
      assert.deepEqual(new Set(sample.processes.map(process => process.pid)), new Set([root.pid, childPid]));
      assert.ok(sample.processes.every(process => process.pssKiB > 0));
      assert.equal(sample.totalPssKiB, sample.processes.reduce((sum, process) => sum + process.pssKiB, 0));
      assert.ok(Number.isFinite(Date.parse(sample.timestamp)));
    }
    const totals = report.samples.map(sample => sample.totalPssKiB);
    assert.equal(report.summary.averagePssKiB, (totals[0] + totals[1]) / 2);
    assert.equal(report.summary.minPssKiB, Math.min(...totals));
    assert.equal(report.summary.maxPssKiB, Math.max(...totals));
  } finally {
    await stop(root);
  }
});

test('rejects invalid arguments without emitting a measurement', async () => {
  for (const args of [[], ['--pid', '0'], ['--pid', '1', '--samples', '0'], ['--pid', '1', '--interval-ms', '0'], ['--pid', '1', '--unknown', '1']]) {
    await assert.rejects(exec(process.execPath, [cli, ...args], { timeout: 10000 }), error => {
      assert.notEqual(error.code, 0);
      assert.equal(error.stdout, '');
      assert.match(error.stderr, /Usage:/);
      return true;
    });
  }
});

test('fails when the supplied root exits during sampling', linuxOnly, async () => {
  const { root } = await fixture();
  try {
    const pending = exec(process.execPath, [cli, '--pid', String(root.pid), '--samples', '2', '--interval-ms', '1500'], { timeout: 10000 })
      .then(value => ({ value }), error => ({ error }));
    await delay(250);
    await stop(root);
    const result = await pending;
    assert.ok(result.error, 'sampling an exited process must fail');
    assert.equal(result.error.stdout, '');
    assert.match(result.error.stderr, /exited/);
  } finally {
    await stop(root);
  }
});
