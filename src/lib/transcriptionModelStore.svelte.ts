import { invoke } from './tauri';

export const transcriptionModelStore = $state({ defaultModel: 'groq/whisper-large-v3-turbo' });

let refreshVersion = 0;
export async function refreshTranscriptionModel() {
  const version = ++refreshVersion;
  try {
    const value = await invoke<string>('get_effective_transcription_model');
    if (typeof value === 'string' && value && version === refreshVersion) transcriptionModelStore.defaultModel = value;
  } catch (err) {
    console.error('refresh transcription model failed', err);
  }
}
