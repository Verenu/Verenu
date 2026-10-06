import { downloadLocalModel, cancelLocalModelDownload, deleteLocalModel } from '../../localSttStore.svelte';
import { downloadLocalLlmModel, cancelLocalLlmModelDownload, deleteLocalLlmModel } from '../../localLlmStore.svelte';

/** Both preset pickers use the same task-to-manager dispatch. */
export const localModelDownloads = {
  transcription: { download: downloadLocalModel, cancel: cancelLocalModelDownload, delete: deleteLocalModel },
  cleanup: { download: downloadLocalLlmModel, cancel: cancelLocalLlmModelDownload, delete: deleteLocalLlmModel },
};
