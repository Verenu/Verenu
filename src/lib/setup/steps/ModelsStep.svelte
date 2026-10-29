<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import type { ProviderId } from '../../settings';
  import ModelPresetPicker from '../../components/settings/ModelPresetPicker.svelte';
  import {
    buildPresets,
    eligibleSystemSpeechId,
    getHardware,
    type ActiveConfig,
    type Hardware,
    type Preset,
  } from '../../components/settings/modelPresets';
  import {
    localSttStore,
    refreshLocalModels,
    refreshLocalState,
  } from '../../localSttStore.svelte';
  import {
    localLlmStore,
    refreshLocalLlmModels,
    refreshLocalLlmRuntimeInfo,
    refreshLocalLlmState,
  } from '../../localLlmStore.svelte';
  import { localModelDownloads } from '../../components/settings/localModelDownloads';
  import { reconcilePresetSelection } from '../presetSelection';
  import Toggle from '../../components/Toggle.svelte';
  import { appleIntelligence, refreshAppleIntelligence } from '../../appleIntelligence.svelte';
  import { appleCleanupOffer, appleCleanupPanel, applyAppleCleanup, missingLocalModels } from '../appleCleanup';
  import { getProviderLogo } from '../ProviderLogos';

  let {
    provider,
    apiKeyStatus,
    preset = $bindable(),
    appleCleanup = $bindable(false),
    cleanupRequested = true,
    basicCleanup = false,
    onOpenApiKeys,
    onChooseCloudProvider,
    onSelect,
  }: {
    provider: ProviderId;
    apiKeyStatus: Record<ProviderId, boolean>;
    preset: Preset | null;
    appleCleanup?: boolean;
    cleanupRequested?: boolean;
    basicCleanup?: boolean;
    onOpenApiKeys: () => void;
    onChooseCloudProvider: (localSupport: 'unsupported' | 'unknown') => void;
    onSelect?: () => void;
  } = $props();

  let hardware = $state<Hardware>({ totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: true });
  let platformLocalSupport = $state<'checking' | 'supported' | 'unsupported' | 'unknown'>('checking');
  let speechListSettled = $state(false);
  const localSupported = $derived(platformLocalSupport === 'supported' && provider === 'local');

  const presetOptions = $derived({
    includeTranscriptionOnly: provider === 'local',
    localOnly: provider === 'local',
    localModels: provider === 'local' ? localSttStore.models : undefined,
  });
  const systemSpeechOnly = $derived(
    provider === 'local' && platformLocalSupport === 'unsupported' && eligibleSystemSpeechId(presetOptions) !== null,
  );
  const presets = $derived(buildPresets(apiKeyStatus, hardware, localSupported, presetOptions));

  const installedLocal = $derived({
    transcription: localSttStore.models.filter((m) => m.is_downloaded).map((m) => m.id),
    cleanup: localLlmStore.models.filter((m) => m.is_downloaded).map((m) => m.id),
  });

  const downloadingLocal = $derived({
    transcription: localSttStore.state.downloading_model_id ?? null,
    cleanup: localLlmStore.state.downloading_model_id ?? null,
  });

  const appleActive = $derived(appleCleanup && cleanupRequested);
  const effectiveTarget = $derived(applyAppleCleanup(preset?.target, appleActive));
  const activeConfig = $derived<ActiveConfig>({
    transcriptionDefaultModel: effectiveTarget?.transcriptionDefaultModel ?? '',
    cleanupEnabled: effectiveTarget?.cleanupEnabled ?? false,
    cleanupDefaultModel: effectiveTarget?.cleanupDefaultModel ?? '',
    dualTranscription: effectiveTarget?.dualTranscription ?? false,
    transcriptionFallbacks: effectiveTarget?.transcriptionFallbacks ?? [],
    cleanupFallbacks: effectiveTarget?.cleanupFallbacks ?? [],
  });
  function applyApplePreset(candidate: Preset): Preset {
    return appleActive ? { ...candidate, target: applyAppleCleanup(candidate.target, true) } : candidate;
  }

  function needsDownload(candidate: Preset): boolean {
    return missingLocalModels(candidate.target, installedLocal, appleActive).length > 0;
  }

  let userPicked = $state(false);
  $effect(() => {
    if (provider === 'local' && platformLocalSupport === 'checking') return;
    const currentPreset = preset;
    const available = presets.filter((p) => p.kind === 'preset');
    if (currentPreset) {
      const reconciled = reconcilePresetSelection(currentPreset, available);
      if (reconciled !== currentPreset) {
        preset = reconciled;
        if (!reconciled) userPicked = false;
      }
    }
    if (userPicked) return;
    const list = available.filter((p) => p.id !== 'local-transcription-only' && !needsDownload(p));
    if (list.length === 0) return;
    if (preset) return;
    preset = list.find((p) => p.id.endsWith('-balanced')) ?? list[0];
  });

  const appleOffer = $derived(appleCleanupOffer(appleIntelligence.status));
  const applePanel = $derived(appleCleanupPanel(appleIntelligence.status, appleCleanup));
  let appleRefreshing = $state(false);
  async function recheckApple() {
    appleRefreshing = true;
    try { await refreshAppleIntelligence(); } finally { appleRefreshing = false; }
  }

  const downloading = $derived(
    (effectiveTarget?.requiredLocalModels ?? []).some((m) => downloadingLocal[m.task] === m.id),
  );

  function choose(next: Preset) {
    if (!next.target) return;
    onSelect?.();
    userPicked = true;
    preset = next;
    for (const model of missingLocalModels(next.target, installedLocal, appleActive)) {
      localModelDownloads[model.task].download(model.id).catch((err) => console.error('setup preset download failed', err));
    }
  }

  function cardModels(candidate: Preset) {
    return applyAppleCleanup(candidate.target, appleActive)?.requiredLocalModels ?? [];
  }

  function cancel(target: Preset) {
    for (const model of cardModels(target)) {
      localModelDownloads[model.task].cancel(model.id).catch((err) => console.error('setup preset cancel failed', err));
    }
  }

  function remove(target: Preset) {
    for (const model of cardModels(target)) {
      if (!installedLocal[model.task]?.includes(model.id)) continue;
      localModelDownloads[model.task].delete(model.id).catch((err) => console.error('setup preset delete failed', err));
    }
  }

  async function checkPlatformLocalSupport() {
    platformLocalSupport = 'checking';
    try {
      const supported = await invoke<boolean>('local_models_supported_on_this_platform');
      platformLocalSupport = supported === true ? 'supported' : supported === false ? 'unsupported' : 'unknown';
    } catch {
      platformLocalSupport = 'unknown';
    }
  }

  onMount(() => {
    refreshLocalModels().catch(() => {}).finally(() => { speechListSettled = true; });
    refreshLocalState().catch(() => {});
    refreshLocalLlmModels().catch(() => {});
    refreshLocalLlmState().catch(() => {});
    refreshLocalLlmRuntimeInfo().catch(() => {});
    void checkPlatformLocalSupport();
    void refreshAppleIntelligence();
    getHardware().then((hw) => { hardware = hw; }).catch(() => {});
  });
