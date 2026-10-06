import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ betaUpdatesEnabled: false, updateInstalling: false, updateInstalled: false, updateInfo: null as unknown }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock('./stores', () => ({ appStore: state }));
vi.mock('./tauri', () => ({ invoke }));
vi.mock('./settings', () => ({ saveSetting: vi.fn() }));
vi.mock('./notifications', () => ({ ensureNotificationPermission: vi.fn(async () => false) }));
import { startAutomaticUpdateChecks } from './updates';
let stop: (() => void) | undefined;
let browser: EventTarget;
beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(state, { betaUpdatesEnabled: false, updateInstalling: false, updateInstalled: false, updateInfo: null });
  browser = Object.assign(new EventTarget(), { setInterval: vi.fn(() => 1), clearInterval: vi.fn() });
  vi.stubGlobal('window', browser);
  invoke.mockResolvedValue(null);
});
afterEach(() => { stop?.(); stop = undefined; vi.unstubAllGlobals(); });
it('coalesces online events while a check is running and retries when back online', async () => {
  let resolve!: (value: unknown) => void;
  invoke.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  stop = startAutomaticUpdateChecks();
  browser.dispatchEvent(new Event('online'));
  browser.dispatchEvent(new Event('online'));
  expect(invoke).toHaveBeenCalledTimes(1);
  resolve(null);
  await vi.waitFor(() => expect(state.updateInfo).toBe(null));
  // Drain the pending check's finally before the next connectivity event.
  await Promise.resolve();
  await Promise.resolve();
  browser.dispatchEvent(new Event('online'));
  expect(invoke).toHaveBeenCalledTimes(2);
});
it('does not publish an update after disposal or a channel change', async () => {
  let resolve!: (value: unknown) => void;
  invoke.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  stop = startAutomaticUpdateChecks();
  stop();
  resolve({ version: '0.21.0' });
  await Promise.resolve();
  expect(state.updateInfo).toBe(null);
  await Promise.resolve();
  invoke.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  stop = startAutomaticUpdateChecks();
  state.betaUpdatesEnabled = true;
  resolve({ version: '0.21.0' });
  await Promise.resolve();
  expect(state.updateInfo).toBe(null);
});
it('does not publish a fetched update if installation starts during settings reads', async () => {
  const previousUpdate = { version: '0.20.0' };
  state.updateInfo = previousUpdate;
  let resolveSettingReads!: (value: string | null) => void;
  const settingReads = new Promise<string | null>((resolve) => { resolveSettingReads = resolve; });
  invoke.mockImplementation(async (command: string) => {
    if (command === 'check_for_update') return { version: '0.21.0' };
    if (command === 'get_setting') return settingReads;
    return null;
  });

  stop = startAutomaticUpdateChecks();
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(3));
  state.updateInstalling = true;
  resolveSettingReads(null);
  await new Promise((resolve) => setTimeout(resolve, 0));

  expect(state.updateInfo).toBe(previousUpdate);
});
it('does not check while installing or awaiting restart', () => {
  state.updateInstalling = true;
  stop = startAutomaticUpdateChecks();
  browser.dispatchEvent(new Event('online'));
  expect(invoke).not.toHaveBeenCalled();
  state.updateInstalling = false;
  state.updateInstalled = true;
  browser.dispatchEvent(new Event('online'));
  expect(invoke).not.toHaveBeenCalled();
});
