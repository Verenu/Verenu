<script lang="ts">
  import { modelLabel, type Preset, type ModelPerformance } from './modelPresets';
  import { modalFocusTrap } from '../../modalFocus';
  import { modalBackdrop, modalCard, MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';

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
    onToggleDetails,
  }: {
    preset: Preset;
    active?: boolean;
    /** Total MB of required local models not yet on disk (0 = ready to use). */
    downloadMb?: number;
    downloading?: boolean;
    /** How many of this preset's local models are already downloaded. */
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
    /** Another preset is downloading; start nothing new until it finishes. */
    busy?: boolean;
    onToggleDetails?: () => void;
  } = $props();

  const isAddKey = $derived(preset.kind === 'add-key');
  const needsDownload = $derived(downloadMb > 0);
  const speechChain = $derived(preset.target ? [preset.target.transcriptionDefaultModel, ...preset.target.transcriptionFallbacks] : []);
  const cleanupChain = $derived(preset.target?.cleanupEnabled && preset.target.cleanupDefaultModel ? [preset.target.cleanupDefaultModel, ...preset.target.cleanupFallbacks] : []);
  const measured = $derived(performance.find(sample => sample.id === preset.target?.transcriptionDefaultModel && sample.task === 'transcription' && sample.samples >= 3));

  function formatSize(mb: number): string {
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }

  const cancelable = $derived(downloading && onCancelDownload != null);
  const manageable = $derived(preset.offline && installedCount > 0 && onDeleteModels != null);
  const actionLabel = $derived(
    downloading ? 'Cancel download' : needsDownload ? `Download ${formatSize(downloadMb)}` : '',
  );

  // Deleting downloaded models is destructive and irreversible, so it gets the
  // same in-app confirm dialog as the other destructive settings actions
  // instead of a blocking native browser confirm.
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

  // Same Escape contract as the other settings confirm dialogs: dismiss the
  // dialog, never the page beneath it (Settings' own Escape guard yields
  // while [role="dialog"] is present).
  function handleDeleteModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmDelete) confirmDelete = false;
  }
</script>

<svelte:window onkeydown={handleDeleteModalKeydown} />

{#if isAddKey}
  <div class="preset-row preset-info">
    <div class="preset-main">
      <span class="preset-name">{preset.name}</span>
      <span class="preset-tagline">{preset.tagline}</span>
    </div>
    <button class="btn-ghost btn-compact" type="button" onclick={() => onAddKey?.()}>Open API keys</button>
  </div>
{:else}
  <div class="preset-row" class:preset-active={active} class:open={showDetails} class:inert-row={needsDownload}>
    <button
      class="preset-select"
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
        <span class="preset-name">{preset.name}</span>
        <span class="preset-tagline">{preset.tagline}</span>
      </div>
      <div class="preset-trail">
        {#if actionLabel}
          <button class="preset-action" class:danger={cancelable} type="button" disabled={busy && !downloading} onclick={handleAction}>{actionLabel}</button>
        {:else if measured}
          <span class="preset-speed">{(measured.latency_ms / 1000).toFixed(1)}s</span>
        {/if}
        <button
          class="preset-more"
          type="button"
          aria-label="Details"
          aria-expanded={showDetails}
          aria-controls={detailsId}
          onclick={(event) => { event.stopPropagation(); onToggleDetails?.(); }}
        >
          <svg class="ui-chevron" class:open={showDetails} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
        </button>
      </div>
    </div>
    {#if showDetails}
      <div id={detailsId} class="preset-details" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
        <dl>
          <dt>Speech</dt><dd>{speechChain.map(modelLabel).join(' → ')}</dd>
          <dt>Cleanup</dt><dd>{cleanupChain.length ? cleanupChain.map(modelLabel).join(' → ') : 'Off'}</dd>
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
  <!-- Renders above every surface it can open from: settings (z-60), the
       cleanup-prompt modal (z-70), and the setup wizard overlay (z-100). -->
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
    padding: 11px 14px;
    pointer-events: none;
  }
  .preset-info { justify-content: space-between; pointer-events: auto; }

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
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .preset-name { font-family: var(--sans); font-size: 13.5px; font-weight: 600; color: var(--ink); }
  .preset-tagline { font-family: var(--sans); font-size: 12px; color: var(--ink-mute); line-height: 1.4; }

  .preset-trail { display: flex; align-items: center; gap: 4px; flex: none; }
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
    width: 44px;
    height: 40px;
    margin: -8px -8px -8px 0;
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
  .preset-details dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0; }
  .preset-details dt { color: var(--ink-soft); font-weight: 600; }
  .preset-details dd { margin: 0; overflow-wrap: anywhere; }
  .preset-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }

  .preset-confirm-wrap { position: fixed; inset: 0; z-index: 120; }

  @media (prefers-reduced-motion: reduce) {
    .preset-radio { transition: none; transform: none; }
  }

  /* Narrow settings column: tagline drops under the name. */
  @container settings-panel (max-width: 560px) {
    .preset-main { flex-direction: column; align-items: flex-start; gap: 2px; }
    .preset-line, .preset-info { padding: 12px; }
    .preset-line { flex-wrap: wrap; row-gap: 4px; }
    .preset-trail { display: contents; }
    .preset-more { width: 44px; height: 44px; margin: -6px -8px -6px 0; order: 2; }
    .preset-action { order: 3; flex-basis: calc(100% - 28px); margin-left: 28px; text-align: left; padding-left: 0; }
    .preset-speed { display: none; }
    .preset-action { min-height: 36px; }
    .preset-details { padding-left: 12px; }
    .preset-actions :global(.btn-ghost) { min-height: 40px; flex: 1 1 auto; }
  }
</style>
