import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { verifyNativePill } from '../../scripts/verification/native-pill.mjs';

const capsule = { x: 10, y: 10, left: 10, top: 10, right: 82, bottom: 44, width: 72, height: 34 };

for (const [missing, message] of [
  ['pill', 'Visible native pill is missing'],
  ['cluster', 'Native pill cluster is missing'],
  ['interactive-pill', 'Interactive native pill is missing'],
]) {
  test(`native pill verification reports a missing ${missing} as an assertion`, async () => {
    let measurements = 0;
    const browser = {
      command: async () => {},
      execute: async script => {
        measurements++;
        const pillMissing = missing === 'pill' || (missing === 'interactive-pill' && measurements > 1);
        return vm.runInNewContext(`(() => { ${script} })()`, {
          innerWidth: 96,
          innerHeight: 54,
          document: {
            querySelector: selector => selector === '.pill'
              ? pillMissing ? null : { getBoundingClientRect: () => ({ toJSON: () => capsule }) }
              : missing === 'cluster' ? null : { offsetWidth: 72, offsetHeight: 34 },
            querySelectorAll: () => [],
          },
        });
      },
    };
    await assert.rejects(verifyNativePill({
      browser,
      invoke: async () => ({ state: 'handsfree', interactive: true, rect: [10, 10, 72, 34],
        appIcon: { windowClass: 'Verenu', matchesMain: true } }),
      directory: '.',
      screenshot: async () => { throw new Error('Missing DOM must fail before capturing screenshots'); },
    }), error => error.code === 'ERR_ASSERTION' && error.message === message);
  });
}
