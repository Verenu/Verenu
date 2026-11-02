import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { root } from './identity.mjs';

const ownedChildren = new Set();
const groupCleanup = new WeakMap();
let shuttingDown = false;

export function assertRunning() {
  if (shuttingDown) throw new Error('Verification is shutting down');
}

export function beginShutdown(graceMs) {
  shuttingDown = true;
  return stopAllOwned(graceMs);
}

export function trackOwned(child) {
  ownedChildren.add(child);
  child.once('exit', () => {
    const cleanup = stopOwnedGroups([child], 500).finally(() => ownedChildren.delete(child));
    groupCleanup.set(child, cleanup);
  });
  child.once('error', () => ownedChildren.delete(child));
  return child;
}

export async function stopAllOwned(graceMs = 2000) {
  await stopOwnedGroups([...ownedChildren], graceMs);
}

export async function stopOwnedGroups(children, graceMs = 500) {
  const active = children.filter(child => child.pid);
  if (!active.length) return;
  for (const child of active) {
    if (process.platform === 'win32') stopOwned(child);
    else { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  }
  // A group can outlive its leader, so keep its ID for the whole grace period.
  await new Promise(resolve => setTimeout(resolve, graceMs));
  for (const child of active) {
    if (process.platform === 'win32') stopOwned(child, 'SIGKILL');
    else { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
  }
}

export function stopOwned(child, signal = 'SIGTERM') {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === 'win32') {
    const killer = spawn('taskkill', ['/F', '/T', '/PID', String(child.pid)], { stdio: 'ignore' });
    killer.on('error', () => child.kill());
  } else {
    try { process.kill(-child.pid, signal); } catch {}
  }
}
export function run(command, args, { directory, name, timeout = 900_000, env = process.env } = {}) {
  if (shuttingDown) return new Promise(() => {});
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  const log = path.join(directory, `${name}.log`);
  const output = fs.openSync(log, 'w', 0o600);
  return new Promise((resolve) => {
    const started = Date.now();
    const child = trackOwned(spawn(command, args, { cwd: root, env, stdio: ['ignore', output, output], detached: process.platform !== 'win32', shell: process.platform === 'win32' && command.endsWith('.cmd') }));
    let timedOut = false;
    let settled = false;
    let forceTimer;
    const timer = setTimeout(() => {
      timedOut = true; stopOwned(child);
      forceTimer = setTimeout(() => stopOwned(child, 'SIGKILL'), 5000);
    }, timeout);
    const finish = async (code, reason) => {
      if (settled) return;
      settled = true; clearTimeout(timer); clearTimeout(forceTimer); fs.closeSync(output);
      if (groupCleanup.has(child)) await groupCleanup.get(child);
      resolve({ status: code === 0 && !timedOut ? 'passed' : 'failed', exitCode: code, reason: timedOut ? 'Check timed out' : reason, durationMs: Date.now() - started, log });
    };
    child.once('error', (error) => finish(null, error.message));
    child.once('exit', (code, signal) => finish(code, signal ? `Process ended with ${signal}` : undefined));
  });
}
