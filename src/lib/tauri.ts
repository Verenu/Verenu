import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { emit as tauriEmit, listen as tauriListen } from '@tauri-apps/api/event';
import { frontendIpcActivity } from './diagnostics';
import { extractIpcErrorMessage } from './errors';
import { isBrowserDevSession, sessionEmit, sessionInvoke, sessionListen } from './devSession';

import type { CommandArgs, EventHandler, UnlistenFn } from './tauri.types';
export type * from './tauri.types';

declare const __APP_VERSION__: string;

type MockRuntime = typeof import('./tauri.dev');
let mockRuntime: MockRuntime | undefined;
let mockLoading: Promise<MockRuntime> | undefined;

function loadMockRuntime(): Promise<MockRuntime> {
  return mockLoading ??= import('./tauri.dev').then((runtime) => {
    mockRuntime = runtime;
    return runtime;
  }).catch((error) => {
    mockLoading = undefined;
    throw error;
  });
}

function hasTauriInternals(): boolean {
  if (typeof window === 'undefined') return false;
  const maybeWindow = window as Window & {
    __TAURI_INTERNALS__?: { invoke?: unknown };
  };
  return typeof maybeWindow.__TAURI_INTERNALS__?.invoke === 'function';
}

export function isTauriRuntime(): boolean {
  return hasTauriInternals();
}

export function invoke<T = unknown>(command: string, args?: CommandArgs): Promise<T> {
  const started = frontendIpcActivity.start(command);
  let request: Promise<T>;
  if (hasTauriInternals()) request = tauriInvoke<T>(command, args);
  else if (isBrowserDevSession()) request = sessionInvoke<T>(command, args);
  else if (mockRuntime) request = mockRuntime.devInvoke<T>(command, args);
  else request = loadMockRuntime().then((mock) => mock.devInvoke<T>(command, args));
  return request.then(
    (value) => { frontendIpcActivity.finish(command, started, true); return value; },
    (error) => { frontendIpcActivity.finish(command, started, false, extractIpcErrorMessage(error)); throw error; },
  );
}

export function listen<T>(
  event: string,
  handler: EventHandler<T>,
): Promise<UnlistenFn> {
  if (isBrowserDevSession()) return sessionListen<T>(event, handler);
  if (hasTauriInternals()) {
    return tauriListen<T>(event, handler as Parameters<typeof tauriListen<T>>[1]);
  }
  if (mockRuntime) return mockRuntime.devListen<T>(event, handler);
  return loadMockRuntime().then((mock) => mock.devListen<T>(event, handler));
}

export function emit<T>(event: string, payload?: T): Promise<void> {
  if (isBrowserDevSession()) return sessionEmit(event, payload);
  if (hasTauriInternals()) {
    return tauriEmit(event, payload);
  }
  if (mockRuntime) return mockRuntime.devEmit(event, payload);
  return loadMockRuntime().then((mock) => mock.devEmit(event, payload));
}

export function getVersion(): Promise<string> {
  // Vite embeds the package version into the frontend bundle after the
  // release workflow applies its temporary nightly version bump. Using that
  // same value here keeps the About screen aligned with the updater and the
  // packaged application instead of relying on a second runtime metadata path.
  return Promise.resolve(__APP_VERSION__);
}
