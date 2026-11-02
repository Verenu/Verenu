import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';

const parser = 'scripts/verification/container/completed-generation.awk';
const press = 'linux hotkey: portal press';
const stop = (generation) => `lifecycle: recording -> stopping gen=${generation}`;
const complete = (generation) => `pipeline: completed gen=${generation} words=10`;
const matches = (...lines) => spawnSync('awk', ['-f', parser], { input: `${lines.join('\n')}\n` }).status === 0;

test('only the fixture recording generation can complete the shortcut', () => {
  assert.equal(matches(press, stop(12), complete(11)), false);
  assert.equal(matches(press, stop(12), complete(11), complete(12)), true);
  assert.equal(matches(press, stop(12), stop(13), complete(12)), false);
  assert.equal(matches(press, stop(12), press, complete(12)), false);
  assert.equal(matches(stop(12), complete(12)), false);
});
