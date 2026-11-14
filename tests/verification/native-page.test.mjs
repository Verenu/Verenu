import assert from 'node:assert/strict';
import test from 'node:test';
import { nativePageIsReady, runNativePhase, waitForNativePage, waitUntil } from '../../scripts/verification/native-page.mjs';

test('native phases identify timed-out operations and preserve incomplete status', async () => {
  assert.equal(await runNativePhase('Refresh native WebView', async () => 42), 42);
  const failure = Object.assign(new Error('timed out'), { verificationIncomplete: true });
  await assert.rejects(runNativePhase('Refresh native WebView', async () => { throw failure; }), error =>
    error.message === 'Refresh native WebView: timed out' && error.cause === failure && error.verificationIncomplete === true);
});

test('strict recovery waits propagate driver errors but poll false conditions', async () => {
  const failure = new Error('driver timed out');
  await assert.rejects(waitUntil(async () => { throw failure; }, 'recovery', 1000, true, async () => {}), error => error === failure);
  let calls = 0;
  await waitUntil(async () => ++calls === 2, 'recovery', 1000, true, async () => {});
  assert.equal(calls, 2);
});

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
