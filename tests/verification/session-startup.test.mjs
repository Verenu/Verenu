import test from 'node:test';
import assert from 'node:assert/strict';
import { sessionIdentityMismatch, sessionStartupDiagnostics } from '../../scripts/verification/session.mjs';

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
