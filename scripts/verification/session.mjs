import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { root, sourceIdentity } from './identity.mjs';
import { stopOwned } from './process.mjs';

export async function startOwnedSession({ id, fixtures, native = false, directory, synthetic = true }) {
  const sessionDirectory = path.join(os.homedir(), '.local/state/verenu/dev-sessions', id);
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const log = await fs.open(path.join(directory, `startup-${Date.now()}.log`), 'w', 0o600);
  let child;
  try {
    child = spawn(process.execPath, ['scripts/dev-session.mjs', '--id', id, '--fixtures', fixtures, ...(synthetic ? ['--synthetic-seed'] : []), ...(native ? ['--native-test'] : [])], { cwd: root, stdio: ['ignore', log.fd, log.fd], detached: process.platform !== 'win32' });
  } catch (error) {
    await log.close();
    throw error;
  }
  let childError;
  child.once('error', (error) => { childError = error; });
  let stopPromise;
  const stop = () => stopPromise ??= (async () => {
    const exited = new Promise((resolve) => { if (child.exitCode !== null || child.signalCode !== null || childError) resolve(); else child.once('exit', resolve); });
    stopOwned(child);
    await Promise.race([exited, new Promise((resolve) => { const timer = setTimeout(resolve, 15_000); timer.unref(); })]);
    if (child.exitCode === null && child.signalCode === null && child.pid) {
      if (process.platform !== 'win32') { try { process.kill(-child.pid, 'SIGKILL'); } catch { /* Owned child exited. */ } }
    }
    await log.close();
  })();
  try {
    const deadline = Date.now() + 900_000;
    while (Date.now() < deadline) {
      if (childError || child.exitCode !== null || child.signalCode !== null) throw new Error('Owned session failed to start; inspect its private startup log');
      try {
        const manifest = JSON.parse(await fs.readFile(path.join(sessionDirectory, 'session.json'), 'utf8'));
        if (manifest.status === 'ready' && manifest.launcherPid === child.pid) {
          const accessFile = path.join(sessionDirectory, 'access.json');
          const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
          const response = await fetch(new URL('/__verenu_dev/session', access.localAccessUrl), { headers: { Authorization: `Bearer ${access.token}` }, signal: AbortSignal.timeout(5000) });
          if (!response.ok) throw new Error(`Session metadata request returned HTTP ${response.status}`);
          const metadata = await response.json();
          const identity = sourceIdentity();
          if (metadata.fingerprint !== identity.fingerprint || metadata.worktree !== identity.worktree) throw new Error('Rust backend does not match current source');
          return { child, stop, directory: sessionDirectory, accessFile, access, metadata, identity };
        }
      } catch (error) {
        if (error.message === 'Rust backend does not match current source') throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('Owned session startup timed out');
  } catch (error) { await stop(); throw error; }
}
export async function invokeSession(session, command, args = {}) {
  const response = await fetch(new URL('/__verenu_dev/invoke', session.access.localAccessUrl), { method: 'POST', headers: { Authorization: `Bearer ${session.access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`Session command failed: ${command}, HTTP ${response.status}`);
  return response.json();
}
