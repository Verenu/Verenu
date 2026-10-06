import { randomUUID } from 'node:crypto';

export async function markCurrentNativePage(browser) {
  const marker = `__verenu_native_page_${randomUUID().replaceAll('-', '')}`;
  const installed = await browser.execute(`window[${JSON.stringify(marker)}] = true; return window[${JSON.stringify(marker)}];`);
  if (installed !== true) throw new Error('Could not mark the current native WebView document');
  return marker;
}

export function nativePageIsReady(snapshot, navigationMarker) {
  return snapshot?.readyState === 'complete'
    && snapshot.app === true
    && snapshot.bridge === true
    && (!navigationMarker || snapshot.markerPresent !== true);
}

export async function waitForNativePage(browser, {
  navigationMarker,
  deadlineMs = 30_000,
  pollMs = 100,
  sleep = ms => new Promise(resolve => setTimeout(resolve, ms)),
} = {}) {
  const deadline = Date.now() + deadlineMs;
  const markerProbe = navigationMarker
    ? `markerPresent: window[${JSON.stringify(navigationMarker)}] === true,`
    : '';
  while (Date.now() < deadline) {
    const snapshot = await browser.execute(`return {
      readyState: document.readyState,
      app: !!document.querySelector('.app'),
      bridge: !!window.__TAURI_INTERNALS__ && typeof window.__TAURI__?.core?.invoke === 'function',
      ${markerProbe}
    };`);
    if (nativePageIsReady(snapshot, navigationMarker)) return snapshot;
    await sleep(pollMs);
  }
  throw new Error(navigationMarker
    ? 'Native WebView did not finish navigation with its Tauri bridge ready'
    : 'Native WebView did not render .app with its Tauri bridge ready');
}
