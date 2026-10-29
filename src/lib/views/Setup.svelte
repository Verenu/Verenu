<script lang="ts">
  import { formatIpcError } from '../errors';
  import { invoke } from '../tauri';
  import { onMount } from 'svelte';
  import { appStore } from '../stores';
  import { loadSettingsSnapshot, saveSetting, type CleanupIntensity, type ProviderId, type ProviderModelMap, type ToneId } from '../settings';
  import { refreshTranscriptionModel } from '../transcriptionModelStore.svelte';
  import { getTranscriptionLanguageLabel, transcriptionLanguages, type TranscriptionLanguageCode } from '../transcriptionLanguages';
  import { isMac, isAndroid } from '../platform';
  import { motionMs, pageSwap } from '../motion';
  import { loadHotkey } from '../hotkey.svelte';
  import { localSttStore, refreshLocalModels, refreshLocalState } from '../localSttStore.svelte';
  import { localLlmStore, refreshLocalLlmModels, refreshLocalLlmState, refreshLocalLlmRuntimeInfo } from '../localLlmStore.svelte';
  import {
    providers,
    cleanupCards,
    toneCards,
    SETUP_APPEARANCE_MODE,
    type WizardProviderId,
  } from '../setup/setupData';
  import type { Preset } from '../components/settings/modelPresets';
  import { splitModelId } from '../components/settings/models';
  import { parseSetupProgress, resumeStep } from '../setup/setupProgress';
  import { setupCleanupEnabled, setupDefaultModels, setupModelReadiness } from '../setup/setupModelReadiness';
  import { appleIntelligence, refreshAppleIntelligence } from '../appleIntelligence.svelte';
  import { appleCleanupReadiness, applyAppleCleanup, applyAppleCleanupDefaults } from '../setup/appleCleanup';
  import SetupShell from '../setup/SetupShell.svelte';
  import IntroStep from '../setup/steps/IntroStep.svelte';
  import AnalyticsStep from '../setup/steps/AnalyticsStep.svelte';
  import ProviderStep from '../setup/steps/ProviderStep.svelte';
  import ApiKeyStep from '../setup/steps/ApiKeyStep.svelte';
  import PermissionsStep from '../setup/steps/PermissionsStep.svelte';
  import AndroidPermissionsStep from '../setup/steps/AndroidPermissionsStep.svelte';
  import ModelsStep from '../setup/steps/ModelsStep.svelte';
  import WritingStyleStep from '../setup/steps/WritingStyleStep.svelte';
  import LanguageStep from '../setup/steps/LanguageStep.svelte';
  import AudioEnvironmentStep from '../setup/steps/AudioEnvironmentStep.svelte';
  import TryItStep from '../setup/steps/TryItStep.svelte';
  import DoneStep from '../setup/steps/DoneStep.svelte';

  const hasOsPermissionStep = isMac || isAndroid;
  const onboardingTotalSteps = isAndroid ? 7 : hasOsPermissionStep ? 9 : 8;
  const analyticsStep = 1;
  const providerStep = 2;
  const apiKeyStep = 3;
  const permissionStep = hasOsPermissionStep ? 4 : -1;
  const modelsStep = isAndroid ? -1 : hasOsPermissionStep ? 5 : 4;
  const writingStyleStep = isAndroid ? 5 : hasOsPermissionStep ? 6 : 5;
  const languageStep = isAndroid ? 6 : hasOsPermissionStep ? 7 : 6;
  const audioEnvStep = isAndroid ? -1 : hasOsPermissionStep ? 8 : 7;
  const tryItStep = isAndroid ? 7 : hasOsPermissionStep ? 9 : 8;
  const doneStep = onboardingTotalSteps + 1;
  const setupKeys = [
    'cleanup_intensity', 'default_tone', 'cleanup_enabled', 'transcription_provider',
    'transcription_model', 'transcription_default_model', 'transcription_models_by_provider',
    'transcription_fallback_models', 'dual_transcription_enabled', 'transcription_language',
    'cleanup_provider', 'cleanup_model', 'cleanup_default_model', 'cleanup_models_by_provider',
    'cleanup_fallback_models', 'appearance_mode', 'mute_audio', 'pause_media_during_dictation',
    'noise_reduction', 'contextual_formatting_enabled', 'contextual_caps_enabled',
    'auto_spacing_enabled', 'caps_lock_uppercase_enabled', 'auto_learn_enabled',
  ] as const;
  let setupBaseline = $state<Record<string, unknown> | null>(null);
  let setupConflict = $state(false);
  let preserveModels = $state(false);
  let preserveProvider = $state(false);
  let preserveDisabledIntensity = $state(false);
  const modelKeys = [
    'transcription_provider', 'transcription_model', 'transcription_default_model',
    'transcription_models_by_provider', 'transcription_fallback_models', 'dual_transcription_enabled',
    'cleanup_provider', 'cleanup_model', 'cleanup_default_model', 'cleanup_models_by_provider',
    'cleanup_fallback_models',
  ] as const;

  let step = $state(0);
  let progressLoaded = false;
  let direction = $state<'forward' | 'back'>('forward');
  let animating = $state(false);
  let queuedJump: number | null = null;
  let stepWrapEl = $state<HTMLDivElement | null>(null);

  let provider = $state<WizardProviderId>('groq');
  let localAiSupported = $state(!isAndroid);
  let localAiUnsupportedReason = $state('');
  let apiKeyDraft = $state('');
  let apiKeyMode = $state<'fork' | 'tutorial' | 'paste'>('fork');
  let keySaved = $state(false);
  let providerKeyStatus = $state<Record<ProviderId, boolean>>({
    'apple-intelligence': false,
    groq: false,
    openai: false,
    google: false,
    assemblyai: false,
    openrouter: false,
    xai: false,
    local: true,
  });
  let previousProvider = $state<ProviderId | null>(null);
  let keySaving = $state(false);
  let keyError = $state('');
  let showKey = $state(false);
  let keyValidation = $state<{ status: 'idle' | 'checking' | 'valid' | 'invalid' | 'unknown'; message: string }>({ status: 'idle', message: '' });

  let allCoreGranted = $state(false);
  let modelPreset = $state<Preset | null>(null);
  let providerExplicitlySelected = $state(false);
  let presetKeyStatus = $derived.by(() => {
    if (!providerExplicitlySelected) return providerKeyStatus;
    return Object.fromEntries(Object.entries(providerKeyStatus).map(([id, present]) =>
      [id, id === provider && present])) as Record<ProviderId, boolean>;
  });
  let appleCleanupChoice = $state(false);

  let cleanupIntensity = $state<CleanupIntensity>('medium');
  const aiCleanupSelected = $derived(cleanupIntensity !== 'none' && cleanupIntensity !== 'rules');
  const useAppleCleanup = $derived(appleCleanupChoice && aiCleanupSelected);
  let tone = $state<ToneId>('casual');
  let language = $state<TranscriptionLanguageCode>('en');
  let usesHeadphones = $state(true);
  let saveError = $state('');
  let finishing = $state(false);
  let analyticsEnabled = $state(true);
  let analyticsError = $state(false);
  const setupStepEnteredAt = new Map<number, number>();

  let providerDisplayName = $derived(providers.find((p) => p.id === provider)?.name ?? '');
  let cleanupName = $derived(cleanupCards.find((c) => c.id === cleanupIntensity)?.name ?? '');
  let defaultModels = $derived(applyAppleCleanupDefaults(setupDefaultModels(provider), useAppleCleanup));
  let effectiveTarget = $derived(applyAppleCleanup(modelPreset?.target, useAppleCleanup));
  let effectiveCleanupEnabled = $derived(setupCleanupEnabled(cleanupIntensity, preserveModels ? null : effectiveTarget));
  let effectiveCleanupName = $derived(effectiveCleanupEnabled ? cleanupName : 'Off');
  function savedModel(current: unknown, legacy: unknown, fallback: string): string {
    for (const value of [current, legacy]) {
      if (typeof value === 'string' && splitModelId(value)) return value;
    }
    return fallback;
  }
  let retainedModels = $derived({
    transcriptionDefaultModel: savedModel(setupBaseline?.transcription_default_model, setupBaseline?.transcription_model, 'groq/whisper-large-v3-turbo'),
    cleanupDefaultModel: savedModel(setupBaseline?.cleanup_default_model, setupBaseline?.cleanup_model, 'groq/qwen/qwen3.8-27b'),
  });
  const activeAppleCleanup = $derived(preserveModels ? retainedModels.cleanupDefaultModel === 'apple-intelligence/system' : appleCleanupChoice);
  const appleReadiness = $derived(appleCleanupReadiness(activeAppleCleanup, aiCleanupSelected, appleIntelligence.status));
  let doneProvider = $derived(
    (preserveModels
      ? splitModelId(retainedModels.transcriptionDefaultModel)?.provider
      : splitModelId(modelPreset?.target?.transcriptionDefaultModel ?? '')?.provider) ?? provider
  );
  let keyProvider = $derived(doneProvider);
  let doneHasKey = $derived(doneProvider === 'local' || !!providerKeyStatus[doneProvider]);
  let doneModelReadiness = $derived(setupModelReadiness(preserveModels ? null : effectiveTarget, {
    speechModels: localSttStore.models,
    cleanupModels: localLlmStore.models,
    transcriptionState: localSttStore.state,
    cleanupState: localLlmStore.state,
    cleanupRuntime: localLlmStore.runtime,
  }, effectiveCleanupEnabled && cleanupIntensity !== 'rules', preserveModels ? retainedModels : defaultModels));
  let toneName = $derived(toneCards.find((t) => t.id === tone)?.name ?? '');
  let languageLabel = $derived(getTranscriptionLanguageLabel(language));

  onMount(async () => {
    sendSetupEvent('setup_started');
    preserveModels = localStorage.getItem('verenu-setup-reconcile') === '1';
    void loadHotkey();
    void Promise.allSettled([
      refreshLocalModels(), refreshLocalState(),
      refreshLocalLlmModels(), refreshLocalLlmState(), refreshLocalLlmRuntimeInfo(),
    ]);
    if (isAndroid && typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)) {
      localAiSupported = true;
    } else if (isAndroid) {
      void invoke<{ localAiSupported?: boolean; localAiUnsupportedReason?: string }>('android_get_platform_info')
        .then((info) => {
          localAiSupported = info.localAiSupported !== false;
          localAiUnsupportedReason = info.localAiUnsupportedReason ?? '';
          if (!localAiSupported && provider === 'local') provider = 'groq';
        })
        .catch(() => {
          localAiSupported = false;
        });
    }
    try {
      const [
        snapshot, keyStatus, savedAnalytics, savedProgress,
      ] = await Promise.all([
        loadSettingsSnapshot(),
        invoke<Record<ProviderId, boolean> | null>('get_api_key_status'),
        invoke<boolean | null>('get_setting', { key: 'analytics_enabled' }),
        invoke<unknown>('get_setting', { key: 'setup_progress' }),
      ]);
      setupBaseline = Object.fromEntries(setupKeys.map(key => [key, snapshot[key] ?? null]));
      const progress = parseSetupProgress(savedProgress, providers.map((p) => p.id));
      preserveProvider = snapshot.transcription_provider != null;
      if (modelKeys.some(key => key !== 'transcription_provider' && snapshot[key] != null)) {
        preserveModels = true;
      }
      // A provider outside the wizard cannot be represented by its presets.
      // Keep its model fields unset until the user makes an explicit choice.
      if (snapshot.transcription_provider && !providers.some(p => p.id === snapshot.transcription_provider)) {
        preserveModels = true;
      }
      const { transcription_language: savedLanguage, transcription_provider: savedProvider,
        cleanup_intensity: savedIntensity, cleanup_enabled: savedCleanupEnabled,
        default_tone: savedTone, mute_audio: savedMute } = snapshot;
      if (savedLanguage && transcriptionLanguages.some((o) => o.code === savedLanguage)) language = savedLanguage;
      const retainedProvider = splitModelId(savedModel(snapshot.transcription_default_model, snapshot.transcription_model, 'groq/whisper-large-v3-turbo'))?.provider;
      const initialProvider = preserveModels ? retainedProvider : savedProvider;
      if (initialProvider && providers.some((p) => p.id === initialProvider) && (initialProvider !== 'local' || localAiSupported)) provider = initialProvider as WizardProviderId;
      if (savedIntensity && cleanupCards.some((c) => c.id === savedIntensity)) cleanupIntensity = savedIntensity;
      if (savedCleanupEnabled === false) cleanupIntensity = 'none';
      preserveDisabledIntensity = savedCleanupEnabled === false;
      if (savedTone && toneCards.some((t) => t.id === savedTone)) tone = savedTone;
      if (keyStatus) {
        providerKeyStatus = { ...providerKeyStatus, ...keyStatus, local: true };
      }
      if (savedMute === true) usesHeadphones = false;
      analyticsEnabled = savedAnalytics ?? true;
      await resumeSavedProgress(progress, keyStatus);
    } catch {}
    progressLoaded = true;
  });

  onMount(() => {
    let mounted = true;
    const refreshKeys = () => {
      void invoke<Record<ProviderId, boolean>>('get_api_key_status').then(status => {
        if (mounted) providerKeyStatus = { ...providerKeyStatus, ...status, local: true };
      }).catch(() => {});
    };
    window.addEventListener('verenu:api-key-saved', refreshKeys);
    window.addEventListener('verenu:api-key-deleted', refreshKeys);
    return () => {
      mounted = false;
      window.removeEventListener('verenu:api-key-saved', refreshKeys);
      window.removeEventListener('verenu:api-key-deleted', refreshKeys);
    };
  });

  async function resumeSavedProgress(saved: ReturnType<typeof parseSetupProgress>, keyStatus: Record<ProviderId, boolean> | null) {
    if (!saved) return;
    if (!preserveModels && !preserveProvider && saved.provider && providers.some((p) => p.id === saved.provider) && (saved.provider !== 'local' || localAiSupported)) {
      provider = saved.provider as WizardProviderId;
    }
    let keys: Partial<Record<ProviderId, boolean>> = { ...(keyStatus ?? {}) };
    if (isAndroid && '__TAURI_INTERNALS__' in window) {
      try {
        const durable = await invoke<Partial<Record<ProviderId, boolean>>>('plugin:verenu-security|getCredentialStatus');
        keys = { ...keys, ...Object.fromEntries(Object.entries(durable).filter(([, present]) => present)) };
        providerKeyStatus = { ...providerKeyStatus, ...keys, local: true };
      } catch {}
    }
    step = resumeStep(saved.step, {
      apiKeyStep,
      doneStep,
      keyReady: keyProvider === 'local' || !!keys[keyProvider],
    });
  }

  $effect(() => {
    const progress = { step, provider };
    if (progressLoaded) void saveSetting('setup_progress', progress).catch(() => {});
  });

  $effect(() => {
    const currentStep = setupStepName(step);
    if (currentStep) {
      sendSetupEvent('setup_step_viewed', currentStep);
      setupStepEnteredAt.set(step, performance.now());
    }
  });

  $effect(() => {
    if (previousProvider !== null && previousProvider !== keyProvider) {
      apiKeyDraft = '';
      keyError = '';
      keyValidation = { status: 'idle', message: '' };
      apiKeyMode = 'fork';
    }
    previousProvider = keyProvider;
    keySaved = keyProvider === 'local' ? true : !!providerKeyStatus[keyProvider];
    if (keyProvider === 'local') {
      keyError = '';
      keyValidation = { status: 'idle', message: '' };
    }
  });

  const onboardingTryItStep = tryItStep;
  const onboardingDoneStep = doneStep;

  function setupStepName(value: number): string | null {
    if (value === 0) return 'intro';
    if (value === analyticsStep) return 'analytics';
    if (value === providerStep) return 'provider';
    if (value === apiKeyStep) return 'api_key';
    if (value === permissionStep) return 'permissions';
    if (value === modelsStep) return 'models';
    if (value === writingStyleStep) return 'writing_style';
    if (value === languageStep) return 'language';
    if (value === audioEnvStep) return 'audio_environment';
    if (value === onboardingTryItStep) return 'try_it';
    if (value === onboardingDoneStep) return 'done';
    return null;
  }

  function sendSetupEvent(event: string, setupStep?: string | null, durationMs?: number) {
    void invoke('analytics_setup_event', {
      event,
      step: setupStep ?? null,
      durationMs: durationMs ?? null,
    }).catch(() => {});
  }

  function delay(ms: number) { return new Promise((r) => setTimeout(r, ms)); }

  const STEP_SHIFT = 26;
  const stepInParams = $derived({
    axis: 'x' as const,
    distance: direction === 'forward' ? STEP_SHIFT : -STEP_SHIFT,
    duration: 300,
  });
  const stepOutParams = $derived({
    axis: 'x' as const,
    distance: direction === 'forward' ? -STEP_SHIFT : STEP_SHIFT,
    duration: 190,
  });

  async function animateTo(target: number, dir: 'forward' | 'back') {
    if (animating) return;
    const currentStep = setupStepName(step);
    const enteredAt = setupStepEnteredAt.get(step);
    if (currentStep && enteredAt !== undefined) {
      sendSetupEvent('setup_step_duration', currentStep, Math.max(0, Math.round(performance.now() - enteredAt)));
    }
    if (dir === 'forward') sendSetupEvent('setup_step_completed', currentStep);
    direction = dir;
    animating = true;
    step = target;
    await delay(motionMs(300));
    animating = false;
    const queued = queuedJump;
    queuedJump = null;
    if (queued !== null) jumpToStep(queued);
  }

  const goNext = () => animateTo(step + 1, 'forward');
  const skip = goNext;

  function goBack() {
    if (step > 0) void animateTo(step - 1, 'back');
  }

  async function handleAnalytics(value: boolean) {
    const previous = analyticsEnabled;
    analyticsEnabled = value;
    try {
      await saveSetting('analytics_enabled', value);
    } catch {
      analyticsEnabled = previous;
      analyticsError = true;
    }
  }

  function jumpToStep(target: number) {
    if (finishing) return;
    if (animating) {
      queuedJump = target;
      return;
    }
    if (target === step) return;
    void animateTo(target, target < step ? 'back' : 'forward');
  }

  function chooseCloudProviderFromModels(localSupport: 'unsupported' | 'unknown') {
    preserveModels = false;
    preserveProvider = false;
    modelPreset = null;
    provider = 'groq';
    localAiSupported = false;
    if (localSupport === 'unsupported') {
      localAiUnsupportedReason = 'On-device models are not available on Intel Macs yet. They have not been tested on Intel hardware. Choose a cloud provider such as Groq, OpenAI, or Google.';
    } else {
      localAiUnsupportedReason = 'Could not confirm whether on-device models are available. Choose a cloud provider to continue.';
    }
    jumpToStep(providerStep);
  }

  async function saveKey() {
    if (keyProvider === 'local') return;
    const trimmed = apiKeyDraft.trim();
    if (!trimmed) return;
    keySaving = true;
    keyError = '';
    keyValidation = { status: 'idle', message: '' };
    try {
      if (isAndroid) {
        try {
          await invoke('plugin:verenu-security|saveCredential', { provider: keyProvider, key: trimmed });
        } catch (error) {
          console.warn('direct Android credential save unavailable; using bridge rotation', error);
        }
      }
      try {
        await invoke('save_api_key', { provider: keyProvider, key: trimmed });
      } catch (error) {
        if (!isAndroid) throw error;
        console.warn('Android in-memory credential cache unavailable; continuing with rotation', error);
      }
      if (isAndroid) await invoke('android_keystore_save', { provider: keyProvider, key: trimmed });
      providerKeyStatus = { ...providerKeyStatus, [keyProvider]: true };
      keySaved = true;
      apiKeyDraft = '';
    } catch (err) {
      keyError = formatIpcError(err, 'Could not save your API key on this device');
      keySaving = false;
      return;
    }
    keySaving = false;
    void validateKey(trimmed);
  }

  async function validateKey(key: string) {
    if (keyProvider === 'local') {
      keyValidation = { status: 'idle', message: '' };
      return;
    }
    keyValidation = { status: 'checking', message: '' };
    try {
      const result = await invoke<{ ok: boolean; status: 'valid' | 'invalid' | 'unknown'; message: string }>('validate_api_key', { provider: keyProvider, key });
      keyValidation = { status: result.status, message: result.message };
    } catch (err) {
      keyValidation = { status: 'unknown', message: formatIpcError(err, 'Could not verify this API key') };
    }
  }

  function modelsByProvider(...selected: string[]): ProviderModelMap {
    const map: ProviderModelMap = {
      groq: ['whisper-large-v3-turbo', 'whisper-large-v3'],
      openai: ['gpt-4o-mini-transcribe', 'gpt-4o-transcribe'],
      google: ['gemini-2.5-flash', 'gemini-3.5-transcribe'],
      assemblyai: [],
      openrouter: [],
      xai: [],
      local: ['parakeet-v3'],
      'apple-intelligence': [],
    };
    for (const id of selected) {
      const parsed = splitModelId(id);
      if (!parsed) continue;
      const providerModels = map[parsed.provider];
      if (!providerModels) continue;
      if (!providerModels.includes(parsed.model)) providerModels.push(parsed.model);
    }
    return map;
  }

  function cleanupModelsByProvider(...selected: string[]): ProviderModelMap {
    const map: ProviderModelMap = {
      groq: ['qwen/qwen3.8-27b'],
      openai: ['gpt-4o-mini', 'gpt-4o'],
      google: ['gemini-3.5-flash-lite'],
      assemblyai: [],
      openrouter: [],
      xai: [],
      local: ['qwen2.5-3b-instruct'],
      'apple-intelligence': [],
    };
    for (const id of selected) {
      const parsed = splitModelId(id);
      if (!parsed) continue;
      const providerModels = map[parsed.provider];
      if (!providerModels) continue;
      if (!providerModels.includes(parsed.model)) providerModels.push(parsed.model);
    }
    return map;
  }

  async function finish() {
    if (finishing) return;
    if (setupConflict) return;
    if (!setupBaseline) {
      saveError = 'Could not load your settings. Reload setup and try again.';
      return;
    }
    finishing = true;
    saveError = '';
    const readiness = activeAppleCleanup && aiCleanupSelected
      ? appleCleanupReadiness(activeAppleCleanup, aiCleanupSelected, await refreshAppleIntelligence())
      : appleReadiness;
    if (!readiness.ready) {
      saveError = readiness.message;
      finishing = false;
      return;
    }
    const target = effectiveTarget;
    const providerDefaultTranscription = defaultModels.transcriptionDefaultModel;
    const providerDefaultCleanup = defaultModels.cleanupDefaultModel;

    const transcriptionDefaultModel = target?.transcriptionDefaultModel ?? providerDefaultTranscription;
    const cleanupDefaultModel = target?.cleanupDefaultModel ?? providerDefaultCleanup;
    const transcriptionProvider = splitModelId(transcriptionDefaultModel)?.provider ?? provider;
    const cleanupProvider = splitModelId(cleanupDefaultModel)?.provider ?? provider;

    const cleanupEnabled = effectiveCleanupEnabled;
    const silenceOtherAudio = !usesHeadphones;

    try {
      const values: Record<string, unknown> = {
        cleanup_intensity: cleanupIntensity, default_tone: tone, cleanup_enabled: cleanupEnabled,
        transcription_provider: transcriptionProvider, transcription_model: transcriptionDefaultModel,
        transcription_default_model: transcriptionDefaultModel,
        transcription_models_by_provider: modelsByProvider(transcriptionDefaultModel, ...(target?.transcriptionFallbacks ?? [])),
        transcription_fallback_models: target?.transcriptionFallbacks ?? [],
        dual_transcription_enabled: target?.dualTranscription ?? false, transcription_language: language,
        cleanup_provider: cleanupProvider, cleanup_model: cleanupDefaultModel, cleanup_default_model: cleanupDefaultModel,
        cleanup_models_by_provider: cleanupModelsByProvider(cleanupDefaultModel, ...(target?.cleanupFallbacks ?? [])),
        cleanup_fallback_models: cleanupEnabled ? (target?.cleanupFallbacks ?? []) : [],
        appearance_mode: setupBaseline.appearance_mode ?? SETUP_APPEARANCE_MODE, mute_audio: silenceOtherAudio,
        pause_media_during_dictation: setupBaseline.pause_media_during_dictation ?? silenceOtherAudio,
        noise_reduction: setupBaseline.noise_reduction ?? true,
        contextual_formatting_enabled: setupBaseline.contextual_formatting_enabled ?? true,
        contextual_caps_enabled: setupBaseline.contextual_caps_enabled ?? true,
        auto_spacing_enabled: setupBaseline.auto_spacing_enabled ?? true,
        caps_lock_uppercase_enabled: setupBaseline.caps_lock_uppercase_enabled ?? true,
        auto_learn_enabled: setupBaseline.auto_learn_enabled ?? true,
      };
      if (preserveModels) {
        for (const key of modelKeys) values[key] = setupBaseline[key];
      } else if (preserveProvider) {
        values.transcription_provider = setupBaseline.transcription_provider;
      }
      if (preserveDisabledIntensity) values.cleanup_intensity = setupBaseline.cleanup_intensity;
      ++appStore.cleanupChangeVersion;
      await invoke('save_model_settings_if_unchanged', { expected: setupBaseline, values });
      setupBaseline = values;
      window.dispatchEvent(new Event('verenu:setting-saved'));
      void refreshTranscriptionModel().catch((error) => console.error('Failed to refresh transcription model:', error));
      const version = ++appStore.cleanupChangeVersion;
      const saved = await invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' });
      if (version === appStore.cleanupChangeVersion) appStore.cleanupEnabled = saved ?? true;
      if (!isAndroid) await invoke('initialize_autostart');
    } catch (err) {
      console.error('Failed to save setup settings:', err);
      setupConflict = String(err).includes('changed while you were editing');
      if (setupConflict) localStorage.setItem('verenu-setup-reconcile', '1');
      saveError = setupConflict
        ? 'Settings changed outside setup. Reload setup to review them; your current choices will be discarded.'
        : formatIpcError(err, 'Could not save your setup choices');
      finishing = false;
      return;
    }

    try {
      await saveSetting('setup_complete', true);
      void saveSetting('setup_progress', null).catch(() => {});
      localStorage.removeItem('verenu-setup-reconcile');
      sendSetupEvent('setup_completed');
    } catch (err) {
      console.error('Failed to mark setup complete:', err);
      saveError = 'Your choices were saved, but setup could not be marked complete. Try again.';
      finishing = false;
      return;
    }

    window.dispatchEvent(new Event('verenu:refresh-agent-appearance'));
    appStore.setupComplete = true;
    finishing = false;
  }

  type HeaderInfo = { title: string; subtitle: string; name: string } | null;
  function headerFor(s: number): HeaderInfo {
    if (s === providerStep) return { name: 'Provider', title: 'Choose your AI provider', subtitle: 'This powers both transcription and text cleanup. You can switch anytime in Settings.' };
    if (s === analyticsStep) return { name: 'Analytics', title: 'Choose whether to share product analytics', subtitle: 'This helps us find reliability problems and improve Verenu. You can change it anytime in Settings.' };
    if (s === apiKeyStep) {
      const keyProviderName = providers.find((p) => p.id === keyProvider)?.name ?? keyProvider;
      if (keyProvider === 'local') {
        return { name: 'Local', title: 'Set up private, on-device dictation', subtitle: 'Download the speech model here. Nothing is sent to a cloud provider.' };
      }
      if (apiKeyMode === 'tutorial') {
        return { name: 'API Key', title: `Creating a ${keyProviderName} API key`, subtitle: 'Follow along in your browser, then come back and paste the key.' };
      }
      if (apiKeyMode === 'paste') {
        return { name: 'API Key', title: `Paste your ${keyProviderName} API key`, subtitle: 'Stored in your OS credential manager — it never leaves this machine.' };
      }
      return { name: 'API Key', title: `Connect ${keyProviderName}`, subtitle: 'Verenu needs a key to send audio for transcription.' };
    }
    if (isMac && s === permissionStep) return { name: 'Permissions', title: 'Check your macOS permissions', subtitle: 'Verenu needs these to hear your voice and type for you.' };
    if (isAndroid && s === permissionStep) return { name: 'Permissions', title: 'Grant a few permissions', subtitle: 'Verenu needs these to hear you, show the pill above your keyboard, and keep recordings alive.' };
    if (s === modelsStep) return { name: 'Models', title: 'Speed or accuracy?', subtitle: provider === 'local' ? 'Choose speech recognition only, or add optional cleanup models.' : 'Pick the balance you want. Each option sets transcription and optional cleanup models for you.' };
    if (s === writingStyleStep) return { name: 'Writing Style', title: 'How should your dictation sound?', subtitle: 'Cleanup applies to every dictation; tone applies only to AI cleanup.' };
    if (s === languageStep) return { name: 'Language', title: 'What language will you dictate in?', subtitle: "This is the language Verenu expects to hear. The app's own interface stays in English." };
    if (s === audioEnvStep) return { name: 'Audio', title: 'Headphones or speakers?', subtitle: 'This decides whether Verenu needs to silence your other audio while you dictate.' };
    if (s === onboardingTryItStep) return { name: 'Try It', title: 'Give it a try', subtitle: 'Test the full pipeline, end to end, before you go.' };
    return null;
  }

  type ActionBarConfig = {
    leftLabel: string | null;
    leftDisabled: boolean;
    onLeft: () => void;
    rightLabel: string;
    rightDisabled: boolean;
    rightLg: boolean;
    rightGlow: boolean;
    onRight: () => void;
  };

  function bar(partial: Partial<ActionBarConfig> & { rightLabel: string; onRight: () => void }): ActionBarConfig {
    return {
      leftLabel: null,
      leftDisabled: false,
      onLeft: skip,
      rightDisabled: false,
      rightLg: false,
      rightGlow: false,
      ...partial,
    };
  }

  let actionBar = $derived.by((): ActionBarConfig => {
    if (step === 0) return bar({ rightLabel: 'Get Started', rightLg: true, onRight: goNext });
    if (step === onboardingDoneStep) return bar({ rightLabel: finishing ? 'Saving…' : 'Finish setup', rightLg: true, rightDisabled: finishing, onRight: finish });
    if (step === analyticsStep) return bar({ rightLabel: 'Next', onRight: goNext });
    if (step === providerStep) return bar({ rightLabel: 'Next', onRight: goNext });
    if (step === apiKeyStep) {
      if (keyProvider === 'local') return bar({ rightLabel: 'Continue', onRight: goNext });
      if (keySaving) return bar({ leftLabel: 'Skip for now', rightLabel: 'Saving…', rightDisabled: true, onRight: () => {} });
      if (apiKeyDraft.trim()) return bar({ leftLabel: 'Skip for now', rightLabel: 'Save Key', onRight: saveKey });
      if (apiKeyMode === 'fork' && !keySaved) return bar({ rightLabel: "I'll add it later", onRight: goNext });
      if (apiKeyMode === 'paste') return bar({ leftLabel: 'Skip for now', rightLabel: 'Continue', onRight: goNext });
      return bar({ leftLabel: 'Skip for now', rightLabel: 'Continue', onRight: goNext });
    }
    if (isMac && step === permissionStep) {
      return bar({
        rightLabel: allCoreGranted ? 'Next' : 'Grant permissions to continue',
        rightDisabled: !allCoreGranted,
        rightGlow: allCoreGranted,
        onRight: goNext,
      });
    }
    if (isAndroid && step === permissionStep) {
      return bar({
        rightLabel: allCoreGranted ? 'Next' : 'Grant permissions to continue',
        rightDisabled: !allCoreGranted,
        rightGlow: allCoreGranted,
        onRight: goNext,
      });
    }
    if (step === modelsStep) {
      const localNeedsChoice = provider === 'local' && modelPreset === null && !preserveModels;
      return bar({
        rightLabel: localNeedsChoice ? 'Choose a setup' : 'Next',
        rightDisabled: localNeedsChoice,
        onRight: goNext,
      });
    }
    if (step === onboardingTryItStep) return bar({ leftLabel: 'Skip for now', rightLabel: 'Next', onRight: goNext });
    return bar({ rightLabel: 'Next', onRight: goNext });
  });

  const canGoBack = $derived(step > 0 && step <= onboardingTotalSteps);

  $effect(() => {
    step;
    if (typeof document === 'undefined') return;
    requestAnimationFrame(() => {
      const content = stepWrapEl;
      if (!content?.isConnected) return;
      if (content.contains(document.activeElement)) return;
      const heading = content.querySelector<HTMLElement>('.settings-h, .settings-subhead, h1, h2');
      const target = heading ?? firstFocusableInStep(content);
      if (heading && !heading.hasAttribute('tabindex')) heading.setAttribute('tabindex', '-1');
      if (target instanceof HTMLElement && content.contains(target)) {
        target.focus({ preventScroll: true });
      }
    });
  });

  function firstFocusableInStep(container: HTMLElement): HTMLElement | null {
    const selector = [
      'button:not([disabled])',
      'input:not([disabled])',
      'select:not([disabled])',
      'textarea:not([disabled])',
      'a[href]',
      '[tabindex]:not([tabindex="-1"])',
    ].join(',');
    return Array.from(container.querySelectorAll<HTMLElement>(selector))
      .find((el) => !el.hasAttribute('inert') && el.offsetParent !== null) ?? null;
  }
