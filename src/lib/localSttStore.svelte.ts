import {
  invoke,
  listen,
  emit,
  type LocalSttDownloadProgressPayload,
  type LocalSttExtractionProgressPayload,
  type LocalSttModelEventPayload,
  type LocalSttModelInfo,
  type LocalSttVerificationProgressPayload,
  type LocalTranscriptionState,
} from './tauri';
import { ensureNotificationPermission } from './notifications';
import { formatIpcError } from './errors';

export const localSttStore = $state({
  models: [] as LocalSttModelInfo[],
  state: {
    current_model_id: null,
    is_loaded: false,
    is_loading: false,
    is_downloading: false,
    downloading_model_id: null,
  } as LocalTranscriptionState,
  downloadProgress: {} as Record<string, LocalSttDownloadProgressPayload | undefined>,
  downloadStage: {} as Record<string, 'downloading' | 'verifying' | 'extracting' | undefined>,
});

export async function refreshLocalModels() {
  try {
    localSttStore.models = await invoke<LocalSttModelInfo[]>('list_local_stt_models');
  } catch (err) {
    console.error('load local models failed', err);
  }
}

export async function refreshLocalState() {
  try {
    localSttStore.state = await invoke<LocalTranscriptionState>('get_local_transcription_state');
  } catch (err) {
    console.error('load local transcription state failed', err);
  }
}

export async function downloadLocalModel(modelIdValue: string): Promise<boolean> {
  try {
    ensureNotificationPermission().catch(() => {});
    await invoke('download_local_stt_model', { modelId: modelIdValue });
    await refreshLocalModels();
    await refreshLocalState();
    return true;
  } catch (err) {
    console.error('download local model failed', err);
    emit('verenu:error', formatIpcError(err, 'Could not start the transcription model download'));
    return false;
  }
}

export async function cancelLocalModelDownload(modelIdValue: string) {
  try {
    await invoke('cancel_local_stt_model_download', { modelId: modelIdValue });
    await refreshLocalModels();
    await refreshLocalState();
    delete localSttStore.downloadProgress[modelIdValue];
    localSttStore.downloadProgress = { ...localSttStore.downloadProgress };
    delete localSttStore.downloadStage[modelIdValue];
    localSttStore.downloadStage = { ...localSttStore.downloadStage };
  } catch (err) {
    console.error('cancel local model download failed', err);
    void emit('verenu:error', formatIpcError(err, 'Could not cancel the transcription model download'));
  }
}

export async function deleteLocalModel(modelIdValue: string) {
  try {
    await invoke('delete_local_stt_model', { modelId: modelIdValue });
    await refreshLocalModels();
    await refreshLocalState();
    delete localSttStore.downloadProgress[modelIdValue];
    localSttStore.downloadProgress = { ...localSttStore.downloadProgress };
    delete localSttStore.downloadStage[modelIdValue];
    localSttStore.downloadStage = { ...localSttStore.downloadStage };
  } catch (err) {
    console.error('delete local model failed', err);
    void emit('verenu:error', formatIpcError(err, 'Could not delete the transcription model'));
  }
}

export async function openLocalModelsFolder() {
  try {
    await invoke('open_local_stt_models_folder');
  } catch (err) {
    console.error('open local models folder failed', err);
    void emit('verenu:error', formatIpcError(err, 'Could not open the models folder'));
  }
}

let listenersStarted = false;
let currentSession = 0;

export function startLocalSttListeners(): () => void {
  if (listenersStarted) {
    return () => {};
  }
  listenersStarted = true;
  const session = ++currentSession;
  const unlisteners: Array<() => void> = [];

  function registerUnlisten(unlisten: () => void) {
    if (session !== currentSession) {
      unlisten();
    } else {
      unlisteners.push(unlisten);
    }
  }

  (async () => {
    registerUnlisten(
      await listen<LocalSttDownloadProgressPayload>('local-stt-model-download-progress', (event) => {
        if (!event.payload?.model_id) return;
        localSttStore.downloadProgress = {
          ...localSttStore.downloadProgress,
          [event.payload.model_id]: event.payload,
        };
        localSttStore.downloadStage = {
          ...localSttStore.downloadStage,
          [event.payload.model_id]: 'downloading',
        };
      }),
    );
    for (const [eventName, stage] of [
      ['local-stt-model-verification-started', 'verifying'],
      ['local-stt-model-extraction-started', 'extracting'],
    ] as const) {
      registerUnlisten(
        await listen<LocalSttModelEventPayload>(eventName, (event) => {
          if (!event.payload?.model_id) return;
          localSttStore.downloadStage = {
            ...localSttStore.downloadStage,
            [event.payload.model_id]: stage,
          };
        }),
      );
    }
    registerUnlisten(
      await listen<LocalSttVerificationProgressPayload>('local-stt-model-verification-progress', (event) => {
        if (!event.payload?.model_id) return;
        localSttStore.downloadProgress = {
          ...localSttStore.downloadProgress,
          [event.payload.model_id]: {
            model_id: event.payload.model_id,
            downloaded_bytes: 0,
            total_bytes: 1,
            progress: event.payload.progress,
          },
        };
        localSttStore.downloadStage = {
          ...localSttStore.downloadStage,
          [event.payload.model_id]: 'verifying',
        };
      }),
    );
    registerUnlisten(
      await listen<LocalSttExtractionProgressPayload>('local-stt-model-extraction-progress', (event) => {
        if (!event.payload?.model_id) return;
        localSttStore.downloadProgress = {
          ...localSttStore.downloadProgress,
          [event.payload.model_id]: {
            model_id: event.payload.model_id,
            downloaded_bytes: 0,
            total_bytes: 1,
            progress: event.payload.progress,
          },
        };
        localSttStore.downloadStage = {
          ...localSttStore.downloadStage,
          [event.payload.model_id]: 'extracting',
        };
      }),
    );
    for (const eventName of [
      'local-stt-model-download-complete',
      'local-stt-model-download-failed',
      'local-stt-model-deleted',
    ]) {
      registerUnlisten(
        await listen<LocalSttModelEventPayload>(eventName, async (event) => {
          await Promise.all([refreshLocalModels(), refreshLocalState()]);
          if (event.payload?.model_id) {
            delete localSttStore.downloadProgress[event.payload.model_id];
            localSttStore.downloadProgress = { ...localSttStore.downloadProgress };
            delete localSttStore.downloadStage[event.payload.model_id];
            localSttStore.downloadStage = { ...localSttStore.downloadStage };
          }
        }),
      );
    }
    registerUnlisten(
      await listen<Record<string, unknown>>('local-stt-model-state', async () => {
        await refreshLocalState();
      }),
    );
  })().catch((err) => {
    console.error('local STT listeners failed', err);
    if (session === currentSession) {
      listenersStarted = false;
    }
    for (const unlisten of unlisteners) unlisten();
    unlisteners.length = 0;
  });

  return () => {
    if (session === currentSession) {
      listenersStarted = false;
    }
    currentSession += 1;
    for (const unlisten of unlisteners) unlisten();
    unlisteners.length = 0;
  };
}
