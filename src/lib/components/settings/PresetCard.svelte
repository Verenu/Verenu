<script lang="ts">
  import { modelLabel, type Preset, type ModelPerformance } from './modelPresets';
  import { modalFocusTrap } from '../../modalFocus';
  import { modalBackdrop, modalCard, MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { slide } from 'svelte/transition';
  import { consentBlocked } from '../../legal/consentStore.svelte';
  import { cubicOut } from 'svelte/easing';
  import { isCustomProviderId } from '../../customProviders.svelte';
  import { providerDisplayLabel, splitModelId } from './models';

  let {
    preset,
    active = false,
    downloadMb = 0,
    downloading = false,
    installedCount = 0,
    onSelect,
    onAddKey,
    onCancelDownload,
    onDeleteModels,
    onUseFallback,
    onTestLocal,
    testingLocal = false,
    fallbackActive = false,
    performance = [],
    detailsOpen = false,
    busy = false,
    recommended = false,
    onToggleDetails,
    layout = 'rows',
    index = 0,
  }: {
    /** `cards` lays presets out side by side with the details panel under the whole row. */
    layout?: 'rows' | 'cards';
    index?: number;
    preset: Preset;
    active?: boolean;
    downloadMb?: number;
    downloading?: boolean;
    installedCount?: number;
    onSelect: () => void;
    onAddKey?: () => void;
    onCancelDownload?: () => void;
    onDeleteModels?: () => void;
    onUseFallback?: () => void;
    onTestLocal?: () => void;
    testingLocal?: boolean;
    fallbackActive?: boolean;
    performance?: ModelPerformance[];
    detailsOpen?: boolean;
    busy?: boolean;
    recommended?: boolean;
    onToggleDetails?: () => void;
  } = $props();

  const isAddKey = $derived(preset.kind === 'add-key');
  const needsDownload = $derived(downloadMb > 0);
  const speechChain = $derived(preset.target ? [preset.target.transcriptionDefaultModel, ...preset.target.transcriptionFallbacks] : []);
  const cleanupChain = $derived(preset.target?.cleanupEnabled && preset.target.cleanupDefaultModel ? [preset.target.cleanupDefaultModel, ...preset.target.cleanupFallbacks] : []);
  const measured = $derived(performance.find(sample => sample.id === preset.target?.transcriptionDefaultModel && sample.task === 'transcription' && sample.samples >= 3));

  /** A custom provider's id is a UUID, so name it the way the user did. */
  function chainLabel(id: string): string {
    const parsed = splitModelId(id);
    return parsed && isCustomProviderId(parsed.provider) ? `${providerDisplayLabel(parsed.provider)}: ${parsed.model}` : modelLabel(id);
  }

  function formatSize(mb: number): string {
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }

  const cancelable = $derived(downloading && onCancelDownload != null);
  const manageable = $derived(preset.offline && installedCount > 0 && onDeleteModels != null);
  const actionLabel = $derived(
    downloading ? 'Cancel download' : needsDownload ? `Download ${formatSize(downloadMb)}` : '',
  );

  /** Cloud presets and downloads need accepted terms; the click opens the accept dialog via ModelsSection. */
  const consentDim = $derived(consentBlocked() && (!preset.offline || (needsDownload && !cancelable)));

  let confirmDelete = $state(false);
  const showDetails = $derived(detailsOpen);
  const detailsId = `preset-details-${Math.random().toString(36).slice(2, 9)}`;
  let confirmCancelButton = $state<HTMLButtonElement | null>(null);

  function handleAction(event: MouseEvent) {
    event.stopPropagation();
    if (cancelable) onCancelDownload?.();
    else onSelect();
  }

  function confirmDeleteModels() {
    confirmDelete = false;
    onDeleteModels?.();
  }

  function handleDeleteModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmDelete) confirmDelete = false;
  }
</script>

<svelte:window onkeydown={handleDeleteModalKeydown} />