</script>

<SetupShell
  {step}
  totalSteps={onboardingTotalSteps}
  header={headerFor(step)}
  onDotClick={jumpToStep}
>
  {#snippet left()}
    {#if canGoBack}
      <button class="btn-back ui-focus-ring" onclick={goBack} disabled={animating}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6"/></svg>
        Back
      </button>
    {/if}
    {#if actionBar.leftLabel}
      <button class="btn-skip ui-focus-ring" onclick={actionBar.onLeft} disabled={actionBar.leftDisabled || animating}>{actionBar.leftLabel}</button>
    {/if}
  {/snippet}

  {#snippet right()}
    {#if saveError}
      <span class="setup-save-error" role="alert">{saveError}</span>
    {/if}
    {#if setupConflict}
      <button class="btn-primary ui-focus-ring" onclick={() => { localStorage.setItem('verenu-setup-reconcile', '1'); window.location.reload(); }}>Reload setup</button>
    {:else}
      <button
        class="btn-primary ui-focus-ring"
        class:btn-lg={actionBar.rightLg}
        class:btn-primary--glow={actionBar.rightGlow}
        onclick={actionBar.onRight}
        disabled={actionBar.rightDisabled || animating}
      >{actionBar.rightLabel}</button>
    {/if}
  {/snippet}

  {#key step}
  <div class="step-wrap" bind:this={stepWrapEl} in:pageSwap={stepInParams} out:pageSwap={stepOutParams}>
    {#if step === 0}
      <IntroStep />
    {:else if step === analyticsStep}
      <AnalyticsStep enabled={analyticsEnabled} onchange={handleAnalytics} bind:error={analyticsError} />
    {:else if step === providerStep}
      <ProviderStep
        bind:provider
        onSelect={() => { preserveModels = false; preserveProvider = false; modelPreset = null; providerExplicitlySelected = true; }}
        localSupported={localAiSupported}
        localUnsupportedReason={localAiUnsupportedReason}
      />
    {:else if step === apiKeyStep}
      <ApiKeyStep
        provider={keyProvider}
        providerName={providers.find((p) => p.id === keyProvider)?.name ?? keyProvider}
        bind:apiKeyDraft
        bind:showKey
        bind:mode={apiKeyMode}
        {keySaved}
        {keySaving}
        {keyError}
        {keyValidation}
      />
    {:else if isMac && step === permissionStep}
      <PermissionsStep {provider} bind:allCoreGranted />
    {:else if isAndroid && step === permissionStep}
      <AndroidPermissionsStep bind:allCoreGranted />
    {:else if step === modelsStep}
      <ModelsStep
        {provider}
        apiKeyStatus={presetKeyStatus}
        bind:preset={modelPreset}
        onSelect={() => { preserveModels = false; preserveProvider = false; }}
        bind:appleCleanup={appleCleanupChoice}
        cleanupRequested={aiCleanupSelected}
        basicCleanup={cleanupIntensity === 'rules'}
        onOpenApiKeys={() => jumpToStep(apiKeyStep)}
        onChooseCloudProvider={chooseCloudProviderFromModels}
      />
    {:else if step === writingStyleStep}
      <WritingStyleStep bind:intensity={cleanupIntensity} bind:tone onIntensitySelect={() => { preserveDisabledIntensity = false; }} />
    {:else if step === languageStep}
      <LanguageStep bind:language />
    {:else if step === audioEnvStep}
      <AudioEnvironmentStep bind:usesHeadphones />
    {:else if step === onboardingTryItStep}
      <TryItStep />
    {:else if step === onboardingDoneStep}
      <DoneStep
        providerName={providers.find((p) => p.id === doneProvider)?.name ?? doneProvider}
        cleanupName={effectiveCleanupName}
        toneName={cleanupIntensity === 'rules' ? 'No AI tone' : toneName}
        {languageLabel}
        {usesHeadphones}
        hasKey={doneHasKey}
        modelsReady={doneModelReadiness.ready && appleReadiness.ready}
        modelReadinessMessage={appleReadiness.ready ? doneModelReadiness.message : appleReadiness.message}
        presetName={preserveModels ? 'Saved model settings' : modelPreset?.name ?? ''}
        onReviewModels={() => jumpToStep(modelsStep >= 0 ? modelsStep : providerStep)}
        modelRecoveryDisabled={animating || finishing}
      />
    {/if}
  </div>
  {/key}
</SetupShell>

<style>
  .step-wrap {
    width: 100%;
    display: flex;
    justify-content: center;
  }

  .setup-save-error {
    font-size: 12px;
    color: var(--danger);
    max-width: 320px;
    line-height: 1.4;
    text-align: right;
  }

</style>
