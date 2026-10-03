import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { root } from './identity.mjs';

export function stopOwned(child, signal = 'SIGTERM') {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === 'win32') {
    const killer = spawn('taskkill', ['/F', '/T', '/PID', String(child.pid)], { stdio: 'ignore' });
    killer.on('error', () => child.kill());
  } else {
    try { process.kill(-child.pid, signal); } catch { /* Already stopped. */ }
  }
}
export function run(command, args, { directory, name, timeout = 900_000, env = process.env } = {}) {
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  const log = path.join(directory, `${name}.log`);
  const output = fs.openSync(log, 'w', 0o600);
  return new Promise((resolve) => {
    const started = Date.now();
    const child = spawn(command, args, { cwd: root, env, stdio: ['ignore', output, output], detached: process.platform !== 'win32', shell: process.platform === 'win32' && command.endsWith('.cmd') });
    let timedOut = false;
    let settled = false;
    let forceTimer;
    const timer = setTimeout(() => {
      timedOut = true; stopOwned(child);
      forceTimer = setTimeout(() => stopOwned(child, 'SIGKILL'), 5000);
    }, timeout);
    const finish = (code, reason) => {
      if (settled) return;
      settled = true; clearTimeout(timer); clearTimeout(forceTimer); fs.closeSync(output);
      resolve({ status: code === 0 && !timedOut ? 'passed' : 'failed', exitCode: code, reason: timedOut ? 'Check timed out' : reason, durationMs: Date.now() - started, log });
    };
    child.once('error', (error) => finish(null, error.message));
    child.once('exit', (code, signal) => finish(code, signal ? `Process ended with ${signal}` : undefined));
  });
}
