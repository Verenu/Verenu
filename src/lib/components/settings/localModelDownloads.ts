import { downloadLocalModel, cancelLocalModelDownload, deleteLocalModel } from '../../localSttStore.svelte';
import { downloadLocalLlmModel, cancelLocalLlmModelDownload, deleteLocalLlmModel } from '../../localLlmStore.svelte';

export const localModelDownloads = {
  transcription: { download: downloadLocalModel, cancel: cancelLocalModelDownload, delete: deleteLocalModel },
  cleanup: { download: downloadLocalLlmModel, cancel: cancelLocalLlmModelDownload, delete: deleteLocalLlmModel },
};
