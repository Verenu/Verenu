import test from 'node:test';
import assert from 'node:assert/strict';
import { stopOwned } from '../../scripts/verification/process.mjs';

test('stopOwned ignores a child process that never received a PID', () => {
  assert.doesNotThrow(() => stopOwned({ pid: undefined, exitCode: null, signalCode: null }));
});
