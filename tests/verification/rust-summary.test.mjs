import test from 'node:test';
import assert from 'node:assert/strict';
import { executedRustTests } from '../../scripts/verification/rust-summary.mjs';

test('zero matches and ignored Rust tests cannot establish fixture execution', () => {
  assert.equal(executedRustTests('test result: ok. 0 passed; 0 failed; 1 ignored;'), 0);
  assert.equal(executedRustTests('test result: ok. 1 passed; 0 failed; 0 ignored;\ntest result: ok. 0 passed; 0 failed;'), 1);
  assert.equal(executedRustTests('test result: FAILED. 0 passed; 2 failed;'), 2);
  assert.equal(executedRustTests('Finished build'), 0);
});
