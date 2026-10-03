export type CommandArgs = Record<string, unknown>;
export type EventEnvelope<T> = {
  event: string;
  id: number;
  payload: T;
};
export type EventHandler<T> = (event: EventEnvelope<T>) => void;
export type UnlistenFn = () => void;
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
