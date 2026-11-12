import test from 'node:test';
import assert from 'node:assert/strict';
import { classifyStartupLog, sessionIdentityMismatch, sessionStartupDiagnostics } from '../../scripts/verification/session.mjs';

test('backend identity comparison names stale source fields without exposing worktree paths', () => {
  const runner = { commit: 'runner-commit', fingerprint: 'a'.repeat(64), worktree: 'D:\\a\\Verenu\\Verenu' };
  assert.equal(sessionIdentityMismatch(runner, { ...runner }), null);

  const mismatch = sessionIdentityMismatch(runner, {
    commit: 'backend-commit', fingerprint: 'b'.repeat(64), worktree: 'D:\\a\\Verenu\\Verenu',
  });
  assert.deepEqual(mismatch, {
    fields: ['fingerprint'],
    runner: { commit: 'runner-commit', fingerprint: 'a'.repeat(64) },
    backend: { commit: 'backend-commit', fingerprint: 'b'.repeat(64) },
  });
  assert.equal(JSON.stringify(mismatch).includes('D:\\a\\'), false);
});

test('backend identity mismatch can include repo-relative digests without source contents', () => {
  const runner = { commit: 'runner-commit', fingerprint: 'a'.repeat(64), worktree: 'D:\\a\\Verenu\\Verenu' };
  const sourceChanges = {
    count: 1,
    files: [{ path: 'src/generated.rs', before: null, after: { state: 'present', mode: '33188', type: 'file', sha256: 'c'.repeat(64) } }],
    truncated: false,
  };
  const mismatch = sessionIdentityMismatch(runner, {
    ...runner, fingerprint: 'b'.repeat(64),
  }, sourceChanges);
  assert.deepEqual(mismatch.sourceChanges, sourceChanges);
  assert.equal(JSON.stringify(mismatch).includes('D:\\a\\'), false);
  assert.equal(JSON.stringify(mismatch).includes('source contents'), false);
});

test('startup diagnostics expose safe child exit details without log contents', () => {
  const diagnostics = sessionStartupDiagnostics({
    launcherPid: 42,
    status: 'stopped',
    childFailure: {
      component: 'tauri',
      kind: 'unexpected-exit',
      exitCode: 1,
      signal: null,
      message: 'SYNTHETIC_PRIVATE_STARTUP_LOG',
    },
  }, 42, { exitCode: 1, errorCode: 'not-a-safe-code' });

  assert.deepEqual(diagnostics, {
    manifestState: 'stopped',
    failedComponent: 'tauri',
    failureKind: 'unexpected-exit',
    childExitCode: 1,
    launcherExitCode: 1,
  });
  assert.equal(JSON.stringify(diagnostics).includes('SYNTHETIC_PRIVATE_STARTUP_LOG'), false);
});

test('startup diagnostics ignore manifests owned by another process', () => {
  assert.deepEqual(sessionStartupDiagnostics({
    launcherPid: 7,
    status: 'stopped',
    childFailure: { component: 'tauri', kind: 'unexpected-exit', exitCode: 1 },
  }, 42), { manifestState: 'missing' });
});

test('startup log classification exposes only fixed categories, never raw text', () => {
  const log = '\u001b[31merror[E0308]:\u001b[0m private path and value\nerror: linking with private-linker failed\nthread private-name panicked at private-file\nNo space left on device\n';
  assert.deepEqual(classifyStartupLog(log), ['rust-compiler-error', 'linker-error', 'disk-full', 'runtime-panic']);
  assert.deepEqual(classifyStartupLog('error: failed to run custom build command for private-package\nerror: failed to get private-dependency'), ['build-script-error', 'dependency-error']);
  assert.deepEqual(classifyStartupLog('private transcript and token'), []);
  assert.deepEqual(classifyStartupLog("thread 'main' panicked at private-file:\nfailed to prepare isolated dev session: private details"), ['runtime-panic', 'dev-session-prepare-panic']);
  assert.deepEqual(classifyStartupLog("thread 'main' panicked at private-file:\nerror building Verenu: private details"), ['runtime-panic', 'app-build-panic']);
  assert.deepEqual(classifyStartupLog('error[E0308]: compiler error\n  .expect("error building Verenu")'), ['rust-compiler-error']);
  assert.deepEqual(classifyStartupLog("thread 'main' panicked at private-file: error building Verenu: private details"), ['runtime-panic', 'app-build-panic']);
  assert.equal(JSON.stringify(classifyStartupLog(log)).includes('private'), false);
});
