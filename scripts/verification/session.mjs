import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { root, sourceFileChanges, sourceSnapshot } from './identity.mjs';
import { assertRunning, stopOwned, trackOwned } from './process.mjs';

export function cargoTargetDirectory(environment = process.env) {
  return environment.CARGO_TARGET_DIR || path.join(root, 'src-tauri', 'target');
}

export function ownedSessionStartupState(manifest, launcherPid) {
  if (!manifest || manifest.launcherPid !== launcherPid) return null;
  if (manifest.status === 'ready') return 'ready';
  if (manifest.status === 'starting') return 'starting';
  return 'stopped';
}

export function sessionStartupDiagnostics(manifest, launcherPid, launcher = {}) {
  const state = ownedSessionStartupState(manifest, launcherPid);
  const result = { manifestState: state ?? 'missing' };
  const failure = state === 'stopped' ? manifest.childFailure : null;
  if (failure && ['frontend', 'tauri'].includes(failure.component)) {
    result.failedComponent = failure.component;
    if (['spawn-error', 'unexpected-exit'].includes(failure.kind)) result.failureKind = failure.kind;
    if (Number.isInteger(failure.exitCode)) result.childExitCode = failure.exitCode;
    if (typeof failure.signal === 'string' && /^SIG[A-Z0-9]+$/.test(failure.signal)) result.childSignal = failure.signal;
    if (typeof failure.errorCode === 'string' && /^E[A-Z0-9_]+$/.test(failure.errorCode)) result.childSpawnErrorCode = failure.errorCode;
  }
  if (Number.isInteger(launcher.exitCode)) result.launcherExitCode = launcher.exitCode;
  if (typeof launcher.signal === 'string' && /^SIG[A-Z0-9]+$/.test(launcher.signal)) result.launcherSignal = launcher.signal;
  if (typeof launcher.errorCode === 'string' && /^E[A-Z0-9_]+$/.test(launcher.errorCode)) result.launcherSpawnErrorCode = launcher.errorCode;
  return result;
}

