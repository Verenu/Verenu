<script lang="ts">
  import { refreshAppleIntelligence } from '../../appleIntelligence.svelte';
  import { onMount, tick } from 'svelte';
  import { customProviderStore } from '../../customProviders.svelte';
  import { fly, slide } from 'svelte/transition';
  import { cubicOut, expoOut } from 'svelte/easing';
  import { invoke } from '../../tauri';
  import {
    localSttStore,
    refreshLocalModels,
    refreshLocalState,
    downloadLocalModel,
    cancelLocalModelDownload,
    deleteLocalModel,
    openLocalModelsFolder,
  } from '../../localSttStore.svelte';
  import {
    localLlmStore,
    refreshLocalLlmModels,
    refreshLocalLlmState,
    refreshLocalLlmRuntimeInfo,
    downloadLocalLlmModel,
    downloadLocalLlmRuntime,
    cancelLocalLlmRuntimeDownload,
    deleteLocalLlmRuntime,
    cancelLocalLlmModelDownload,
    deleteLocalLlmModel,
  } from '../../localLlmStore.svelte';
  import { animateWidth, MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import Toggle from '../Toggle.svelte';
  import Dropdown from '../Dropdown.svelte';
  import { appStore, cleanupPromptStore } from '../../stores.svelte';
  import {
    loadSettingsSnapshot,
    saveSetting,
    type LocalModelMemoryPolicy,
    type ProviderId,
    type ProviderModelMap,
    type SettingsSnapshot,
  } from '../../settings';
  import ModelTaskTile from './ModelTaskTile.svelte';
  import { localModelDownloads } from './localModelDownloads';
  import ModelPresetPicker from './ModelPresetPicker.svelte';
  import { settingsSearchNavigation } from '../../settingsSearch.svelte';
  import {
    getHardware,
    type ActiveConfig,
    type Hardware,
    type Preset,
    type PresetTarget,
    type RequiredLocalModel,
    type ModelPerformance,
    supportsLocalLanguage,
  } from './modelPresets';
  import { refreshTranscriptionModel } from '../../transcriptionModelStore.svelte';
  import {
    hydrateCatalogCache,
    modelCatalogStore,
    refreshCatalog,
    refreshStaleCatalogs,
    trackedIds,
  } from '../../modelCatalogStore.svelte';
  import { type LocalControls, type PickerContext } from './modelStates';
  import {
    CATALOG,
    emptyProviderModelMap,
    mergeProviderModelMap,
    modelId,
    recommendedModels,
    migrateDeprecatedGroqCleanupModel,
    migrateDeprecatedGoogleModel,
    splitModelId,
    type TaskType,
    type UiProviderId,
  } from './models';

  let apiKeyStatus = $state<Record<ProviderId, boolean>>({
    groq: false,
    openai: false,
    google: false,
    assemblyai: false,
    openrouter: false,
    xai: false,
    local: false,
    'apple-intelligence': false,
  });

  let transcriptionModelsByProvider = $state<ProviderModelMap>(emptyProviderModelMap());
  let cleanupModelsByProvider = $state<ProviderModelMap>(emptyProviderModelMap());

  let transcriptionDefaultModel = $state('groq/whisper-large-v3-turbo');
  let cleanupDefaultModel = $state('groq/qwen/qwen3.8-27b');
  let transcriptionFallbackModels = $state<string[]>([]);
  let dualTranscriptionEnabled = $state(false);
  let cleanupFallbackModels = $state<string[]>([]);
  let cleanupEnabled = $state(true);
  let modelSelectionMode = $state<'manual' | 'fastest' | 'balanced' | 'quality'>('manual');
  let transcriptionLanguage = $state('en');
  let performance = $state<ModelPerformance[]>([]);
  let testingLocal = $state(false);
  let localTestResult = $state('');

  function useOfflineFallback(preset: Preset) {
    const target = preset.target;
    if (!target) return;
    applyPreset({ ...preset, id: `fallback-${preset.id}`, offline: false, target: {
      ...target,
      transcriptionDefaultModel,
      cleanupEnabled,
      cleanupDefaultModel,
      dualTranscription: dualTranscriptionEnabled,
      transcriptionFallbacks: [...transcriptionFallbackModels.filter(id => !id.startsWith('local/')), target.transcriptionDefaultModel, ...target.transcriptionFallbacks],
      cleanupFallbacks: [...cleanupFallbackModels.filter(id => !id.startsWith('local/')), ...(target.cleanupDefaultModel && cleanupDefaultModel !== target.cleanupDefaultModel ? [target.cleanupDefaultModel] : [])],
    } });
  }

  async function testLocalSpeed(preset: Preset) {
    if (!preset.target || testingLocal) return;
    testingLocal = true;
    localTestResult = '';
    try {
      const result = await invoke<{ initial_ms: number; warm_ms: number; cleanup_ms: number | null; initially_loaded: boolean }>('benchmark_local_models', {
        speechModel: preset.target.transcriptionDefaultModel.slice('local/'.length),
        cleanupModel: preset.target.cleanupDefaultModel?.slice('local/'.length) ?? null,
      });
      localTestResult = `${result.initially_loaded ? 'First run' : 'Load and first run'}: ${(result.initial_ms / 1000).toFixed(1)}s. Warm speech: ${(result.warm_ms / 1000).toFixed(1)}s.${result.cleanup_ms === null ? '' : ` Warm cleanup: ${(result.cleanup_ms / 1000).toFixed(1)}s.`} Uses bundled English sample audio; this measures speed, not accuracy.`;
      performance = await invoke<ModelPerformance[]>('get_model_performance');
    } catch {
      localTestResult = 'The local test could not finish. Check that the models and cleanup engine are installed and no dictation is running.';
    } finally { testingLocal = false; }
  }

  let hardware = $state<Hardware>({ totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: true });

  let pendingPreset = $state<Preset | null>(null);
  let pendingPresetDownloads = $state<RequiredLocalModel[]>([]);
  let pendingPresetRequestsSettled = $state(0);

  const activeConfig = $derived<ActiveConfig>({
    transcriptionDefaultModel,
    cleanupEnabled,
    cleanupDefaultModel,
    dualTranscription: dualTranscriptionEnabled,
    transcriptionFallbacks: transcriptionFallbackModels,
    cleanupFallbacks: cleanupFallbackModels,
  });

  const installedLocal = $derived({
    transcription: localSttStore.models.filter((model) => model.is_downloaded).map((model) => model.id),
    cleanup: localLlmStore.models.filter((model) => model.is_downloaded).map((model) => model.id),
  });
  const presetOptions = $derived({
    language: transcriptionLanguage,
    localModels: localSttStore.models,
    installedLocal,
    localCleanupReady: localLlmStore.runtime?.installed ?? false,
    cache: modelCatalogStore.cache,
    performance,
    customProviders: customProviderStore.providers,
  });

  function localControls(type: TaskType): LocalControls {
    if (type === 'transcription') {
      return {
        supported: localModelsSupported,
        models: localSttStore.models,
        downloadProgress: localSttStore.downloadProgress,
        downloadStage: localSttStore.downloadStage,
        onDownload: (id) => downloadLocalModel(id).catch((err) => console.error('download stt model failed', err)),
        onCancel: (id) => cancelLocalModelDownload(id).catch((err) => console.error('cancel stt download failed', err)),
        onDelete: (id) => handleDeleteTranscriptionModel(id).catch((err) => console.error('delete stt model failed', err)),
      };
    }
    return {
      supported: localModelsSupported,
      models: localLlmStore.models,
      downloadProgress: localLlmStore.downloadProgress,
      downloadStage: localLlmStore.downloadStage,
      onDownload: (id) => downloadLocalLlmModel(id).catch((err) => console.error('download llm model failed', err)),
      onCancel: (id) => cancelLocalLlmModelDownload(id).catch((err) => console.error('cancel llm download failed', err)),
      onDelete: (id) => handleDeleteCleanupModel(id).catch((err) => console.error('delete llm model failed', err)),
      runtime: {
        info: localLlmStore.runtime,
        progress: localLlmStore.runtimeDownloadProgress,
        onDownload: () => downloadLocalLlmRuntime().catch((err) => console.error('download runtime failed', err)),
        onCancel: () => cancelLocalLlmRuntimeDownload().catch((err) => console.error('cancel runtime download failed', err)),
        onDelete: () => deleteLocalLlmRuntime().catch((err) => console.error('delete runtime failed', err)),
      },
    };
  }

  function pickerContext(type: TaskType): PickerContext {
    return {
      task: type,
      apiKeyStatus,
      cache: modelCatalogStore.cache,
      localModels: type === 'transcription' ? localSttStore.models : localLlmStore.models,
      hardware,
    };
  }

  function trackedModelIds(): string[] {
    const selected = [
      transcriptionDefaultModel,
      ...transcriptionFallbackModels,
      cleanupDefaultModel,
      ...cleanupFallbackModels,
    ].filter(Boolean);
    return trackedIds(selected, CATALOG);
  }

  const downloadingLocal = $derived({
    transcription: localSttStore.state.downloading_model_id ?? null,
    cleanup: localLlmStore.state.downloading_model_id ?? null,
  });

  function requiredModelsInstalled(target: PresetTarget): boolean {
    return target.requiredLocalModels.every((model) =>
      (installedLocal[model.task]?.includes(model.id) ?? false)
      && (model.task !== 'cleanup' || localLlmStore.runtime?.installed === true),
    );
  }

  function clearPendingPreset() {
    pendingPreset = null;
    pendingPresetDownloads = [];
    pendingPresetRequestsSettled = 0;
  }

  function activatePreset(target: PresetTarget, presetId: string) {
    if (modelConflict) return;
    if (!presetId.startsWith('fallback-')) {
      modelSelectionMode = presetId.endsWith('-accurate') ? 'quality' : presetId.endsWith('-balanced') ? 'balanced' : 'fastest';
    }
    transcriptionDefaultModel = target.transcriptionDefaultModel;
    transcriptionFallbackModels = [...target.transcriptionFallbacks];
    dualTranscriptionEnabled = target.dualTranscription;
    if (target.cleanupEnabled && target.cleanupDefaultModel) {
      cleanupDefaultModel = target.cleanupDefaultModel;
    }
    cleanupFallbackModels = [...target.cleanupFallbacks];
    cleanupEnabled = target.cleanupEnabled;
    persistAll().catch((err) => console.error('persist preset failed', err));
  }

  function applyPreset(preset: Preset) {
    if (modelConflict) return;
    const target = preset.target;
    if (!target) return;
    clearPendingPreset();

    const missing = target.requiredLocalModels.filter(
      (model) => !installedLocal[model.task]?.includes(model.id)
        || (model.task === 'cleanup' && !localLlmStore.runtime.installed),
    );
    if (missing.length > 0) {
      pendingPreset = preset;
      pendingPresetDownloads = missing;
      pendingPresetRequestsSettled = 0;
      for (const task of ['transcription', 'cleanup'] as const) {
        void (async () => {
          for (const model of missing.filter(model => model.task === task)) {
            if (pendingPreset?.id !== preset.id) return;
            const started = isExpectedModelDownloading(model) || await localModelDownloads[task].download(model.id);
            if (!started) { clearPendingPreset(); return; }
            while (!installedLocal[task].includes(model.id) || (task === 'cleanup' && !localLlmStore.runtime.installed)) {
              if (pendingPreset?.id !== preset.id) return;
              if (!isExpectedModelDownloading(model)) {
                if (task === 'transcription') await refreshLocalModels();
                else await Promise.all([refreshLocalLlmModels(), refreshLocalLlmRuntimeInfo()]);
                if (!installedLocal[task].includes(model.id) || (task === 'cleanup' && !localLlmStore.runtime.installed)) { clearPendingPreset(); return; }
                break;
              }
              await new Promise(resolve => setTimeout(resolve, 200));
            }
            if (pendingPreset?.id === preset.id) pendingPresetRequestsSettled += 1;
          }
        })().catch(() => { if (pendingPreset?.id === preset.id) clearPendingPreset(); });
      }
      return;
    }

    activatePreset(target, preset.id);
  }

  function isExpectedModelDownloading(model: RequiredLocalModel): boolean {
    if (model.task === 'cleanup' && localLlmStore.runtime.is_downloading) return true;
    if (downloadingLocal[model.task] === model.id) return true;
    if (model.task === 'transcription') {
      return localSttStore.models.some((entry) => entry.id === model.id && entry.is_downloading);
    }
    return localLlmStore.models.some((entry) => entry.id === model.id && entry.is_downloading);
  }

  function openApiKeysSection() {
    appStore.settingsSection = 'keys';
  }

  function cancelPresetDownload(preset: Preset) {
    const target = preset.target;
    if (!target) return;
    if (pendingPreset?.id === preset.id || pendingPreset?.id === `fallback-${preset.id}`) clearPendingPreset();
    for (const model of target.requiredLocalModels) {
      if (!isExpectedModelDownloading(model)) continue;
      localModelDownloads[model.task].cancel(model.id).catch((err) =>
        console.error(model.task === 'transcription' ? 'cancel preset stt download failed' : 'cancel preset llm download failed', err));
    }
  }

  function deletePresetModels(preset: Preset) {
    const target = preset.target;
    if (!target) return;
    for (const model of target.requiredLocalModels) {
      if (!installedLocal[model.task]?.includes(model.id)) continue;
      if (model.task === 'transcription') {
        handleDeleteTranscriptionModel(model.id).catch((err) => console.error('delete preset stt model failed', err));
      } else {
        handleDeleteCleanupModel(model.id).catch((err) => console.error('delete preset llm model failed', err));
      }
    }
  }

  $effect(() => {
    const preset = pendingPreset;
    if (!preset?.target) return;
    if (requiredModelsInstalled(preset.target)) {
      activatePreset(preset.target, preset.id);
      clearPendingPreset();
      return;
    }

    const isDownloadingExpectedModel = pendingPresetDownloads.some(
      isExpectedModelDownloading,
    );
    if (!isDownloadingExpectedModel && pendingPresetRequestsSettled >= pendingPresetDownloads.length) {
      clearPendingPreset();
    }
  });

  let advancedModelUi = $state(false);
  let localModelMemoryPolicy = $state<LocalModelMemoryPolicy>('unload_after_5m');
  let localModelMemoryDropdownOpen = $state(false);
  let localModelsSupported = $state(true);
  $effect(() => { if (transcriptionModeDropdownOpen) localModelMemoryDropdownOpen = false; });
  $effect(() => { if (localModelMemoryDropdownOpen) transcriptionModeDropdownOpen = false; });
  const LOCAL_MEMORY_POLICY_MENU_ID = 'models-local-memory-policy-menu';
  const localMemoryPolicyOptions: { value: LocalModelMemoryPolicy; label: string }[] = [
    { value: 'unload_after_5m', label: 'Unload after 5 minutes' },
    { value: 'unload_after_15m', label: 'Unload after 15 minutes' },
    { value: 'keep_loaded', label: 'Keep loaded' },
    { value: 'unload_immediately', label: 'Unload immediately' },
  ];

  function localMemoryPolicyLabel(policy: LocalModelMemoryPolicy): string {
    return localMemoryPolicyOptions.find((option) => option.value === policy)?.label
      ?? localMemoryPolicyOptions[0].label;
  }

  function taskMap(type: TaskType): ProviderModelMap {
    return type === 'transcription' ? transcriptionModelsByProvider : cleanupModelsByProvider;
  }

  function setTaskMap(type: TaskType, next: ProviderModelMap) {
    if (type === 'transcription') {
      transcriptionModelsByProvider = next;
    } else {
      cleanupModelsByProvider = next;
    }
  }

  function taskDefault(type: TaskType): string {
    return type === 'transcription' ? transcriptionDefaultModel : cleanupDefaultModel;
  }

  function setTaskDefault(type: TaskType, value: string) {
    if (type === 'transcription') {
      transcriptionDefaultModel = value;
    } else {
      cleanupDefaultModel = value;
    }
  }

  function taskFallbacks(type: TaskType): string[] {
    return type === 'transcription' ? transcriptionFallbackModels : cleanupFallbackModels;
  }

  function setTaskFallbacks(type: TaskType, next: string[]) {
    if (type === 'transcription') {
      transcriptionFallbackModels = next;
    } else {
      cleanupFallbackModels = next;
    }
  }

  function ensureModelsContainSelection(type: TaskType, provider: ProviderId, modelName: string) {
    const map = taskMap(type);
    if (!(map[provider] ?? []).includes(modelName)) {
      setTaskMap(type, { ...map, [provider]: [...(map[provider] ?? []), modelName] });
    }
  }

  function pickLocalReplacementDefault(type: TaskType, excludeLocalId: string): string {
    const localModels = type === 'transcription' ? localSttStore.models : localLlmStore.models;
    const otherLocal = localModels.find((model) => model.is_downloaded && model.id !== excludeLocalId && (type === 'cleanup' || supportsLocalLanguage(model.id, presetOptions)));
    if (otherLocal) {
      return modelId('local', otherLocal.id);
    }
    return modelId('local', excludeLocalId);
  }

  function reassignAfterLocalModelDeleted(type: TaskType, deletedLocalId: string) {
    const deletedId = modelId('local', deletedLocalId);
    const remainingFallbacks = taskFallbacks(type).filter((id) => id !== deletedId);

    if (taskDefault(type) === deletedId) {
      if (remainingFallbacks.length > 0) {
        const [nextActive, ...rest] = remainingFallbacks;
        setTaskDefault(type, nextActive);
        setTaskFallbacks(type, rest);
      } else {
        setTaskDefault(type, pickLocalReplacementDefault(type, deletedLocalId));
        setTaskFallbacks(type, remainingFallbacks);
      }
    } else if (remainingFallbacks.length !== taskFallbacks(type).length) {
      setTaskFallbacks(type, remainingFallbacks);
    }

    const map = taskMap(type);
    if (map.local?.includes(deletedLocalId)) {
      setTaskMap(type, { ...map, local: map.local.filter((id) => id !== deletedLocalId) });
    }

    persistAll().catch((err) => console.error('persist after local model delete failed', err));
  }

  async function handleDeleteTranscriptionModel(localModelId: string) {
    await deleteLocalModel(localModelId);
    const stillDownloaded = localSttStore.models.some(
      (model) => model.id === localModelId && model.is_downloaded,
    );
    if (!stillDownloaded) {
      reassignAfterLocalModelDeleted('transcription', localModelId);
    }
  }

  async function handleDeleteCleanupModel(localModelId: string) {
    await deleteLocalLlmModel(localModelId);
    const stillDownloaded = localLlmStore.models.some(
      (model) => model.id === localModelId && model.is_downloaded,
    );
    if (!stillDownloaded) {
      reassignAfterLocalModelDeleted('cleanup', localModelId);
    }
  }

  function ensureDefaultAndFallbacks() {
    for (const type of ['transcription', 'cleanup'] as TaskType[]) {
      const defaultParsed = splitModelId(taskDefault(type));
      if (!defaultParsed) continue;

      ensureModelsContainSelection(type, defaultParsed.provider, defaultParsed.model);

      const normalizedFallbacks = taskFallbacks(type)
        .map(splitModelId)
        .filter((parsed): parsed is { provider: ProviderId; model: string } => !!parsed)
        .map((parsed) => {
          ensureModelsContainSelection(type, parsed.provider, parsed.model);
          return modelId(parsed.provider, parsed.model);
        })
        .filter((id, index, all) => all.indexOf(id) === index)
        .filter((id) => id !== taskDefault(type));

      setTaskFallbacks(type, normalizedFallbacks);
    }
  }

  let persistChain: Promise<void> = Promise.resolve();
  const modelKeys = [
    'transcription_models_by_provider', 'cleanup_models_by_provider',
    'transcription_default_model', 'cleanup_default_model',
    'transcription_fallback_models', 'cleanup_fallback_models',
    'dual_transcription_enabled', 'model_selection_mode',
    'transcription_model', 'cleanup_model', 'transcription_provider', 'cleanup_provider',
    'cleanup_enabled',
  ] as const;
  const modelRelatedKeys: readonly string[] = [
    ...modelKeys, 'custom_providers', 'local_model_memory_policy', 'transcription_language',
    'advanced_model_ui', 'cleanup_prompt_override',
  ];
  let modelBaseline: Record<string, unknown> | null = null;
  let modelConflict = $state(false);
  let agentChangeVersion = 0;

  function modelSnapshot(all: SettingsSnapshot): Record<string, unknown> {
    return Object.fromEntries(modelKeys.map(key => [key, all[key] ?? null]));
  }

  function persistAll(): Promise<void> {
    ensureDefaultAndFallbacks();
    const transcriptionProvider = splitModelId(transcriptionDefaultModel)?.provider ?? 'groq';
    const cleanupProvider = splitModelId(cleanupDefaultModel)?.provider ?? 'groq';
    const snapshot = structuredClone($state.snapshot({
      transcription_models_by_provider: transcriptionModelsByProvider,
      cleanup_models_by_provider: cleanupModelsByProvider,
      transcription_default_model: transcriptionDefaultModel,
      cleanup_default_model: cleanupDefaultModel,
      transcription_fallback_models: transcriptionFallbackModels,
      dual_transcription_enabled: dualTranscriptionEnabled,
      model_selection_mode: modelSelectionMode,
      cleanup_fallback_models: cleanupFallbackModels,
      transcription_model: transcriptionDefaultModel,
      cleanup_model: cleanupDefaultModel,
      transcription_provider: transcriptionProvider,
      cleanup_provider: cleanupProvider,
      cleanup_enabled: cleanupEnabled,
    }));
    const writeSnapshot = async () => {
      if (!modelBaseline || modelConflict) throw new Error('Reload model settings before saving.');
      const version = agentChangeVersion;
      const cleanupVersion = ++appStore.cleanupChangeVersion;
      try {
        await invoke('save_model_settings_if_unchanged', { expected: modelBaseline, values: snapshot });
        modelBaseline = snapshot;
        if (version === agentChangeVersion && cleanupVersion === appStore.cleanupChangeVersion) appStore.cleanupEnabled = snapshot.cleanup_enabled;
        void refreshTranscriptionModel();
        window.dispatchEvent(new Event('verenu:setting-saved'));
      } catch (error) {
        if (String(error).includes('changed while you were editing')) modelConflict = true;
        throw error;
      }
    };

    persistChain = persistChain.then(writeSnapshot, writeSnapshot);
    return persistChain;
  }

  async function migrateAndLoad(readOnly = false) {
    const version = agentChangeVersion;
    const cleanupVersion = appStore.cleanupChangeVersion;
    void refreshAppleIntelligence();
    const [all, keyStatus] = await Promise.all([
      loadSettingsSnapshot(),
      invoke<Record<ProviderId, boolean>>('get_api_key_status'),
    ]);
    const { advanced_model_ui: advancedRaw, cleanup_enabled: cleanupRaw,
      transcription_language: language, model_selection_mode: selectionMode } = all;
    modelBaseline = modelSnapshot(all);

    apiKeyStatus = { ...apiKeyStatus, ...keyStatus, local: true };
    transcriptionLanguage = language ?? 'en';
    modelSelectionMode = selectionMode === 'fastest' || selectionMode === 'balanced' || selectionMode === 'quality' ? selectionMode : 'manual';
    invoke<ModelPerformance[]>('get_model_performance').then(value => { performance = value; }).catch(() => {});
    customProviderStore.providers = all.custom_providers ?? [];
    hydrateCatalogCache(all.provider_model_cache);
    if (typeof cleanupRaw === 'boolean') {
      cleanupEnabled = cleanupRaw;
      if (version === agentChangeVersion && cleanupVersion === appStore.cleanupChangeVersion) appStore.cleanupEnabled = cleanupRaw;
    }
    transcriptionModelsByProvider = mergeProviderModelMap(all.transcription_models_by_provider);
    const rawCleanupModelsByProvider = mergeProviderModelMap(all.cleanup_models_by_provider);
    cleanupModelsByProvider = {
      ...rawCleanupModelsByProvider,
      groq: rawCleanupModelsByProvider.groq
        .map(migrateDeprecatedGroqCleanupModel)
        .filter((model, index, models) => models.indexOf(model) === index),
    };
    const cleanupModelMapChanged = JSON.stringify(cleanupModelsByProvider) !== JSON.stringify(rawCleanupModelsByProvider);

    const rawLegacyTranscription = String(all.transcription_model ?? '');
    const rawLegacyCleanup = String(all.cleanup_model ?? '');
    const legacyTranscription = rawLegacyTranscription
      ? (rawLegacyTranscription.includes('/') ? rawLegacyTranscription : `groq/${rawLegacyTranscription}`)
      : 'groq/whisper-large-v3-turbo';
    const legacyCleanup = rawLegacyCleanup
      ? (rawLegacyCleanup.includes('/') ? rawLegacyCleanup : `groq/${rawLegacyCleanup}`)
      : 'groq/qwen/qwen3.8-27b';

    transcriptionDefaultModel = migrateDeprecatedGoogleModel(all.transcription_default_model ?? legacyTranscription);
    cleanupDefaultModel = migrateDeprecatedGoogleModel(all.cleanup_default_model ?? legacyCleanup);
    const parsedTranscriptionDefault = splitModelId(transcriptionDefaultModel);
    if (parsedTranscriptionDefault?.provider === 'google') {
      transcriptionDefaultModel = modelId('google', migrateDeprecatedGoogleModel(parsedTranscriptionDefault.model));
    }
    const parsedCleanupDefault = splitModelId(cleanupDefaultModel);
    if (parsedCleanupDefault?.provider === 'groq') {
      cleanupDefaultModel = modelId('groq', migrateDeprecatedGroqCleanupModel(parsedCleanupDefault.model));
    } else if (parsedCleanupDefault?.provider === 'google') {
      cleanupDefaultModel = modelId('google', migrateDeprecatedGoogleModel(parsedCleanupDefault.model));
    }

    if (Array.isArray(all.transcription_fallback_models)) {
      transcriptionFallbackModels = all.transcription_fallback_models
        .filter((id) => !!splitModelId(id))
        .map((id) => {
          const parsed = splitModelId(id);
          return parsed?.provider === 'google'
            ? modelId('google', migrateDeprecatedGoogleModel(parsed.model))
            : id;
        });
    }
    dualTranscriptionEnabled = all.dual_transcription_enabled === true;
    if (Array.isArray(all.cleanup_fallback_models)) {
      cleanupFallbackModels = all.cleanup_fallback_models
        .filter((id) => !!splitModelId(id))
        .map((id) => {
          const parsed = splitModelId(id);
          if (parsed?.provider === 'groq') {
            return modelId('groq', migrateDeprecatedGroqCleanupModel(parsed.model));
          }
          return parsed?.provider === 'google'
            ? modelId('google', migrateDeprecatedGoogleModel(parsed.model))
            : id;
        });
    }
    if (typeof advancedRaw === 'boolean') {
      advancedModelUi = advancedRaw;
    }
    if (
      all.local_model_memory_policy === 'keep_loaded'
      || all.local_model_memory_policy === 'unload_after_5m'
      || all.local_model_memory_policy === 'unload_after_15m'
      || all.local_model_memory_policy === 'unload_immediately'
    ) {
      localModelMemoryPolicy = all.local_model_memory_policy;
    }
    cleanupPromptStore.override =
      typeof all.cleanup_prompt_override === 'string' ? all.cleanup_prompt_override : '';

    const preTranscriptionDefault = transcriptionDefaultModel;
    const preCleanupDefault = cleanupDefaultModel;
    const preTranscriptionFallbackCount = transcriptionFallbackModels.length;
    const preCleanupFallbackCount = cleanupFallbackModels.length;
    const preCleanupFallbackModels = [...cleanupFallbackModels];
    const needsMigration =
      !all.transcription_default_model
      || !splitModelId(all.transcription_default_model)
      || !all.cleanup_default_model
      || !splitModelId(all.cleanup_default_model);

    ensureDefaultAndFallbacks();

    const changed =
      needsMigration
      || transcriptionDefaultModel !== preTranscriptionDefault
      || cleanupDefaultModel !== preCleanupDefault
      || transcriptionFallbackModels.length !== preTranscriptionFallbackCount
      || cleanupFallbackModels.length !== preCleanupFallbackCount
      || JSON.stringify(cleanupFallbackModels) !== JSON.stringify(preCleanupFallbackModels)
      || cleanupModelMapChanged;

    if (changed && !readOnly) {
      await persistAll();
    }

    await Promise.all([
      refreshLocalModels(),
      refreshLocalState(),
      refreshLocalLlmModels(),
      refreshLocalLlmState(),
    ]);

    if (!readOnly) validateLocalSelectionsAfterLoad();
    await refreshTranscriptionModel();
    if (version === agentChangeVersion) modelConflict = false;
  }

  function validateLocalSelectionsAfterLoad() {
    for (const type of ['transcription', 'cleanup'] as TaskType[]) {
      const localModels = type === 'transcription' ? localSttStore.models : localLlmStore.models;
      const downloadedIds = new Set(localModels.filter((model) => model.is_downloaded).map((model) => model.id));

      const missingLocalIds = [...new Set(
        [taskDefault(type), ...taskFallbacks(type)]
          .map((id) => splitModelId(id))
          .filter((parsed): parsed is { provider: ProviderId; model: string } => !!parsed && parsed.provider === 'local')
          .map((parsed) => parsed.model)
          .filter((modelName) => !downloadedIds.has(modelName)),
      )];

      for (const missingId of missingLocalIds) {
        reassignAfterLocalModelDeleted(type, missingId);
      }
    }
  }

  function selectModel(type: TaskType, id: string) {
    modelSelectionMode = 'manual';
    const parsed = splitModelId(id);
    if (!parsed) return;
    ensureModelsContainSelection(type, parsed.provider, parsed.model);
    setTaskFallbacks(type, taskFallbacks(type).filter((entry) => entry !== id));
    setTaskDefault(type, id);
    persistAll().catch((err) => console.error('persist model selection failed', err));
  }

  function addFallbackModel(type: TaskType, id: string) {
    modelSelectionMode = 'manual';
    const parsed = splitModelId(id);
    if (!parsed) return;
    if (taskDefault(type) === id || taskFallbacks(type).includes(id)) return;
    ensureModelsContainSelection(type, parsed.provider, parsed.model);
    setTaskFallbacks(type, [...taskFallbacks(type), id]);
    persistAll().catch((err) => console.error('persist fallback add failed', err));
  }

  function removeFallbackModel(type: TaskType, id: string) {
    modelSelectionMode = 'manual';
    setTaskFallbacks(type, taskFallbacks(type).filter((entry) => entry !== id));
    persistAll().catch((err) => console.error('persist fallback removal failed', err));
  }

  function moveFallbackModel(type: TaskType, id: string, delta: -1 | 1) {
    modelSelectionMode = 'manual';
    const chain = [...taskFallbacks(type)];
    const index = chain.indexOf(id);
    const next = index + delta;
    if (index === -1 || next < 0 || next >= chain.length) return;
    [chain[index], chain[next]] = [chain[next], chain[index]];
    setTaskFallbacks(type, chain);
    persistAll().catch((err) => console.error('persist fallback reorder failed', err));
  }

  function addCustomModelForTask(type: TaskType, draft: string) {
    const raw = draft.trim();
    if (!raw) return;
    const parsed = splitModelId(raw) ?? {
      provider: splitModelId(taskDefault(type))?.provider ?? 'groq',
      model: raw,
    };
    if (!parsed.model) return;
    ensureModelsContainSelection(type, parsed.provider, parsed.model);
    selectModel(type, modelId(parsed.provider, parsed.model));
  }

  let advancedModelUiError = $state(false);

  async function handleAdvancedModelUi(value: boolean) {
    advancedModelUi = value;
    try {
      await saveSetting('advanced_model_ui', value);
    } catch (err) {
      advancedModelUi = !value;
      advancedModelUiError = true;
      console.error('save advanced_model_ui failed:', err);
    }
  }

  $effect(() => {
    const target = settingsSearchNavigation.request?.target;
    if (target?.startsWith('models-') && !advancedModelUi) {
      void handleAdvancedModelUi(true);
    }
  });

  let transcriptionModeDropdownOpen = $state(false);

  async function handleDualTranscription(value: boolean) {
    modelSelectionMode = 'manual';
    dualTranscriptionEnabled = value;
    transcriptionModeDropdownOpen = false;
    try {
      await persistAll();
    } catch (err) {
      dualTranscriptionEnabled = !value;
      console.error('save dual transcription setting failed:', err);
    }
  }

  async function updateLocalMemoryPolicy(policy: LocalModelMemoryPolicy) {
    localModelMemoryPolicy = policy;
    localModelMemoryDropdownOpen = false;
    try {
      await saveSetting('local_model_memory_policy', policy);
    } catch (err) {
      console.error('save local model memory policy failed', err);
    }
  }

  onMount(() => {
    const onAgentSettingsChanged = (event: Event) => {
      const keys = (event as CustomEvent<string[] | null>).detail;
      agentChangeVersion += 1;
      if (!keys || keys.some(key => modelRelatedKeys.includes(key))) modelConflict = true;
      void invoke<Record<ProviderId, boolean>>('get_api_key_status')
        .then(status => { apiKeyStatus = { ...apiKeyStatus, ...status, local: true }; })
        .catch(err => console.error('refresh API key status failed', err));
    };
    window.addEventListener('verenu:agent-settings-changed', onAgentSettingsChanged);
    refreshLocalModels().catch((err) => console.error('refresh local models failed', err));
    refreshLocalState().catch((err) => console.error('refresh local state failed', err));
    refreshLocalLlmModels().catch((err) => console.error('refresh local cleanup models failed', err));
    refreshLocalLlmState().catch((err) => console.error('refresh local cleanup state failed', err));
    refreshLocalLlmRuntimeInfo().catch((err) => console.error('refresh local cleanup runtime info failed', err));
    invoke<boolean>('local_models_supported_on_this_platform')
      .then((supported) => {
        if (supported === false) localModelsSupported = false;
      })
      .catch((err) => console.error('check local models platform support failed', err));
    getHardware()
      .then((hw) => (hardware = hw))
      .catch((err) => console.error('read hardware capabilities failed', err));

    const onKeySaved = async (event: Event) => {
      const provider = (event as CustomEvent<{ provider: ProviderId }>).detail?.provider;
      if (!provider) return;
      apiKeyStatus = { ...apiKeyStatus, [provider]: true };
      await refreshCatalog(provider, trackedModelIds());
    };
    const onKeyDeleted = (event: Event) => {
      const provider = (event as CustomEvent<{ provider: ProviderId }>).detail?.provider;
      if (provider) apiKeyStatus = { ...apiKeyStatus, [provider]: false };
    };
    window.addEventListener('verenu:api-key-saved', onKeySaved);
    window.addEventListener('verenu:api-key-deleted', onKeyDeleted);
    return () => {
      window.removeEventListener('verenu:agent-settings-changed', onAgentSettingsChanged);
      window.removeEventListener('verenu:api-key-saved', onKeySaved);
      window.removeEventListener('verenu:api-key-deleted', onKeyDeleted);
    };
  });

  $effect(() => {
    refreshStaleCatalogs(apiKeyStatus, trackedModelIds()).catch((err) =>
      console.error('refresh model catalogs failed', err),
    );
  });

  migrateAndLoad().catch((err) => console.error('load models failed', err));
</script>

<h2 class="settings-h">Models</h2>

{#if modelConflict}
  <div class="panel-note" role="alert">
    Model settings changed outside this screen. Reload to use the latest settings; unsaved selections here will be discarded.
    <button class="btn-ghost" onclick={() => { void persistChain.catch(() => {}).then(() => migrateAndLoad(true)); }}>Reload model settings</button>
  </div>
{/if}

<div inert={modelConflict}>
<div data-setting-target="model-presets">
  <ModelPresetPicker
    {apiKeyStatus}
    {hardware}
    localSupported={localModelsSupported}
    {activeConfig}
    {installedLocal}
    {downloadingLocal}
    onApplyPreset={applyPreset}
    onOpenApiKeys={openApiKeysSection}
    onCancelPreset={cancelPresetDownload}
    onDeletePreset={deletePresetModels}
    options={presetOptions}
    onUseFallback={useOfflineFallback}
    onTestLocal={testLocalSpeed}
    {testingLocal}
    selectionMode={modelSelectionMode}
    pendingPresetId={pendingPreset?.id ?? null}
  />
  {#if localTestResult}<p class="adv-desc" role="status">{localTestResult}</p>{/if}
</div>

<div class="advanced-toggle-row" data-setting-target="advanced-models">
  <div class="adv-text">
    <span class="adv-label">Advanced Models</span>
    <span class="adv-desc">Choose specific models, edit cleanup prompts, and manage downloads</span>
  </div>
  <Toggle checked={advancedModelUi} onchange={handleAdvancedModelUi} label="Advanced Models" bind:error={advancedModelUiError} />
</div>

{#if advancedModelUi}
<div class="advanced-block" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
<h3 class="settings-subhead">Model selection</h3>
<ModelTaskTile
  type="transcription"
  {advancedModelUi}
  {apiKeyStatus}
  context={pickerContext('transcription')}
  defaultModel={transcriptionDefaultModel}
  fallbackModels={transcriptionFallbackModels}
  onSelectModel={selectModel}
  onAddFallback={addFallbackModel}
  onRemoveFallback={removeFallbackModel}
  onMoveFallback={moveFallbackModel}
  onAddCustomModel={addCustomModelForTask}
  onOpenApiKeys={openApiKeysSection}
  local={localControls('transcription')}
/>

<ModelTaskTile
  type="cleanup"
  {advancedModelUi}
  {apiKeyStatus}
  context={pickerContext('cleanup')}
  defaultModel={cleanupDefaultModel}
  fallbackModels={cleanupFallbackModels}
  onSelectModel={selectModel}
  onAddFallback={addFallbackModel}
  onRemoveFallback={removeFallbackModel}
  onMoveFallback={moveFallbackModel}
  onAddCustomModel={addCustomModelForTask}
  onOpenApiKeys={openApiKeysSection}
  local={localControls('cleanup')}
/>

<h3 class="settings-subhead">Model settings</h3>
<div class="setting-row transcription-mode-row" data-setting-target="models-strategy">
  <div>
    <div class="label">Transcription strategy</div>
    <div class="desc">Use one model, or compare two working models from the existing transcription fallback chain before cleanup.</div>
  </div>
  <Dropdown bind:open={transcriptionModeDropdownOpen} closeSelector=".models-dropdown">
    <div class="ui-dropdown models-dropdown">
      <button
        class="ui-dropdown-trigger ui-dropdown-trigger--compact models-dropdown-btn"
        type="button"
        onclick={() => (transcriptionModeDropdownOpen = !transcriptionModeDropdownOpen)}
        aria-haspopup="listbox"
        aria-expanded={transcriptionModeDropdownOpen}
        aria-controls="transcription-mode-menu"
        aria-label="Transcription strategy"
      >
        <span>{dualTranscriptionEnabled ? 'Dual model' : 'Single model'}</span>
        <svg class:open={transcriptionModeDropdownOpen} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="m6 9 6 6 6-6"/>
        </svg>
      </button>
      {#if transcriptionModeDropdownOpen}
        <div
          id="transcription-mode-menu"
          class="ui-dropdown-menu ui-dropdown-menu--padded models-dropdown-menu scroll-styled scroll-thumb-elev"
          role="listbox"
          tabindex="-1"
          aria-label="Transcription strategy"
          in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast), easing: expoOut }}
          out:fly={{ y: -3, duration: motionMs(110), easing: expoOut }}
        >
          <button
            class="ui-dropdown-option models-dropdown-item"
            class:is-active={!dualTranscriptionEnabled}
            type="button"
            onclick={() => handleDualTranscription(false)}
            role="option"
            aria-selected={!dualTranscriptionEnabled}
          >
            <span>Single model</span>
            <small>Fastest, uses the primary model and fallbacks only on failure.</small>
          </button>
          <button
            class="ui-dropdown-option models-dropdown-item"
            class:is-active={dualTranscriptionEnabled}
            type="button"
            onclick={() => handleDualTranscription(true)}
            role="option"
            aria-selected={dualTranscriptionEnabled}
          >
            <span>Dual model</span>
            <small>Compares two successful fallback-chain models before cleanup.</small>
          </button>
        </div>
      {/if}
    </div>
  </Dropdown>
</div>
</div>
{/if}

<div class="setting-row" data-setting-target="models-memory">
  <div>
    <div class="label">Memory policy</div>
    <div class="desc">Controls when idle local models are unloaded.</div>
  </div>
  <Dropdown bind:open={localModelMemoryDropdownOpen} closeSelector=".models-dropdown">
    <div class="ui-dropdown models-dropdown">
      <button
        class="ui-dropdown-trigger ui-dropdown-trigger--compact models-dropdown-btn"
        type="button"
        use:animateWidth={{ text: localMemoryPolicyLabel(localModelMemoryPolicy), max: 220 }}
        onclick={() => (localModelMemoryDropdownOpen = !localModelMemoryDropdownOpen)}
        aria-haspopup="listbox"
        aria-expanded={localModelMemoryDropdownOpen}
        aria-controls={LOCAL_MEMORY_POLICY_MENU_ID}
        aria-label="Local model memory policy"
      >
        <span>{localMemoryPolicyLabel(localModelMemoryPolicy)}</span>
        <svg class:open={localModelMemoryDropdownOpen} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="m6 9 6 6 6-6"/>
        </svg>
      </button>
      {#if localModelMemoryDropdownOpen}
        <div
          id={LOCAL_MEMORY_POLICY_MENU_ID}
          class="ui-dropdown-menu ui-dropdown-menu--padded models-dropdown-menu scroll-styled scroll-thumb-elev"
          role="listbox"
          tabindex="-1"
          aria-label="Local model memory policy options"
          in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast), easing: expoOut }}
          out:fly={{ y: -3, duration: motionMs(110), easing: expoOut }}
        >
          {#each localMemoryPolicyOptions as option}
            <button
              class="ui-dropdown-option models-dropdown-item"
              class:is-active={localModelMemoryPolicy === option.value}
              type="button"
              onclick={() => updateLocalMemoryPolicy(option.value)}
              role="option"
              aria-selected={localModelMemoryPolicy === option.value}
            >
              {option.label}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </Dropdown>
</div>

{#if !hardware.isAndroid}
<div class="setting-row" data-setting-target="models-folder">
  <div>
    <div class="label">Models folder</div>
    <div class="desc">Open the shared folder where local transcription and cleanup models are stored.</div>
  </div>
  <button class="btn-ghost" type="button" onclick={openLocalModelsFolder}>Open models folder</button>
</div>
{/if}

</div>

<style>

  .advanced-toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 13px 0;
    border-top: 1px solid var(--line);
  }

  .transcription-mode-row {
    position: relative;
    padding: 14px 0;
  }

  .models-dropdown-item {
    display: grid;
    gap: 3px;
    text-align: left;
  }

  .models-dropdown-item small {
    color: var(--ink-mute);
    font-size: 11px;
    line-height: 1.35;
  }

  .adv-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .adv-label {
    font-size: 13px;
    font-family: var(--sans);
    font-weight: 500;
    color: var(--ink-soft);
  }

  .adv-desc {
    font-size: 11px;
    font-family: var(--sans);
    color: var(--ink-mute);
  }

  .models-dropdown-btn {
    max-width: 220px;
  }

  .models-dropdown-btn span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .models-dropdown-menu {
    min-width: 240px;
    max-height: 260px;
    right: 0;
    left: auto;
  }

  .models-dropdown-item span { font-weight: 550; }
  .models-dropdown-item.is-active small { color: var(--ink-soft); }

  @container settings-panel (max-width: 700px) {
    .models-dropdown {
      width: 100%;
    }

    .models-dropdown-btn {
      width: 100% !important;
      max-width: none;
      justify-content: space-between;
    }

    .models-dropdown-menu {
      width: max-content;
      min-width: min(240px, 100cqw);
      max-width: min(320px, 100cqw);
      left: auto;
      right: 0;
    }

    .models-dropdown-item {
      min-width: 0;
      white-space: normal;
    }

    .models-dropdown-item small {
      overflow-wrap: anywhere;
      white-space: normal;
    }

    .models-dropdown-btn { min-height: 44px; }
    .models-dropdown-item { min-height: 44px; padding-block: 10px; }
  }

  @container settings-panel (max-width: 400px) {
    .models-dropdown-menu {
      width: 100%;
      min-width: 100%;
      max-width: 100%;
      left: 0;
      right: 0;
    }
  }
</style>
