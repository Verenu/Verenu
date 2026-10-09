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
    onOpenApiKeys,
    onChooseCloudProvider,
  }: {
    /** The provider chosen earlier. Local presets only appear when it is 'local'. */
    provider: ProviderId;
    apiKeyStatus: Record<ProviderId, boolean>;
    /** The chosen preset. Written to settings by Setup's finish(), not here. */
    preset: Preset | null;
    /** Explicit opt-in to cleanup on Apple Intelligence. Cleanup only; speech is unchanged. */
    appleCleanup?: boolean;
    /** False when cleanup intensity is Off. The Apple choice is kept but not applied, required, or downloaded. */
    cleanupRequested?: boolean;
    onOpenApiKeys: () => void;
    onChooseCloudProvider: (localSupport: 'unsupported' | 'unknown') => void;
  } = $props();

  // Keep local choices hidden until the platform probe confirms support.
  let hardware = $state<Hardware>({ totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: true });
  let platformLocalSupport = $state<'checking' | 'supported' | 'unsupported' | 'unknown'>('checking');
  // Whether the first speech-model listing finished (even if it failed), so a
  // gated platform doesn't flash the unavailable notice before Apple Speech is known.
  let speechListSettled = $state(false);
  // Someone who picked a cloud provider didn't ask for a multi-gigabyte local
  // model; Settings → Models still offers one. Local stays for the local path.
  const localSupported = $derived(platformLocalSupport === 'supported' && provider === 'local');

  const presetOptions = $derived({
    includeTranscriptionOnly: provider === 'local',
    localOnly: provider === 'local',
    localModels: provider === 'local' ? localSttStore.models : undefined,
  });
  // The blanket gate (Intel Macs) stays closed for every downloaded engine, but
  // Apple Speech ships with macOS and is offered on its own once listed.
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

  // The picker highlights whichever card matches this config, so reflecting the
  // selection back through it is what makes the card read as "Selected".
  // Card display and active matching read the Apple target while the opt-in is on.
  // The stored `preset` stays the original, so toggling off restores its cleanup.
  // Apple applies only while cleanup runs; with intensity Off the choice is kept but inert.
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

  // Every download decision reads the Apple override, so an opt-in before choosing
  // never waits on or starts a cleanup model/engine that Apple replaces.
  function needsDownload(candidate: Preset): boolean {
    return missingLocalModels(candidate.target, installedLocal, appleActive).length > 0;
  }

  // Pre-select a sensible middle option so the step has a working answer even if
  // the user just hits Next — but never one that would commit them to a
  // multi-gigabyte download they didn't ask for. If everything needs a download,
  // nothing is pre-selected and the provider defaults stand.
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

  // Hidden unless a supported Mac reports in. An opt-in is never dropped silently:
  // if the Mac stops being ready the choice stays, flagged, until turned off here.
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
    userPicked = true;
    preset = next;
    // Start any missing downloads now so they run while the user finishes the
    // wizard. Unlike the Models tab we don't defer activation — finish() writes
    // the settings minutes later, and the card shows download progress meanwhile.
    for (const model of missingLocalModels(next.target, installedLocal, appleActive)) {
      localModelDownloads[model.task].download(model.id).catch((err) => console.error('setup preset download failed', err));
    }
  }

  // The models the card shows, so explicit cancel/delete never touch a model the
  // card no longer lists (e.g. the cleanup model replaced by Apple Intelligence).
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
            <button class="btn-ghost btn-compact" type="button" onclick={() => { appleCleanup = false; }}>Turn off selected cleanup</button>
          </div>
        </div>
      </div>
    {:else if applePanel === 'offer'}
      <div class="apple-cleanup" class:on={appleActive} role="group" aria-labelledby="apple-cleanup-title">
        <div class="apple-icon">{@html getProviderLogo('apple-intelligence')}</div>
        <div class="apple-info">
          <span class="apple-title" id="apple-cleanup-title">Clean up with Apple Intelligence</span>
          <p class="apple-desc">
            {#if !cleanupRequested}
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
          onchange={(value) => { appleCleanup = value; }}
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

  /* PresetCard's narrow layout is keyed to the settings panel container, which
     doesn't exist here — name the container so the cards still fold on small
     windows instead of overflowing the wizard column. */
  .models-picker {
    container-type: inline-size;
    container-name: settings-panel;
  }

  /* The Settings cards are sized for a scrolling panel. The wizard has a fixed
     height budget and up to four cards, so tighten the vertical rhythm here
     rather than letting the step overflow the action bar. */
  .models-picker :global(.preset-grid) { gap: 7px; margin-bottom: 0; }
  .models-picker :global(.preset-content) { padding: 9px 14px; gap: 14px; }
  .models-picker :global(.preset-info) { padding: 9px 14px; gap: 14px; }
  .models-picker :global(.preset-side) { width: 156px; gap: 7px; }
  .models-picker :global(.preset-name) { font-size: 15px; }
  .models-picker :global(.preset-tagline) { font-size: 12px; }
  .models-picker :global(.preset-action-btn) { min-height: 26px; padding: 4px 10px; }
  .models-picker :global(.preset-card) { min-height: 72px; border-radius: var(--setup-card-radius); }

  /* Short windows: four cards plus a footnote don't fit. The footnote is the
     least load-bearing thing on the step, so it goes first. */
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