</script>

<div class="step models-step">
  {#if provider === 'local' && (platformLocalSupport === 'checking' || (platformLocalSupport === 'unsupported' && !speechListSettled))}
    <p class="models-note" role="status">Checking whether on-device models are available…</p>
  {:else if provider === 'local' && platformLocalSupport === 'unsupported' && !systemSpeechOnly}
    <div class="local-support-recovery" data-support="unsupported" role="note">
      <p>On-device models are not available on Intel Macs yet. They have not been tested on Intel hardware. Choose a cloud provider to continue.</p>
      <button class="btn-primary" type="button" onclick={() => onChooseCloudProvider('unsupported')}>Choose a cloud provider</button>
    </div>
  {:else if provider === 'local' && platformLocalSupport === 'unknown'}
    <div class="local-support-recovery" data-support="unknown" role="group" aria-label="On-device model availability">
      <p role="alert">Could not confirm whether on-device models are available. Retry the check or choose a cloud provider.</p>
      <div class="local-support-actions">
        <button class="btn-ghost btn-compact" type="button" onclick={checkPlatformLocalSupport}>Retry check</button>
        <button class="btn-primary" type="button" onclick={() => onChooseCloudProvider('unknown')}>Choose a cloud provider</button>
      </div>
    </div>
  {:else}
    <div class="models-picker">
      <ModelPresetPicker
        {apiKeyStatus}
        {hardware}
        {localSupported}
        {activeConfig}
        {installedLocal}
        {downloadingLocal}
        onApplyPreset={choose}
        onOpenApiKeys={onOpenApiKeys}
        onCancelPreset={cancel}
        onDeletePreset={remove}
        showCustomNote={false}
        options={presetOptions}
        transformPreset={applyApplePreset}
      />
    </div>

    {#if applePanel === 'recovery'}
      <div class="apple-cleanup apple-recovery" role="group" aria-labelledby="apple-recovery-title" data-apple-panel="recovery">
        <div class="apple-info">
          <span class="apple-title" id="apple-recovery-title">Apple Intelligence cleanup is selected</span>
          <p class="apple-desc apple-warn" role="alert">
            {#if cleanupRequested}
              This Mac cannot confirm Apple Intelligence right now. Setup cannot finish until you turn the selected cleanup off.
            {:else if basicCleanup}
              Basic cleanup runs on this device, so Apple Intelligence is not used. Turn the selected cleanup off to clear it.
            {:else}
              Cleanup is off, so Apple Intelligence is not used. Turn the selected cleanup off to clear it.
            {/if}
          </p>
          <div class="apple-actions">
            {#if cleanupRequested}
              <button class="btn-ghost btn-compact" type="button" onclick={recheckApple} disabled={appleRefreshing}>
                {appleRefreshing ? 'Checking…' : 'Check again'}
              </button>
            {/if}
            <button class="btn-ghost btn-compact" type="button" onclick={() => { appleCleanup = false; onSelect?.(); }}>Turn off selected cleanup</button>
          </div>
        </div>
      </div>
    {:else if applePanel === 'offer'}
      <div class="apple-cleanup" class:on={appleActive} role="group" aria-labelledby="apple-cleanup-title">
        <div class="apple-icon">{@html getProviderLogo('apple-intelligence')}</div>
        <div class="apple-info">
          <span class="apple-title" id="apple-cleanup-title">Clean up with Apple Intelligence</span>
          <p class="apple-desc">
            {#if !cleanupRequested && basicCleanup}
              Basic cleanup runs on this device, so Apple Intelligence is not used. Your choice is kept if you switch back to AI cleanup.
            {:else if !cleanupRequested}
              Cleanup is off, so Apple Intelligence is not used. Your choice is kept if you turn cleanup back on.
            {:else if appleOffer.selectable}
              Cleanup runs on this Mac with no API key or model download. Speech recognition stays as chosen above.
            {:else}
              {appleOffer.reason || 'Apple Intelligence is not ready on this Mac yet.'}
            {/if}
          </p>
          {#if appleActive && !appleOffer.selectable}
            <p class="apple-desc apple-warn" role="alert">Selected, but not ready. Setup cannot finish until it is ready or turned off.</p>
          {/if}
          {#if !appleOffer.selectable && cleanupRequested}
            <button class="btn-ghost btn-compact apple-recheck" type="button" onclick={recheckApple} disabled={appleRefreshing}>
              {appleRefreshing ? 'Checking…' : 'Check again'}
            </button>
          {/if}
        </div>
        <Toggle
          checked={appleCleanup}
          disabled={!appleOffer.selectable && !appleCleanup}
          label="Clean up with Apple Intelligence"
          onchange={(value) => { appleCleanup = value; onSelect?.(); }}
        />
      </div>
    {/if}

    <p class="models-note">
      {#if downloading}
        Downloading in the background — keep going. Dictation starts working once it finishes.
      {:else}
        {provider === 'local' ? 'Speech only needs no cleanup model or engine. Cleanup bundles are optional and include additional downloads.' : 'Change this anytime in Settings → Models, where you can also pick individual models.'}
      {/if}
    </p>
  {/if}
</div>

<style>
  .models-step { gap: 12px; }

  .local-support-recovery { display: flex; flex-direction: column; align-items: flex-start; gap: 12px; }
  .local-support-recovery p { margin: 0; font-size: 13px; color: var(--ink-mute); line-height: 1.5; }
  .local-support-actions { display: flex; flex-wrap: wrap; gap: 8px; }

  .models-picker {
    container-type: inline-size;
    container-name: settings-panel;
  }

  .models-picker :global(.preset-grid) { gap: 7px; margin-bottom: 0; }
  .models-picker :global(.preset-content) { padding: 9px 14px; gap: 14px; }
  .models-picker :global(.preset-info) { padding: 9px 14px; gap: 14px; }
  .models-picker :global(.preset-side) { width: 156px; gap: 7px; }
  .models-picker :global(.preset-name) { font-size: 15px; }
  .models-picker :global(.preset-tagline) { font-size: 12px; }
  .models-picker :global(.preset-action-btn) { min-height: 26px; padding: 4px 10px; }
  .models-picker :global(.preset-card) { min-height: 72px; border-radius: var(--setup-card-radius); }

  @media (max-height: 660px) {
    .models-note { display: none; }
    .models-picker :global(.preset-grid) { gap: 6px; }
    .models-picker :global(.preset-content) { padding: 7px 12px; }
  }

  .apple-cleanup {
    display: flex;
    align-items: center;
    gap: 13px;
    padding: 11px 14px;
    background: var(--bg-elev);
    border: 1.5px solid var(--line);
    border-radius: var(--setup-card-radius);
    transition:
      border-color var(--ui-duration-fast) var(--ui-ease-out),
      background-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .apple-cleanup.on { border-color: var(--accent); background: var(--accent-soft); }
  .apple-icon { width: 30px; height: 30px; flex-shrink: 0; }
  .apple-icon :global(svg) { width: 100%; height: 100%; }
  .apple-info { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 3px; }
  .apple-title { font-size: 14px; font-weight: 500; color: var(--ink-strong); }
  .apple-desc { margin: 0; font-size: 12px; color: var(--ink-mute); line-height: 1.45; }
  .apple-warn { color: var(--danger); }
  .apple-recheck { margin-top: 4px; }
  .apple-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 4px; }

  .models-note {
    margin: 0;
    font-size: 11.5px;
    color: var(--ink-faint);
    line-height: 1.5;
    text-align: center;
  }
</style>
