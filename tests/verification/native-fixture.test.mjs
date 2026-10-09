import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { formattingResult, stopFixture } from '../../scripts/verification/native-fixture.mjs';

const passed = 'test result: ok. 1 passed; 0 failed; 0 ignored;';
const cases = [1, 2, 3, 4].map(index => `VERENU_FORMAT_CASE_PASSED:${index}`).join('\n');

test('formatting requires execution of the test and each of its four cases', () => {
  assert.equal(formattingResult({ status: 'passed' }, `${cases}\n${passed}`).status, 'verified');
  for (const output of [passed, cases, `${cases}\n${passed.replace('1 passed', '0 passed')}`, `${cases}\nVERENU_FORMAT_CASE_PASSED:4\n${passed}`]) {
    assert.equal(formattingResult({ status: 'passed' }, output).status, 'failed');
  }
});

test('missing readiness is incomplete while assertion failures stay failed', () => {
  const unavailable = 'VERENU_FIXTURE_PREREQUISITE_UNAVAILABLE: deadline\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;';
  assert.equal(formattingResult({ status: 'failed' }, unavailable).status, 'incomplete');
  assert.equal(formattingResult({ status: 'failed' }, 'assertion failed').status, 'failed');
  assert.equal(formattingResult({ status: 'failed' }, 'VERENU_FIXTURE_PREREQUISITE_UNAVAILABLE: no tests ran').status, 'failed');
  assert.equal(formattingResult({ status: 'failed' }, `VERENU_FORMAT_CASE_PASSED:1\n${unavailable}`).status, 'failed');
});

test('owned fixture cleanup waits for exit, including a child ignoring SIGTERM', async () => {
  for (const ignore of [false, true]) {
    const child = spawn(process.execPath, ['-e', `${ignore ? "process.on('SIGTERM', () => {});" : ''} console.log('started'); setInterval(() => {}, 1000);`], { detached: true, stdio: ['ignore', 'pipe', 'ignore'] });
    try {
      await new Promise((resolve, reject) => { child.stdout.once('data', resolve); child.once('error', reject); });
      await stopFixture(child, 50);
      if (process.platform === 'win32') assert.notEqual(child.exitCode, null);
      else assert.equal(child.signalCode, ignore ? 'SIGKILL' : 'SIGTERM');
      await stopFixture(child);
    } finally {
      await stopFixture(child, 50);
    }
  }
});
