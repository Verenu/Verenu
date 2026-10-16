import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

it.each([
  ['Windows touch desktop', 'Windows NT 10.0', 1280, 800, 5, true, false, false, false],
  ['Wayland touch desktop', 'Linux aarch64', 400, 200, 5, true, false, false, true],
  ['Android phone', 'Android 14; Linux', 390, 844, 5, true, false, true, false],
  ['native Android with desktop UA', 'Windows NT 10.0', 1280, 800, 0, true, true, true, false],
  ['headless phone viewport', 'X11; Linux x86_64', 390, 844, 0, false, false, false, true],
  ['touch phone browser', 'X11; Linux x86_64', 390, 844, 5, false, false, true, false],
  ['wide touch browser', 'X11; Linux x86_64', 1280, 800, 5, false, false, false, true],
] as const)('%s', async (_name, userAgent, innerWidth, innerHeight, maxTouchPoints, tauri, nativeAndroid, android, linuxDesktop) => {
  vi.resetModules();
  vi.stubGlobal('navigator', { userAgent, maxTouchPoints });
  vi.stubGlobal('window', {
    innerWidth,
    innerHeight,
    ...(tauri ? { __TAURI_INTERNALS__: {} } : {}),
  });
  vi.stubGlobal('__VERENU_ANDROID__', nativeAndroid);
  const platform = await import('./platform');
  expect(platform.isAndroid).toBe(android);
  expect(platform.isLinuxDesktop).toBe(linuxDesktop);
  expect(platform.isWindows).toBe(/Win/i.test(userAgent) && !android);
});
