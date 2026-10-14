import type { LocalLlmModelInfo, LocalLlmRuntimeInfo, LocalLlmState, LocalSttModelInfo, LocalTranscriptionState } from '../tauri';
import type { PresetTarget, RequiredLocalModel } from '../components/settings/modelPresets';
import type { CleanupIntensity } from '../settings';

export function setupDefaultModels(provider: string) {
  return provider === 'local'
    ? { transcriptionDefaultModel: 'local/parakeet-v3', cleanupDefaultModel: 'local/qwen2.5-3b-instruct' }
    : provider === 'openai'
      ? { transcriptionDefaultModel: 'openai/gpt-4o-transcribe', cleanupDefaultModel: 'openai/gpt-4o-mini' }
      : provider === 'google'
        ? { transcriptionDefaultModel: 'google/gemini-3.5-transcribe', cleanupDefaultModel: 'google/gemini-3.5-flash-lite' }
        : { transcriptionDefaultModel: 'groq/whisper-large-v3-turbo', cleanupDefaultModel: 'groq/qwen/qwen3.8-27b' };
}

export type SetupModelInventory = {
  speechModels: Pick<LocalSttModelInfo, 'id' | 'is_downloaded' | 'is_downloading'>[];
  cleanupModels: Pick<LocalLlmModelInfo, 'id' | 'is_downloaded' | 'is_downloading'>[];
  transcriptionState: Pick<LocalTranscriptionState, 'is_downloading' | 'downloading_model_id'>;
  cleanupState: Pick<LocalLlmState, 'is_downloading' | 'downloading_model_id'>;
  cleanupRuntime: Pick<LocalLlmRuntimeInfo, 'installed' | 'is_downloading'>;
};

export type SetupModelReadiness = {
  ready: boolean;
  pending: boolean;
  message: string;
};

function modelInstalled(model: RequiredLocalModel, inventory: SetupModelInventory): boolean {
  const models = model.task === 'transcription' ? inventory.speechModels : inventory.cleanupModels;
  return models.some(candidate => candidate.id === model.id && candidate.is_downloaded);
}

function modelDownloading(model: RequiredLocalModel, inventory: SetupModelInventory): boolean {
  const models = model.task === 'transcription' ? inventory.speechModels : inventory.cleanupModels;
  const state = model.task === 'transcription' ? inventory.transcriptionState : inventory.cleanupState;
  return models.some(candidate => candidate.id === model.id && candidate.is_downloading)
    || (state.is_downloading && state.downloading_model_id === model.id);
}

function joined(items: string[]): string {
  if (items.length < 2) return items[0] ?? '';
  if (items.length === 2) return `${items[0]} and ${items[1]}`;
  return `${items.slice(0, -1).join(', ')}, and ${items[items.length - 1]}`;
}

export function setupCleanupEnabled(
  intensity: CleanupIntensity,
  target: PresetTarget | null | undefined,
): boolean {
  if (intensity === 'none') return false;
  // Basic is on-device rule cleanup with no cleanup model, so a transcription-only
  // preset must not switch it off. The pipeline gates Basic and voice commands on cleanup_enabled.
  if (intensity === 'rules') return true;
  return target ? target.cleanupEnabled : true;
}

export function setupModelReadiness(
  target: PresetTarget | null | undefined,
  inventory: SetupModelInventory,
  cleanupEnabled = target?.cleanupEnabled === true,
  defaults?: ReturnType<typeof setupDefaultModels>,
): SetupModelReadiness {
  const defaultRequirements: RequiredLocalModel[] = [];
  for (const [task, model] of [['transcription', defaults?.transcriptionDefaultModel], ['cleanup', defaults?.cleanupDefaultModel]] as const) {
    if (model?.startsWith('local/')) defaultRequirements.push({ task, id: model.slice(6), sizeMb: 0 });
  }
  const required = (target?.requiredLocalModels ?? defaultRequirements)
    .filter(model => model.task !== 'cleanup' || cleanupEnabled);
  const missingModels = required.filter(model => !modelInstalled(model, inventory));
  const cleanupEngineMissing = cleanupEnabled
    && required.some(model => model.task === 'cleanup')
    && !inventory.cleanupRuntime.installed;
  if (missingModels.length === 0 && !cleanupEngineMissing) return { ready: true, pending: false, message: '' };

  const pendingModels = missingModels.filter(model => modelDownloading(model, inventory));
  const unavailableModels = missingModels.filter(model => !modelDownloading(model, inventory));
  const cleanupEnginePending = cleanupEngineMissing && inventory.cleanupRuntime.is_downloading;
  const pendingLabels = [
    ...(pendingModels.some(model => model.task === 'transcription') ? ['speech model'] : []),
    ...(pendingModels.some(model => model.task === 'cleanup') ? ['cleanup model'] : []),
    ...(cleanupEnginePending ? ['cleanup engine'] : []),
  ];
  const unavailableLabels = [
    ...(unavailableModels.some(model => model.task === 'transcription') ? ['speech model'] : []),
    ...(unavailableModels.some(model => model.task === 'cleanup') ? ['cleanup model'] : []),
    ...(cleanupEngineMissing && !cleanupEnginePending ? ['cleanup engine'] : []),
  ];
  const pending = pendingLabels.length > 0;
  const message = unavailableLabels.length === 0
    ? `Your selected on-device ${joined(pendingLabels)} ${pendingLabels.length === 1 ? 'is' : 'are'} still downloading. Dictation will work when ${pendingLabels.length === 1 ? 'it finishes' : 'they finish'}.`
    : pendingLabels.length === 0
      ? `Your selected on-device ${joined(unavailableLabels)} ${unavailableLabels.length === 1 ? "isn't" : "aren't"} installed yet.`
      : `Your selected on-device ${joined(pendingLabels)} ${pendingLabels.length === 1 ? 'is' : 'are'} still downloading, and ${joined(unavailableLabels)} ${unavailableLabels.length === 1 ? "isn't" : "aren't"} installed yet. Finish the download and install any remaining requirements before dictating.`;
  return { ready: false, pending, message };
}
