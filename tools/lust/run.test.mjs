import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./run.mjs', import.meta.url));
function invoke(mode, lust) {
  return spawnSync(process.execPath, [script, mode], {
    env: { ...process.env, LUST_BIN: lust }, encoding: 'utf8', timeout: 15_000,
  });
}

test('a missing compiler fails instead of falling back to Rust', () => {
  const result = invoke('check', '/verenu-deliberately-missing/lust');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ENOENT/);
});

test('full backend checks also fail when Lust is missing', () => {
  const result = invoke('full-check', '/verenu-deliberately-missing/lust');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ENOENT/);
});

test('the audited desktop build rejects a missing compiler', () => {
  const result = invoke('build-no-rustc', '/verenu-deliberately-missing/lust');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /No rustc-free desktop is available/);
});

test('the audited desktop build rejects an incompatible compiler before building', () => {
  const result = invoke('build-no-rustc', process.execPath);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Expected Lust 0\.2\.0/);
  assert.doesNotMatch(result.stdout, /vite.*building/);
});

test('an incompatible executable is rejected before checking sources', () => {
  const result = invoke('check', process.execPath);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Expected Lust 0\.2\.0/);
});

test('unknown commands fail with usage', () => {
  const result = invoke('typo', process.execPath);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Usage:/);
});