export function classifyStartupLog(log) {
  if (typeof log !== 'string') return [];
  const text = log.replace(/\u001b\[[0-9;]*m/g, '');
  const checks = [
    ['rust-compiler-error', /(?:^|\n)\s*error\[E\d{4}\]:/],
    ['linker-error', /(?:^|\n)\s*error: linking with\b/],
    ['build-script-error', /(?:^|\n)\s*error: failed to run custom build command\b/],
    ['dependency-error', /(?:^|\n)\s*error: failed to (?:get|download|fetch)\b/],
    ['disk-full', /(?:no space left on device|not enough space on the disk)/i],
    ['runtime-panic', /(?:^|\n)\s*thread [^\r\n]{1,200} panicked at /],
    ['dev-session-prepare-panic', /(?:^|\n)\s*thread [^\r\n]{1,200} panicked at [^\r\n]*(?:\r?\n[ \t]*)?failed to prepare isolated dev session/],
    ['app-build-panic', /(?:^|\n)\s*thread [^\r\n]{1,200} panicked at [^\r\n]*(?:\r?\n[ \t]*)?error building Verenu/],
  ];
  return checks.filter(([, pattern]) => pattern.test(text)).map(([category]) => category);
}

export function cargoPackageNames(lockfile) {
  if (typeof lockfile !== 'string') return new Set();
  return new Set([...lockfile.matchAll(/^name = "([a-zA-Z0-9_-]+)"\r?$/gm)].map((match) => match[1]));
}

export function classifyPanicLocation(log, packages = new Set()) {
  if (typeof log !== 'string') return null;
  const match = log.replace(/\u001b\[[0-9;]*m/g, '').match(/(?:^|\n)\s*thread [^\r\n]{1,200} panicked at ([^\r\n]{1,1024}?):(\d{1,6}):\d{1,6}:/);
  if (!match) return null;
  const location = match[1].replace(/\\/g, '/');
  const line = Number(match[2]);
  if (line < 1) return null;
  const crate = location.match(/(?:^|\/)([a-zA-Z0-9_-]+)-\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?\/(?:[^/]+\/)*[^/]+\.rs$/)?.[1];
  if (crate && packages.has(crate)) return { source: crate, line };
  if (location === 'src/lib.rs') return { source: 'app-lib', line };
  if (location === 'src/dev_session.rs') return { source: 'dev-session', line };
  if (/^src\/(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.rs$/.test(location)) return { source: 'app-source', line };
  if (/^(?:.*\/)?library\/(?:core|std|alloc)\/src\/.+\.rs$/.test(location)) return { source: 'rust-standard-library', line };
  return null;
}

export function sessionIdentityMismatch(expected, actual, sourceChanges = undefined) {
  const fields = [];
  if (expected?.fingerprint !== actual?.fingerprint) fields.push('fingerprint');
  if (expected?.worktree !== actual?.worktree) fields.push('worktree');
  if (!fields.length) return null;
  return {
    fields,
    runner: { commit: expected?.commit ?? null, fingerprint: expected?.fingerprint ?? null },
    backend: { commit: actual?.commit ?? null, fingerprint: actual?.fingerprint ?? null },
    ...(sourceChanges ? { sourceChanges } : {}),
  };
}

export async function startOwnedSession({ id, fixtures, native = false, directory, synthetic = true }) {
  if (typeof id !== 'string' || !/^[a-zA-Z0-9-]{1,80}$/.test(id)) {
    throw new TypeError('startOwnedSession requires a valid session ID');
  }
  if (typeof directory !== 'string' || directory.length === 0) {
    throw new TypeError('startOwnedSession requires a directory path');
  }
  const sourceAtStart = sourceSnapshot();
  const sessionDirectory = path.join(os.homedir(), '.local', 'state', 'verenu', 'dev-sessions', id);
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const logPath = path.join(directory, `startup-${Date.now()}.log`);
  const log = await fs.open(logPath, 'w', 0o600);
  let child;
  try {
    assertRunning();
    child = trackOwned(spawn(process.execPath, ['scripts/dev-session.mjs', '--id', id, ...(fixtures ? ['--fixtures', fixtures] : []), ...(synthetic ? ['--synthetic-seed'] : []), ...(native ? ['--native-test'] : [])], { cwd: root, stdio: ['ignore', log.fd, log.fd], detached: process.platform !== 'win32' }));
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
      if (process.platform !== 'win32') { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
      else { try { child.kill('SIGKILL'); } catch {} }
    }
    await log.close();
  })();
  try {
    const deadline = Date.now() + 900_000;
    let latestManifest = null;
    while (Date.now() < deadline) {
      const manifest = await fs.readFile(path.join(sessionDirectory, 'session.json'), 'utf8').then(JSON.parse).catch(() => null);
      if (manifest) latestManifest = manifest;
      const state = ownedSessionStartupState(manifest, child.pid);
      const launcher = { exitCode: child.exitCode, signal: child.signalCode, errorCode: childError?.code };
      if (childError || child.exitCode !== null || child.signalCode !== null) {
        const error = new Error('Owned session process exited before ready');
        error.startupFailure = sessionStartupDiagnostics(latestManifest, child.pid, launcher);
        throw error;
      }
      if (state === 'stopped') {
        const error = new Error('Owned session stopped before ready');
        error.startupFailure = sessionStartupDiagnostics(manifest, child.pid, launcher);
        throw error;
      }
      if (state === 'ready') {
        try {
          const accessFile = path.join(sessionDirectory, 'access.json');
          const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
          const response = await fetch(new URL('/__verenu_dev/session', access.localAccessUrl), { headers: { Authorization: `Bearer ${access.token}` }, signal: AbortSignal.timeout(5000) });
          if (!response.ok) throw new Error(`Session metadata request returned HTTP ${response.status}`);
          const metadata = await response.json();
          const currentSource = sourceSnapshot();
          const identity = currentSource.identity;
          const identityMismatch = sessionIdentityMismatch(
            identity,
            metadata,
            sourceFileChanges(sourceAtStart.files, currentSource.files),
          );
          if (identityMismatch) {
            const error = new Error(`Rust backend source identity mismatch (${identityMismatch.fields.join(', ')})`);
            error.identityMismatch = identityMismatch;
            throw error;
          }
          return { child, stop, directory: sessionDirectory, accessFile, access, metadata, identity };
        } catch (error) {
          if (error.identityMismatch) throw error;
        }
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('Owned session startup timed out');
  } catch (error) {
    await stop();
    if (error.startupFailure) {
      try {
        const file = await fs.open(logPath, 'r');
        try {
          const stat = await file.stat();
          const size = Math.min(stat.size, 256 * 1024);
          const buffer = Buffer.alloc(size);
          const { bytesRead } = await file.read(buffer, 0, size, stat.size - size);
          const text = buffer.subarray(0, bytesRead).toString('utf8');
          const categories = classifyStartupLog(text);
          if (categories.length) error.startupFailure.logCategories = categories;
          const lockfile = await fs.readFile(path.join(root, 'src-tauri', 'Cargo.lock'), 'utf8').catch(() => '');
          const panicLocation = classifyPanicLocation(text, cargoPackageNames(lockfile));
          if (panicLocation) error.startupFailure.panicLocation = panicLocation;
        } finally { await file.close(); }
      } catch {}
    }
    throw error;
  }
}
export async function invokeSession(session, command, args = {}) {
  const response = await fetch(new URL('/__verenu_dev/invoke', session.access.localAccessUrl), { method: 'POST', headers: { Authorization: `Bearer ${session.access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`Session command failed: ${command}, HTTP ${response.status}`);
  return response.json();
}
