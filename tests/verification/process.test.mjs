import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { run, stopAllOwned, stopOwned, trackOwned } from '../../scripts/verification/process.mjs';

function isDead(pid) {
  try { return fs.readFileSync(`/proc/${pid}/stat`, 'utf8').split(' ')[2] === 'Z'; }
  catch (error) { if (error.code === 'ENOENT') return true; throw error; }
}

test('stopOwned ignores a child process that never received a PID', () => {
  assert.doesNotThrow(() => stopOwned({ pid: undefined, exitCode: null, signalCode: null }));
});

test('cancellation kills descendants even after their group leader exits', { skip: process.platform !== 'linux' }, async () => {
  const script = `const { spawn } = require('node:child_process');
    const child = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"], { stdio: ['ignore', 'pipe', 'ignore'] });
    child.stdout.once('data', () => console.log(child.pid));
    setInterval(() => {}, 1000);`;
  const leader = trackOwned(spawn(process.execPath, ['-e', script], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] }));
  let descendant;
  try {
    descendant = Number((await once(leader.stdout, 'data'))[0].toString().trim());
    assert.ok(descendant > 0);
    await stopAllOwned(100);
    await new Promise(resolve => setTimeout(resolve, 50));
    assert.ok(isDead(descendant));
  } finally {
    if (descendant) { try { process.kill(descendant, 'SIGKILL'); } catch {} }
    stopOwned(leader, 'SIGKILL');
  }
});

test('timeout cleans up a descendant after the command leader exits', { skip: process.platform !== 'linux' }, async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-process-test-'));
  const script = `const { spawn } = require('node:child_process');
    const child = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"], { stdio: ['ignore', 'pipe', 'ignore'] });
    child.stdout.once('data', () => console.log(child.pid));
    setInterval(() => {}, 1000);`;
  let descendant;
  try {
    const result = await run(process.execPath, ['-e', script], { directory, name: 'timeout', timeout: 500 });
    descendant = Number(fs.readFileSync(result.log, 'utf8').trim());
    assert.ok(descendant > 0);
    assert.equal(result.reason, 'Check timed out');
    await new Promise(resolve => setTimeout(resolve, 50));
    assert.ok(isDead(descendant));
  } finally {
    if (descendant) { try { process.kill(descendant, 'SIGKILL'); } catch {} }
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test('an exited leader does not leave an owned descendant running', { skip: process.platform !== 'linux' }, async () => {
  const script = `const { spawn } = require('node:child_process');
    const child = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"], { stdio: ['ignore', 'pipe', 'ignore'] });
    child.stdout.once('data', () => { console.log(child.pid); process.exit(0); });`;
  const leader = trackOwned(spawn(process.execPath, ['-e', script], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] }));
  let descendant;
  try {
    descendant = Number((await once(leader.stdout, 'data'))[0].toString().trim());
    await once(leader, 'exit');
    await stopAllOwned(50);
    await new Promise(resolve => setTimeout(resolve, 50));
    assert.ok(isDead(descendant));
  } finally {
    if (descendant) { try { process.kill(descendant, 'SIGKILL'); } catch {} }
  }
});

test('cancelling the runner stops its detached children', { skip: process.platform === 'win32' }, async () => {
  const child = trackOwned(spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { detached: true, stdio: 'ignore' }));
  const exited = once(child, 'exit');
  try {
    await stopAllOwned();
    const [, signal] = await exited;
    assert.equal(signal, 'SIGTERM');
  } finally {
    stopOwned(child, 'SIGKILL');
  }
});

test('cancellation escalates when an owned child ignores termination', { skip: process.platform === 'win32' }, async () => {
  const child = trackOwned(spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] }));
  const exited = once(child, 'exit');
  try {
    await once(child.stdout, 'data');
    await stopAllOwned(50);
    const [, signal] = await exited;
    assert.equal(signal, 'SIGKILL');
  } finally {
    stopOwned(child, 'SIGKILL');
  }
});

test('renderer runner cancellation stops its separately owned process group', { skip: process.platform !== 'linux' }, async () => {
  const script = `import importlib.util, signal, subprocess, sys, time
spec = importlib.util.spec_from_file_location('runner', 'tests/OnePyFone.py')
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)
signal.signal(signal.SIGTERM, runner._shutdown)
child = runner._start_owned([sys.executable, '-c', 'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)'], start_new_session=True)
time.sleep(0.2)
print(child.pid, flush=True)
while not runner._STOPPING.is_set():
    time.sleep(0.01)`;
  const runner = trackOwned(spawn('python3', ['-c', script], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] }));
  let child;
  try {
    child = Number((await once(runner.stdout, 'data'))[0].toString().trim());
    assert.ok(child > 0);
    await stopAllOwned(1000);
    assert.ok(isDead(child));
  } finally {
    if (child) { try { process.kill(-child, 'SIGKILL'); } catch {} }
    stopOwned(runner, 'SIGKILL');
  }
});
