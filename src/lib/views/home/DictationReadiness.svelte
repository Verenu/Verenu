<script lang="ts">
  import { refreshAppleIntelligence } from '../../appleIntelligence.svelte';
  import { onMount } from 'svelte';
  import { invoke, listen, type LocalSttModelInfo, type LocalLlmModelInfo, type LocalLlmRuntimeInfo } from '../../tauri';
  import { appStore } from '../../stores';
  import { dictationReadiness, hasCleanupIntensityOverride, hasCloudSpeechCandidate, hasReachableOffCleanupOverride, hasReadyOfflineSpeech, readinessModel, type ReadinessCustomProvider, type ReadinessIssue, type ReadinessInput } from '../../dictationReadiness';
  import { openSetupSettings } from '../../settingsNavigation';
  import type { AppMapping } from '../../appMappings';
  import type { Context, ContextTarget, ContextWebsiteTarget, ContextSubApp } from '../../stores';
  import { listenForHomeReadinessSyncChanges } from './readinessSync';

  let { onchange }: { onchange: (incomplete: boolean) => void } = $props();
  let issues = $state<ReadinessIssue[]>([]);
  let checking = $state(true);
  let failed = $state(false);
  let cloudSpeechConfigured = $state(false);
  let offlineSpeechReady = $state(false);
  let sequence = 0;
  let mounted = false;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  const connectionWarning = $derived(
    !checking && !failed && !issues.some(issue => issue.task === 'transcription') &&
    cloudSpeechConfigured && !offlineSpeechReady && !appStore.isOnline,
  );

  async function refresh(isOnline = appStore.isOnline) {
    const current = ++sequence;
    try {
      const setting = <T,>(key: string) => invoke<T | null>('get_setting', { key });
      const [
        contexts, targets, websites, subApps, appMappings, speech, cleanup, speechProvider, cleanupProvider, legacySpeech, legacyCleanup,
        speechFallbacks, cleanupFallbacks, enabled, intensity, dualTranscriptionEnabled, keys, customProviders,
        speechModels, cleanupModels, engine,
        appleIntelligence,
      ] = await Promise.all([
        invoke<Pick<Context, 'id' | 'is_everywhere' | 'cleanup_intensity'>[]>('get_contexts'),
        invoke<ContextTarget[]>('get_context_targets', { contextId: null }),
        invoke<ContextWebsiteTarget[]>('get_context_websites', { contextId: null }),
        invoke<ContextSubApp[]>('get_sub_apps'),
        setting<AppMapping[]>('app_mappings'),
        setting<string>('transcription_default_model'),
        setting<string>('cleanup_default_model'),
        setting<string>('transcription_provider'),
        setting<string>('cleanup_provider'),
        setting<string>('transcription_model'),
        setting<string>('cleanup_model'),
        setting<string[]>('transcription_fallback_models'),
        setting<string[]>('cleanup_fallback_models'),
        setting<boolean>('cleanup_enabled'),
        setting<string>('cleanup_intensity'),
        setting<boolean>('dual_transcription_enabled'),
        invoke<Record<string, boolean>>('get_api_key_status'),
        setting<ReadinessCustomProvider[]>('custom_providers'),
        invoke<LocalSttModelInfo[]>('list_local_stt_models'),
        invoke<LocalLlmModelInfo[]>('list_local_llm_models'),
        invoke<LocalLlmRuntimeInfo>('get_local_llm_runtime_info'),
        refreshAppleIntelligence(),
      ]);
      if (!mounted || current !== sequence) return;
      const cleanupAssignments = [...(targets ?? []), ...(websites ?? []), ...(subApps ?? [])];
      const input: ReadinessInput = {
        transcriptionModel: readinessModel('transcription', speech, legacySpeech, speechProvider),
        cleanupModel: readinessModel('cleanup', cleanup, legacyCleanup, cleanupProvider),
        transcriptionFallbacks: speechFallbacks ?? [],
        cleanupFallbacks: cleanupFallbacks ?? [],
        cleanupEnabled: enabled ?? true,
        cleanupIntensity: intensity,
        cleanupIntensityOverrideMayBeUsed: hasCleanupIntensityOverride(contexts ?? [], appMappings ?? [], cleanupAssignments),
        cleanupOffContextMayBeUsed: hasReachableOffCleanupOverride(contexts ?? [], appMappings ?? [], cleanupAssignments),
        dualTranscriptionEnabled: dualTranscriptionEnabled ?? false,
        isOnline,
        keys,
        customProviders: customProviders ?? [],
        speechModels,
        cleanupModels,
        cleanupEngineInstalled: engine.installed,
        appleIntelligence,
      };
      issues = dictationReadiness(input);
      cloudSpeechConfigured = hasCloudSpeechCandidate(input);
      offlineSpeechReady = hasReadyOfflineSpeech(input);
      failed = false;
    } catch {
      if (!mounted || current !== sequence) return;
      failed = true;
    }
    checking = false;
  }

  // Home remains mounted behind Settings. Refresh after it closes and after a
  // setting, credential, imported backup, or local model changes.
  $effect(() => {
    const isOnline = appStore.isOnline;
    if (!appStore.settingsOpen && mounted) void refresh(isOnline);
  });
  $effect(() => {
    onchange(checking || failed || issues.length > 0 || connectionWarning);
  });

  onMount(() => {
    mounted = true;
    void refresh();
    const browserEvents = ['focus', 'verenu:api-key-saved', 'verenu:api-key-deleted', 'verenu:setting-saved', 'verenu:context-saved'];
    const update = () => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        refreshTimer = null;
        void refresh();
      }, 60);
    };
    browserEvents.forEach(event => window.addEventListener(event, update));
    const unlisteners: (() => void)[] = [];
    const events = [
      'verenu:settings-imported',
      'local-stt-model-download-complete',
      'local-stt-model-deleted',
      'local-llm-model-download-complete',
      'local-llm-model-deleted',
      'local-llm-runtime-download-complete',
      'local-llm-runtime-deleted',
    ];
    for (const event of events) {
      void listen(event, update).then(stop => {
        if (mounted) unlisteners.push(stop);
        else stop();
      });
    }
    unlisteners.push(listenForHomeReadinessSyncChanges(
      (eventName, handler) => listen<{ tables?: string[] }>(eventName, handler),
      update,
    ));
    return () => {
      mounted = false;
      sequence++;
      if (refreshTimer) clearTimeout(refreshTimer);
      browserEvents.forEach(event => window.removeEventListener(event, update));
      unlisteners.forEach(stop => stop());
    };
  });