{#if isAddKey}
  <div class="preset-row preset-info" class:cards={layout === 'cards'}>
    <div class="preset-main">
      <span class="preset-name">{preset.name}</span>
      <span class="preset-tagline" title={preset.tagline}>{preset.tagline}</span>
    </div>
    <button class="btn-ghost btn-compact" type="button" onclick={() => onAddKey?.()}>Open API keys</button>
  </div>
{:else}
  <div class="preset-row" class:cards={layout === 'cards'} class:preset-active={active} class:open={showDetails} class:inert-row={needsDownload} class:consent-dim={consentBlocked() && !preset.offline} style:--i={index}>
    <div class="preset-face">
    <button
      class="preset-select"
      class:consent-dim={consentDim}
      aria-disabled={consentDim || undefined}
      type="button"
      aria-label={`Use ${preset.offline ? 'local ' : ''}${preset.name}`}
      aria-pressed={active}
      disabled={needsDownload || downloading}
      tabindex={needsDownload ? -1 : undefined}
      onclick={() => onSelect()}
    ></button>
    <div class="preset-line">
      <span class="preset-radio" class:on={active} aria-hidden="true">
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 5 5 9-10"/></svg>
      </span>
      <div class="preset-main">
        <span class="preset-name">
          {preset.name}
          {#if recommended}<span class="preset-badge">Recommended</span>{/if}
        </span>
        <span class="preset-tagline" title={preset.tagline}>{preset.tagline}</span>
      </div>
      <div class="preset-trail">
        {#if actionLabel}
          <button class="preset-action" class:danger={cancelable} class:consent-dim={consentDim} aria-disabled={consentDim || undefined} type="button" disabled={busy && !downloading} onclick={handleAction}>{actionLabel}</button>
        {:else if measured}
          <span class="preset-speed" title="Measured on this device">{(measured.latency_ms / 1000).toFixed(1)}s{layout === 'cards' ? ' average' : ''}</span>
        {/if}
        <button
          class="preset-more"
          type="button"
          aria-label="Details"
          aria-expanded={showDetails}
          aria-controls={detailsId}
          onclick={(event) => { event.stopPropagation(); onToggleDetails?.(); }}
        >
          {#if layout === 'cards'}<span class="more-label" aria-hidden="true">Details</span>{/if}
          <svg class="ui-chevron" class:open={showDetails} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
        </button>
      </div>
    </div>
    {#if downloading}<span class="preset-progress" aria-hidden="true"><i></i></span>{/if}
    </div>
    {#if showDetails}
      <div id={detailsId} class="preset-details" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
        {#if layout === 'cards'}<p class="details-title">{preset.offline ? 'Local' : 'Cloud'} {preset.name}</p>{/if}
        <dl>
          <dt>Speech</dt><dd>{speechChain.map(chainLabel).join(' → ')}</dd>
          <dt>Cleanup</dt><dd>{cleanupChain.length ? cleanupChain.map(chainLabel).join(' → ') : 'Off'}</dd>
          <dt>Mode</dt><dd>{preset.offline ? 'On-device only' : 'Cloud first, local fallback'} · {preset.target?.dualTranscription ? 'two speech models cross-check' : 'one speech model'}</dd>
        </dl>
        {#if onUseFallback || onTestLocal || manageable}
          <div class="preset-actions">
            {#if onUseFallback}
              <button class="btn-ghost btn-compact" disabled={downloading || fallbackActive} onclick={onUseFallback}>{fallbackActive ? 'Offline fallback selected' : needsDownload ? 'Prepare offline fallback' : 'Use as offline fallback'}</button>
            {/if}
            {#if onTestLocal}
              <button class="btn-ghost btn-compact" disabled={testingLocal || downloading} onclick={onTestLocal}>{testingLocal ? 'Testing…' : 'Test local speed'}</button>
            {/if}
            {#if manageable}
              <button class="btn-ghost btn-compact" onclick={() => (confirmDelete = true)}>Delete models</button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

{#if confirmDelete}
  <div class="preset-confirm-wrap">
    <button type="button" class="ui-modal-backdrop" aria-label="Close dialog" onclick={() => (confirmDelete = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
    <div
      class="modal-card ui-modal-card"
      use:modalFocusTrap={{
        active: confirmDelete,
        initialFocus: () => confirmCancelButton,
      }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="preset-delete-confirm-title"
      tabindex="-1"
      in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
      out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
    >
      <div class="ui-modal-head">
        <h2 id="preset-delete-confirm-title" class="ui-modal-title">Delete downloaded models?</h2>
      </div>
      <div class="ui-modal-body">
        <p class="ui-modal-copy">
          This removes the downloaded model files for {preset.name} from this machine. Your settings and dictation history are untouched.
        </p>
      </div>
      <div class="ui-modal-foot">
        <div class="ui-modal-actions">
          <button bind:this={confirmCancelButton} class="btn-ghost" onclick={() => (confirmDelete = false)}>Cancel</button>
          <button class="btn-danger btn-compact" onclick={confirmDeleteModels}>Delete models</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .consent-dim { opacity: 0.5; }
  .preset-row {
    position: relative;
    border-bottom: 1px solid var(--line);
    transition: background var(--ui-duration-fast) var(--ui-ease-out);
  }
  .preset-row:last-child { border-bottom: 0; }
  .preset-row:hover:not(.inert-row) { background: var(--control-hover); }
  .preset-row.preset-active { background: var(--control-active); }
  .preset-row:has(.preset-select:active:not(:disabled)) { background: var(--control-active); }
  .preset-select:disabled { pointer-events: none; }

  .preset-face { position: relative; }

  .preset-select {
    position: absolute;
    inset: 0;
    z-index: 1;
    border: 0;
    background: transparent;
    cursor: pointer;
  }
  .preset-select:disabled { cursor: default; }
  .preset-select:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

  .preset-line, .preset-info {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 8px 8px 8px 14px;
    pointer-events: none;
  }
  .preset-info { justify-content: space-between; pointer-events: auto; padding-right: 14px; }

  .preset-radio {
    flex: none;
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 1.5px solid var(--line-strong);
    color: transparent;
    transition: background var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .preset-radio.on { background: var(--ink); border-color: var(--ink); color: var(--bg-elev); transform: scale(1.05); }

  .preset-main {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: var(--preset-name-col, 164px) minmax(0, 1fr);
    align-items: center;
    column-gap: 12px;
  }
  .preset-name {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-family: var(--sans);
    font-size: 13.5px;
    font-weight: 600;
    color: var(--ink);
    white-space: nowrap;
  }
  .preset-badge {
    flex: none;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-ink);
    font-size: 10.5px;
    font-weight: 500;
    line-height: 1.5;
  }
  .preset-tagline {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    font-family: var(--sans);
    font-size: 12px;
    color: var(--ink-mute);
    line-height: 1.4;
  }

  .preset-trail {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    flex: none;
    width: var(--preset-trail-col, 164px);
  }
  .preset-speed { font-family: var(--sans); font-size: 11.5px; color: var(--ink-faint); font-variant-numeric: tabular-nums; padding-right: 2px; }

  .preset-action, .preset-more { pointer-events: auto; position: relative; z-index: 2; }
  .preset-action {
    border: 0;
    background: transparent;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 600;
    color: var(--ink);
    padding: 6px 8px;
    border-radius: 6px;
    cursor: pointer;
    transition: background var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .preset-action:disabled { opacity: 0.45; cursor: default; }
  .preset-action:hover:not(:disabled) { background: var(--paper-2); }
  .preset-action.danger:hover { color: var(--danger); }
  .preset-more {
    display: grid;
    place-items: center;
    flex: none;
    width: 36px;
    height: 36px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-mute);
    cursor: pointer;
    transition: background var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .preset-more:hover { background: var(--paper-2); color: var(--ink); }
  .preset-action:focus-visible, .preset-more:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }

  .preset-details {
    position: relative;
    z-index: 2;
    padding: 0 14px 12px 42px;
    font-family: var(--sans);
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-mute);
  }
  .preset-details dl { display: grid; grid-template-columns: 64px minmax(0, 1fr); gap: 4px 12px; margin: 0; }
  .preset-details dt { color: var(--ink-soft); font-weight: 600; }
  .preset-details dd { margin: 0; overflow-wrap: anywhere; }
  .preset-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }

  .preset-confirm-wrap { position: fixed; inset: 0; z-index: 120; }

  .preset-progress { position: absolute; left: 0; right: 0; bottom: 0; height: 2px; overflow: hidden; pointer-events: none; }
  .preset-progress i { display: block; width: 40%; height: 100%; background: var(--ink); animation: preset-progress 1.4s var(--ui-ease-out) infinite; }
  @keyframes preset-progress { from { transform: translateX(-100%); } to { transform: translateX(250%); } }
  .details-title { margin: 0 0 8px; font-size: 12.5px; font-weight: 600; color: var(--ink); }

  /* Cards: each preset is a tile, and the open details panel spans the row below the tiles. */
  .preset-row.cards { display: contents; }
  .cards .preset-face {
    order: 0;
    display: flex;
    min-width: 0;
    border-radius: var(--r-lg, 14px);
    background: var(--settings-card-bg, var(--bg-elev));
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
    transition: box-shadow var(--ui-duration-base) var(--ui-ease-out), background-color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out);
  }
  .preset-row.cards:hover:not(.inert-row) { background: none; }
  .cards:not(.inert-row):not(.preset-active) .preset-face:hover { box-shadow: inset 0 0 0 1px var(--line-strong); background: var(--control-hover); }
  .cards .preset-face:has(.preset-select:active:not(:disabled)) { transform: scale(0.985); }
  .cards.open .preset-face { box-shadow: inset 0 0 0 1px var(--line-strong); }
  .cards.preset-active .preset-face { box-shadow: inset 0 0 0 1.5px var(--ink); background: var(--settings-card-bg, var(--bg-elev)); }
  .cards .preset-select { border-radius: var(--r-lg, 14px); }
  .cards .preset-line {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas: 'main radio' 'trail trail';
    grid-template-rows: 1fr auto;
    align-items: start;
    gap: 10px 8px;
    min-height: 116px;
    padding: 14px 8px 6px 14px;
  }
  .cards .preset-radio { grid-area: radio; width: 18px; height: 18px; margin: 1px 6px 0 0; }
  .cards .preset-radio.on { animation: preset-check var(--ui-duration-base) var(--ui-ease-out); }
  .cards .preset-radio svg { transition: transform var(--ui-duration-base) var(--ui-ease-out); transform: scale(0.4); }
  .cards .preset-radio.on svg { transform: scale(1); }
  @keyframes preset-check { 0% { transform: scale(0.8); } 60% { transform: scale(1.14); } 100% { transform: scale(1.05); } }
  .cards .preset-main { grid-area: main; grid-template-columns: minmax(0, 1fr); row-gap: 6px; align-items: start; }
  .cards .preset-name { flex-wrap: wrap; row-gap: 4px; font-size: 14px; letter-spacing: -0.01em; }
  .cards .preset-tagline { -webkit-line-clamp: 4; line-clamp: 4; }
  .cards .preset-trail { grid-area: trail; width: auto; justify-content: space-between; min-height: 36px; }
  .cards .preset-action { margin-left: -8px; }
  .cards .preset-more { display: inline-flex; align-items: center; gap: 4px; width: auto; height: 30px; margin-left: auto; padding: 0 6px 0 8px; font-family: var(--sans); font-size: 12px; font-weight: 500; }
  .cards.open .preset-more { color: var(--ink); }
  .cards .preset-speed { color: var(--ink-mute); }
  .cards .preset-details {
    order: 1;
    grid-column: 1 / -1;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: var(--paper-2);
  }
  .preset-info.cards { display: flex; grid-column: 1 / -1; border: 1px dashed var(--line-strong); border-radius: var(--r-lg, 14px); }
  .preset-info.cards .preset-main { grid-template-columns: minmax(0, 1fr); row-gap: 2px; }

  @media (prefers-reduced-motion: reduce) {
    .preset-radio { transition: none; transform: none; }
    .cards .preset-face, .cards .preset-radio svg { transition: none; }
    .cards .preset-radio.on, .preset-progress i { animation: none; }
    .preset-progress i { width: 100%; opacity: 0.4; }
  }

  @container settings-panel (max-width: 560px) {
    .preset-main { grid-template-columns: minmax(0, 1fr); row-gap: 2px; }
    .preset-line, .preset-info { padding: 10px 6px 10px 12px; }
    .preset-line { flex-wrap: wrap; row-gap: 4px; }
    .preset-trail { display: contents; }
    .preset-more { width: 44px; height: 44px; order: 2; }
    .preset-action { order: 3; flex-basis: calc(100% - 28px); margin-left: 28px; text-align: left; padding-left: 0; min-height: 36px; }
    .preset-speed { display: none; }
    .preset-details { padding-left: 12px; }
    .preset-actions :global(.btn-ghost) { min-height: 40px; flex: 1 1 auto; }

    /* One column: each details panel follows its own card. */
    .cards .preset-face { order: calc(var(--i) * 2); }
    .cards .preset-details { order: calc(var(--i) * 2 + 1); padding-left: 14px; }
    .cards .preset-line { min-height: 0; padding: 12px 6px 4px 14px; flex-wrap: nowrap; }
    .cards .preset-trail { display: flex; }
    .cards .preset-more { order: 0; width: auto; height: 40px; }
    .cards .preset-action { order: 0; flex-basis: auto; margin-left: -8px; padding-left: 8px; }
    .cards .preset-speed { display: inline; }
  }
</style>
