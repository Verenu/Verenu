import { invoke } from './tauri';

export const transcriptionModelStore = $state({ defaultModel: 'groq/whisper-large-v3-turbo' });

export async function refreshTranscriptionModel() {
  try {
    const value = await invoke<string | null>('get_setting', { key: 'transcription_default_model' });
    if (value) transcriptionModelStore.defaultModel = value;
  } catch (err) {
    console.error('refresh transcription model failed', err);
  }
}