</script>

{#if checking || failed || issues.length || connectionWarning}
  <div class="readiness-notice" role="status" aria-live="polite">
    {#if checking}
      <p>Checking dictation setup...</p>
    {:else if failed}
      <p>Could not check dictation setup on this device.</p>
      <button class="btn-ghost btn-compact" onclick={() => refresh()}>Check setup again</button>
      <button class="btn-ghost btn-compact" onclick={() => openSetupSettings('models')}>Open model settings</button>
    {:else}
      {#each issues as issue}
        <div class="readiness-row">
          <p>{issue.message}</p>
          <button class="btn-ghost btn-compact" onclick={() => openSetupSettings(issue.section)}>{issue.action}</button>
        </div>
      {/each}
      {#if connectionWarning}
        <div class="readiness-row">
          <p>Cloud speech recognition needs a connection. Reconnect or choose installed local speech or a ready custom endpoint.</p>
          <button class="btn-ghost btn-compact" onclick={() => openSetupSettings('models')}>Open model settings</button>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .readiness-notice { padding: 12px 16px; margin-bottom: 16px; border: 1px solid var(--warning-line); border-radius: var(--r-md); background: var(--warning-bg); }
  .readiness-row { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 12px; }
  .readiness-row + .readiness-row { margin-top: 8px; }
  p { margin: 0; flex: 1 1 240px; font-size: 12.5px; line-height: 1.5; color: var(--ink-soft); }
</style>
