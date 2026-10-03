import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { emit as tauriEmit, listen as tauriListen } from '@tauri-apps/api/event';
import { frontendIpcActivity } from './diagnostics';
import { extractIpcErrorMessage } from './errors';
import { isBrowserDevSession, sessionEmit, sessionInvoke, sessionListen } from './devSession';

declare const __APP_VERSION__: string;
declare const __VERENU_GIT_SHA__: string;
declare const __VERENU_GIT_BRANCH__: string;
declare const __VERENU_GIT_DIRTY__: boolean;
declare const __VERENU_BUILD_TIME__: string;

type CommandArgs = Record<string, unknown>;
type EventEnvelope<T> = {
  event: string;
  id: number;
  payload: T;
};
type EventHandler<T> = (event: EventEnvelope<T>) => void;
type UnlistenFn = () => void;
type LocalSttEngineType =
  | 'parakeet'
  | 'moonshine'
  | 'moonshine_streaming'
  | 'sense_voice'
  | 'giga_am'
  | 'canary'
  | 'cohere';
export type LocalSttModelInfo = {
  id: string;
  name: string;
  description: string;
  filename: string;
  url: string | null;
  sha256: string | null;
  size_mb: number;
  is_directory: boolean;
  is_downloaded: boolean;
  is_downloading: boolean;
  partial_size: number;
  engine_type: LocalSttEngineType;
  speed_score: number;
  accuracy_score: number;
  privacy_label: string;
  supported_languages: string[];
  supports_language_selection: boolean;
  supports_translation: boolean;
  is_recommended: boolean;
};
export type LocalTranscriptionState = {
  current_model_id: string | null;
  is_loaded: boolean;
  is_loading: boolean;
  is_downloading: boolean;
  downloading_model_id: string | null;
};
type LocalLlmPromptFamily =
  | 'gemma4'
  | 'qwen25'
  | 'phi3'
  | 'smollm2'
  | 'granite33';
export type LocalLlmModelInfo = {
  id: string;
  name: string;
  description: string;
  repo_id: string;
  size_mb: number;
  quantization: string;
  privacy_label: string;
  is_downloaded: boolean;
  is_downloading: boolean;
  partial_size: number;
  is_recommended: boolean;
  prompt_family: LocalLlmPromptFamily;
};
export type LocalLlmState = {
  current_model_id: string | null;
  is_loaded: boolean;
  is_loading: boolean;
  is_downloading: boolean;
  downloading_model_id: string | null;
  endpoint: string | null;
};
export type LocalSttDownloadProgressPayload = {
  model_id: string;
  downloaded_bytes: number;
  total_bytes: number | null;
  progress: number;
};
export type LocalSttModelEventPayload = {
  model_id: string;
  error: string | null;
};
export type LocalSttExtractionProgressPayload = {
  model_id: string;
  progress: number;
};
export type LocalSttVerificationProgressPayload = {
  model_id: string;
  progress: number;
};
export type LocalLlmDownloadProgressPayload = {
  model_id: string;
  downloaded_bytes: number;
  total_bytes: number | null;
  progress: number;
};
export type LocalLlmModelEventPayload = {
  model_id: string;
  error: string | null;
};
export type LocalLlmVerificationProgressPayload = {
  model_id: string;
  progress: number;
};
type LlamaBackend = 'cuda' | 'vulkan' | 'metal' | 'cpu';
export type LocalLlmRuntimeInfo = {
  installed: boolean;
  is_downloading: boolean;
  backend: LlamaBackend;
  approx_download_mb: number;
};
export type LocalLlmRuntimeDownloadProgressPayload = {
  downloaded_bytes: number;
  total_bytes: number | null;
  progress: number;
  stage: 'downloading' | 'extracting';
};
export type LocalLlmRuntimeEventPayload = {
  error: string | null;
};

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

type DevTransport = typeof import('./tauri.dev');
let devTransport: DevTransport | undefined;
let devTransportLoading: Promise<DevTransport> | undefined;

function loadDevTransport(): Promise<DevTransport> {
  return devTransportLoading ??= import('./tauri.dev').then((transport) => {
    devTransport = transport;
    return transport;
  }).catch((error) => {
    devTransportLoading = undefined;
    throw error;
  });
}

export function invoke<T = unknown>(command: string, args?: CommandArgs): Promise<T> {
  const started = frontendIpcActivity.start(command);
  let request: Promise<T>;
  if (hasTauriInternals()) request = tauriInvoke<T>(command, args);
  else if (isBrowserDevSession()) request = sessionInvoke<T>(command, args);
  else request = devTransport
    ? devTransport.devInvoke<T>(command, args)
    : loadDevTransport().then(({ devInvoke }) => devInvoke<T>(command, args));
  return request.then(
    (value) => { frontendIpcActivity.finish(command, started, true); return value; },
    (error) => { frontendIpcActivity.finish(command, started, false, extractIpcErrorMessage(error)); throw error; },
  );
}

export function listen<T>(event: string, handler: EventHandler<T>): Promise<UnlistenFn> {
  if (isBrowserDevSession()) return sessionListen<T>(event, handler);
  if (hasTauriInternals()) return tauriListen<T>(event, handler as Parameters<typeof tauriListen<T>>[1]);
  return devTransport
    ? devTransport.devListen<T>(event, handler)
    : loadDevTransport().then(({ devListen }) => devListen<T>(event, handler));
}

export function emit<T>(event: string, payload?: T): Promise<void> {
  if (isBrowserDevSession()) return sessionEmit(event, payload);
  if (hasTauriInternals()) return tauriEmit(event, payload);
  return devTransport
    ? devTransport.devEmit(event, payload)
    : loadDevTransport().then(({ devEmit }) => devEmit(event, payload));
}

export function getVersion(): Promise<string> {
  // Vite embeds the package version into the frontend bundle after the
  // release workflow applies its temporary nightly version bump. Using that
  // same value here keeps the About screen aligned with the updater and the
  // packaged application instead of relying on a second runtime metadata path.
  return Promise.resolve(__APP_VERSION__);
}
