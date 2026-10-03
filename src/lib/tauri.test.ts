import { afterEach, expect, it, vi } from 'vitest';

const transport = vi.hoisted(() => ({
  nativeInvoke: vi.fn(async () => 'native'),
  nativeListen: vi.fn(async () => () => {}),
  nativeEmit: vi.fn(async () => {}),
  session: false,
  sessionInvoke: vi.fn(async () => 'session'),
  sessionListen: vi.fn(async () => () => {}),
  sessionEmit: vi.fn(async () => {}),
  loadMocks: vi.fn(),
  devInvoke: vi.fn(async () => 'mock'),
  devListen: vi.fn(async () => () => {}),
  devEmit: vi.fn(async () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: transport.nativeInvoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: transport.nativeListen, emit: transport.nativeEmit }));
vi.mock('./devSession', () => ({
  isBrowserDevSession: () => transport.session,
  sessionInvoke: transport.sessionInvoke,
  sessionListen: transport.sessionListen,
  sessionEmit: transport.sessionEmit,
}));
vi.mock('./tauri.dev', () => {
  transport.loadMocks();
  return { devInvoke: transport.devInvoke, devListen: transport.devListen, devEmit: transport.devEmit };
});

afterEach(() => { vi.unstubAllGlobals(); transport.session = false; });

it('routes native and live-session calls without loading browser mock state', async () => {
  const api = await import('./tauri');
  vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: () => {} } });
  expect(await api.invoke('get_setting', { key: 'test' })).toBe('native');
  await api.listen('test', () => {});
  await api.emit('test', 'payload');
  expect(transport.nativeInvoke).toHaveBeenCalledWith('get_setting', { key: 'test' });
  expect(transport.nativeListen).toHaveBeenCalledTimes(1);
  expect(transport.nativeEmit).toHaveBeenCalledWith('test', 'payload');
  vi.stubGlobal('window', {});
  transport.session = true;
  expect(await api.invoke('get_setting')).toBe('session');
  await api.listen('test', () => {});
  await api.emit('test', 'payload');
  expect(transport.sessionListen).toHaveBeenCalledTimes(1);
  expect(transport.sessionEmit).toHaveBeenCalledWith('test', 'payload');
  expect(transport.loadMocks).not.toHaveBeenCalled();
});

it('loads browser mock transport on demand for calls, listeners, and events', async () => {
  const api = await import('./tauri');
  vi.stubGlobal('window', {});
  expect(await api.invoke('get_setting')).toBe('mock');
  const saved = api.invoke('save_setting', { key: 'history_retention', value: 'Forever' });
  // Settings saves must start before a newly mounted view reads them back.
  expect(transport.devInvoke).toHaveBeenCalledWith('save_setting', { key: 'history_retention', value: 'Forever' });
  await saved;
  await api.listen('test', () => {});
  await api.emit('test', 'payload');
  expect(transport.loadMocks).toHaveBeenCalledTimes(1);
  expect(transport.devListen).toHaveBeenCalledTimes(1);
  expect(transport.devEmit).toHaveBeenCalledWith('test', 'payload');
});
