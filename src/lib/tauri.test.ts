import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const transports = vi.hoisted(() => ({
  nativeInvoke: vi.fn(), nativeListen: vi.fn(), nativeEmit: vi.fn(),
  sessionInvoke: vi.fn(), sessionListen: vi.fn(), sessionEmit: vi.fn(),
  mockInvoke: vi.fn(), mockListen: vi.fn(), mockEmit: vi.fn(),
  session: false,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: transports.nativeInvoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: transports.nativeListen, emit: transports.nativeEmit }));
vi.mock('./devSession', () => ({
  isBrowserDevSession: () => transports.session,
  sessionInvoke: transports.sessionInvoke,
  sessionListen: transports.sessionListen,
  sessionEmit: transports.sessionEmit,
}));
vi.mock('./tauri.dev', () => ({
  devInvoke: transports.mockInvoke, devListen: transports.mockListen, devEmit: transports.mockEmit,
}));

import { emit, invoke, isTauriRuntime, listen } from './tauri';
import { frontendIpcActivity } from './diagnostics';

beforeEach(() => {
  vi.resetAllMocks();
  transports.session = false;
  vi.stubGlobal('window', {});
});
afterEach(() => vi.unstubAllGlobals());

describe('IPC transport routing', () => {
  for (const mode of ['native', 'session', 'mock'] as const) {
    it(`routes commands and events through ${mode}`, async () => {
      if (mode === 'native') vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: () => {} } });
      transports.session = mode === 'session';
      const command = transports[`${mode}Invoke`];
      const subscribe = transports[`${mode}Listen`];
      const publish = transports[`${mode}Emit`];
      const unlisten = vi.fn();
      command.mockResolvedValue({ id: 42 });
      subscribe.mockResolvedValue(unlisten);
      publish.mockResolvedValue(undefined);
      const handler = vi.fn();

      expect(isTauriRuntime()).toBe(mode === 'native');
      await expect(invoke('get_context', { id: 42 })).resolves.toEqual({ id: 42 });
      expect(command).toHaveBeenCalledWith('get_context', { id: 42 });
      expect(await listen('changed', handler)).toBe(unlisten);
      expect(subscribe).toHaveBeenCalledWith('changed', handler);
      await emit('changed', { id: 42 });
      expect(publish).toHaveBeenCalledWith('changed', { id: 42 });
      for (const other of ['native', 'session', 'mock'] as const) {
        if (other === mode) continue;
        expect(transports[`${other}Invoke`]).not.toHaveBeenCalled();
        expect(transports[`${other}Listen`]).not.toHaveBeenCalled();
        expect(transports[`${other}Emit`]).not.toHaveBeenCalled();
      }
    });
  }

  it('propagates real-session failures without falling back to mocks', async () => {
    transports.session = true;
    const failure = new Error('Open the private session access link to connect');
    transports.sessionInvoke.mockRejectedValue(failure);
    await expect(invoke('session-routing-failure')).rejects.toBe(failure);
    expect(transports.mockInvoke).not.toHaveBeenCalled();
    const metric = frontendIpcActivity.snapshot().find((row) => row.command === 'session-routing-failure');
    expect(metric?.failures).toBe(1);
    expect(metric?.currently_running).toBe(0);
  });

  it('starts subsequent mock writes immediately after loading the runtime', async () => {
    transports.mockInvoke.mockResolvedValue(undefined);
    await invoke('get_all_settings');
    transports.mockInvoke.mockClear();
    const saved = invoke('save_setting', { key: 'default_tone', value: 'formal' });
    expect(transports.mockInvoke).toHaveBeenCalledWith('save_setting', { key: 'default_tone', value: 'formal' });
    await saved;
  });
});
