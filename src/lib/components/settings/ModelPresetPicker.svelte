<script lang="ts">
  import PresetCard from './PresetCard.svelte';
  import { buildPresets, matchActivePreset, modelLabel, type ActiveConfig, type Hardware, type Preset, type PresetOptions } from './modelPresets';
  import type { ProviderId } from '../../settings';

  let {
    apiKeyStatus,
    hardware,
    localSupported,
    activeConfig,
    installedLocal,
    downloadingLocal,
    onApplyPreset,
    onOpenApiKeys,
    onCancelPreset,
    onDeletePreset,
    /** The note points at the Advanced panel, which only exists in Settings. */
    showCustomNote = true,
    options = {},
    onUseFallback,
    onTestLocal,
    testingLocal = false,
    selectionMode = 'manual',
    pendingPresetId = null,
    transformPreset,
  }: {
    showCustomNote?: boolean;
    options?: PresetOptions;
    /** Display-only view of each preset (e.g. onboarding's Apple cleanup override). Actions still receive the original preset. */
    transformPreset?: (preset: Preset) => Preset;
    onUseFallback?: (preset: Preset) => void;
    onTestLocal?: (preset: Preset) => void;
    testingLocal?: boolean;
    /** The preset whose download the user started, if any. */
    pendingPresetId?: string | null;
    selectionMode?: 'manual' | 'fastest' | 'balanced' | 'quality';
    apiKeyStatus: Record<ProviderId, boolean>;
    hardware: Hardware;
    localSupported: boolean;
    activeConfig: ActiveConfig;
    /** Downloaded local model ids, per task. */
    installedLocal: { transcription: string[]; cleanup: string[] };
    /** The local model id currently downloading per task, if any. */
    downloadingLocal: { transcription: string | null; cleanup: string | null };
    onApplyPreset: (preset: Preset) => void;
    onOpenApiKeys: () => void;
    onCancelPreset: (preset: Preset) => void;
    onDeletePreset: (preset: Preset) => void;
  } = $props();

  const originals = $derived(buildPresets(apiKeyStatus, hardware, localSupported, options));
  const presets = $derived(transformPreset ? originals.map(transformPreset) : originals);
  const activeId = $derived(matchActivePreset(presets, activeConfig));
  // Callbacks take the untransformed preset so the chosen state never stores a display-only override.
  function original(preset: Preset): Preset {
    return originals.find(candidate => candidate.id === preset.id) ?? preset;
  }
  const cloudPresets = $derived(presets.filter(preset => !preset.offline));
  const localPresets = $derived(presets.filter(preset => preset.offline));
  const cloudActive = $derived(!activeConfig.transcriptionDefaultModel.startsWith('local/'));
  const offlineSpeech = $derived(activeConfig.transcriptionFallbacks.filter(id => id.startsWith('local/') && installedLocal.transcription.includes(id.slice('local/'.length))));
  const offlineCleanup = $derived(activeConfig.cleanupFallbacks.filter(id => id.startsWith('local/') && installedLocal.cleanup.includes(id.slice('local/'.length)) && options.localCleanupReady !== false));
  // Only surface "Custom" when there's a real, actionable preset list that the
  // current selection simply doesn't match (not the add-key placeholder state).

  // Accordion: one Details panel at a time; any click elsewhere or Escape closes it.
  let openDetailsId = $state<string | null>(null);

  function handleWindowClick(event: MouseEvent) {
    if (openDetailsId === null) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest('.preset-row.open')) openDetailsId = null;
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && openDetailsId !== null && !document.querySelector('[role="dialog"]')) {
      event.stopPropagation();
      openDetailsId = null;
    }
  }

  function downloadMbFor(preset: Preset): number {
    if (!preset.target) return 0;
    let total = 0;
    for (const model of preset.target.requiredLocalModels) {
      if (!installedLocal[model.task]?.includes(model.id)) total += model.sizeMb;
    }
    return total;
  }

  // Presets share models (Fastest and Balanced both use Parakeet), so a model
  // download says nothing about which preset asked for it. Only the preset the
  // user clicked shows as downloading.
  function isDownloading(preset: Preset): boolean {
    return pendingPresetId !== null && (pendingPresetId === preset.id || pendingPresetId === `fallback-${preset.id}`);
  }

  function installedCountFor(preset: Preset): number {
    if (!preset.target) return 0;
    return preset.target.requiredLocalModels.filter(
      (model) => installedLocal[model.task]?.includes(model.id) ?? false,
    ).length;
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleWindowKeydown} />

<div class="preset-grid">
{#each [{ title: 'Cloud', presets: cloudPresets }, { title: 'Local AI', presets: localPresets }] as group}
{#if group.presets.length}
<section class="preset-group" aria-label={`${group.title} models`}>
  <div class="group-head">
    <h3 class="settings-subhead">{group.title}</h3>
    {#if group.title === 'Local AI' && cloudActive}
      <span class="group-status" role="status">{offlineSpeech.length ? 'Offline fallback ready' : 'Offline fallback not prepared'}</span>
    {:else if group.title === 'Local AI'}
      <span class="group-status">Runs on this device</span>
    {/if}
  </div>
  <div class="preset-list">
  {#each group.presets as preset (preset.id)}
    <PresetCard
      {preset}
      active={preset.id === activeId}
      downloadMb={downloadMbFor(preset)}
      downloading={isDownloading(preset)}
      busy={pendingPresetId !== null}
      installedCount={installedCountFor(preset)}
      onSelect={() => onApplyPreset(original(preset))}
      onAddKey={onOpenApiKeys}
      onCancelDownload={() => onCancelPreset(original(preset))}
      onDeleteModels={() => onDeletePreset(original(preset))}
      onUseFallback={preset.offline && cloudActive && onUseFallback ? () => onUseFallback(original(preset)) : undefined}
      onTestLocal={preset.offline && onTestLocal && downloadMbFor(preset) === 0 && (!preset.target?.cleanupEnabled || options.localCleanupReady !== false) ? () => onTestLocal(original(preset)) : undefined}
      {testingLocal}
      fallbackActive={preset.offline && offlineSpeech.includes(preset.target?.transcriptionDefaultModel ?? '') && (!preset.target?.cleanupEnabled || offlineCleanup.includes(preset.target.cleanupDefaultModel ?? '') || activeConfig.cleanupDefaultModel === preset.target.cleanupDefaultModel)}
      performance={options.performance}
      detailsOpen={openDetailsId === preset.id}
      onToggleDetails={() => (openDetailsId = openDetailsId === preset.id ? null : preset.id)}
    />
  {/each}
  </div>
</section>
{/if}
{/each}
{#if !localPresets.length && localSupported}
  <p class="empty-note">No local preset supports the selected language on this device.</p>
{/if}
</div>

<style>
  .preset-grid { display: flex; flex-direction: column; gap: 0; margin-bottom: 6px; }
  .preset-group { margin-top: 14px; animation: group-in var(--ui-duration-base) var(--ui-ease-out) both; }
  .preset-group:nth-of-type(2) { animation-delay: 60ms; }
  .preset-group:last-of-type { margin-bottom: 14px; }
  .group-head { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin-bottom: 6px; }
  .group-head :global(.settings-subhead) { margin: 0; }
  .group-status { font-family: var(--sans); font-size: 11.5px; color: var(--ink-mute); }
  .preset-list {
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--bg-elev);
    overflow: hidden;
  }
  .empty-note { margin: 10px 0 0; font-family: var(--sans); font-size: 12px; color: var(--ink-mute); }
  @keyframes group-in { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { .preset-group { animation: none; } }
</style>
