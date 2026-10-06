import assert from 'node:assert/strict';
import test from 'node:test';
import { nativePageIsReady, waitForNativePage } from '../../scripts/verification/native-page.mjs';

test('native page readiness requires a complete app document and the Tauri IPC bridge', () => {
  assert.equal(nativePageIsReady({ readyState: 'complete', app: true, bridge: false }), false);
  assert.equal(nativePageIsReady({ readyState: 'interactive', app: true, bridge: true }), false);
  assert.equal(nativePageIsReady({ readyState: 'complete', app: true, bridge: true }), true);
});

test('native refresh waits for a new document before allowing follow-up IPC', async () => {
  const snapshots = [
    { readyState: 'complete', app: true, bridge: true, markerPresent: true },
    { readyState: 'interactive', app: true, bridge: false, markerPresent: false },
    { readyState: 'complete', app: true, bridge: true, markerPresent: false },
  ];
  let reads = 0;
  const browser = {
    execute: async () => snapshots[reads++],
    executeAsync: async () => { throw new Error('Native navigation readiness must not use a page-bound async callback'); },
  };

  const ready = await waitForNativePage(browser, { navigationMarker: '__before_refresh', sleep: async () => {} });

  assert.deepEqual(ready, snapshots[2]);
  assert.equal(reads, 3);
});
