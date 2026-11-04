<script lang="ts">
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../../motion';
  import Callout from '../../components/Callout.svelte';
  import type { Entry } from './helpers';
  import {
    fmtCost,
    fmtMs,
    fmtTokens,
    formatLogForCopy,
    isOnDevice,
    requestTarget,
    stageLabel,
    stageModel,
    summarizeLog,
    type DictationLog,
  } from './dictationLog';

  let { entry, log, loading = false, error = '', devMode = false, onclose, oncopy }: {
    entry: Entry;
    log: DictationLog | null;
    loading?: boolean;
    error?: string;
    devMode?: boolean;
    onclose: () => void;
    oncopy?: (text: string) => void;
  } = $props();

  let closeButton = $state<HTMLButtonElement | null>(null);
  const summary = $derived(summarizeLog(entry, log));

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    onclose();
  }
</script>

<svelte:window onkeydown={onKeydown} />
<button type="button" class="ui-modal-backdrop dlog-backdrop" aria-label="Close dictation log" use:portal
  onclick={onclose} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
<div class="ui-modal-card ui-modal-card--portalled dlog-dialog" use:portal
  use:modalFocusTrap={{ active: true, initialFocus: () => closeButton, restoreFocus: () => null }}
  role="dialog" aria-modal="true" aria-labelledby="dlog-title" aria-describedby="dlog-summary" tabindex="-1"
  in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
  out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}>
  <div class="ui-modal-head"><h2 id="dlog-title" class="ui-modal-title">Dictation log</h2></div>
  <div class="ui-modal-body dlog-body">
    {#if loading}
      <p id="dlog-summary" class="ui-modal-copy" role="status">Loading log…</p>
    {:else if error}
      <div id="dlog-summary">
        <Callout severity="error" title="Log could not be loaded" compact>{error}</Callout>
      </div>
    {:else}
      <p id="dlog-summary" class="ui-modal-copy dlog-headline">{summary.headline}</p>

      {#if !summary.hasLog}
        <p class="ui-modal-copy dlog-note">
          Dictations recorded before logs were added only keep their text.
          {#if summary.rows.length > 0}Here is what Verenu still knows.{/if}
        </p>
      {/if}

      {#if summary.rows.length > 0}
        <dl class="dlog-rows">
          {#each summary.rows as row (row.label)}
            <div class="dlog-row"><dt>{row.label}</dt><dd>{row.value}</dd></div>
          {/each}
        </dl>
      {/if}

      {#if summary.failures.length > 0}
        <div class="dlog-failures">
          {#each summary.failures as failure (failure)}
            <Callout severity="error" compact>{failure}</Callout>
          {/each}
        </div>
      {/if}

      {#if summary.fallbacks.length > 0}
        <div class="dlog-failures">
          {#each summary.fallbacks as note (note)}
            <Callout severity="info" compact>{note}</Callout>
          {/each}
        </div>
      {/if}

      {#if summary.stages.length > 0}
        <section class="dlog-dev" aria-label="Steps">
          <h3 class="dlog-dev-title">Steps</h3>
          <ul class="dlog-stages">
            {#each summary.stages as stage, i (`${stage.stage}-${i}`)}
              <li class="dlog-stage" data-status={stage.status ?? 'ok'}>
                <div class="dlog-stage-head">
                  <span class="dlog-stage-name">{stageLabel(stage.stage)}</span>
                  {#if stage.status && stage.status !== 'ok'}<span class="dlog-stage-status">{stage.status}</span>{/if}
                  {#if stage.duration_ms != null}<span class="dlog-stage-time">{fmtMs(stage.duration_ms)}</span>{/if}
                </div>
                <div class="dlog-stage-meta">
                  {#if stageModel(stage)}<span>{stageModel(stage)}</span>{/if}
                  {#if requestTarget(stage)}<span>Request sent to {requestTarget(stage)}</span>{/if}
                  {#if fmtTokens(stage)}<span>{fmtTokens(stage)} tokens</span>{/if}
                  {#if stage.cost_usd != null}<span>{fmtCost(stage.cost_usd)}</span>{/if}
                </div>
                {#if devMode}
                  <dl class="dlog-raw" aria-label="Developer details for {stageLabel(stage.stage)}">
                    <div><dt>stage</dt><dd>{stage.stage}</dd></div>
                    {#if stage.provider}<div><dt>provider</dt><dd>{stage.provider}</dd></div>{/if}
                    {#if stage.model}<div><dt>model</dt><dd>{stage.model}</dd></div>{/if}
                    {#if stage.provider || stage.local != null}<div><dt>on device</dt><dd>{isOnDevice(stage) ? 'yes' : 'no'}</dd></div>{/if}
                    {#if stage.duration_ms != null}<div><dt>duration_ms</dt><dd>{Math.round(stage.duration_ms)}</dd></div>{/if}
                    {#if stage.input_tokens != null}<div><dt>input_tokens</dt><dd>{stage.input_tokens}</dd></div>{/if}
                    {#if stage.output_tokens != null}<div><dt>output_tokens</dt><dd>{stage.output_tokens}</dd></div>{/if}
                    {#if stage.cost_usd != null}<div><dt>cost_usd</dt><dd>{stage.cost_usd}</dd></div>{/if}
                    {#if stage.error}<div><dt>error</dt><dd>{stage.error}</dd></div>{/if}
                  </dl>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}
  </div>
  <div class="ui-modal-foot"><div class="ui-modal-actions">
    {#if devMode && oncopy && summary.hasLog && !loading && !error}
      <button type="button" class="btn-ghost" onclick={() => oncopy?.(formatLogForCopy(summary))}>Copy details</button>
    {/if}
    <button bind:this={closeButton} type="button" class="btn-primary" onclick={onclose}>Close</button>
  </div></div>
</div>

<style>
  .dlog-backdrop { border: 0; cursor: default; padding: 0; z-index: 160; }
  .dlog-dialog { --ui-modal-w: 440px; z-index: 161; }
  .dlog-body { display: grid; gap: 12px; max-height: min(60vh, 460px); }
  .dlog-headline { color: var(--ink-strong); font-weight: 500; }
  .dlog-note { font-size: 12.5px; color: var(--ink-mute); }

  .dlog-rows { margin: 0; display: grid; }
  .dlog-row {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
    font-size: 13px;
  }
  .dlog-row:first-child { border-top: 1px solid var(--line); }
  .dlog-row dt { color: var(--ink-soft); }
  .dlog-row dd { margin: 0; color: var(--ink-strong); text-align: right; min-width: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }

  .dlog-failures { display: grid; gap: 6px; }

  .dlog-dev { display: grid; gap: 8px; margin-top: 4px; }
  .dlog-dev-title { margin: 0; font-size: 13px; font-weight: 600; color: var(--ink-soft); }
  .dlog-stages { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .dlog-stage {
    border: 1px solid var(--line);
    border-radius: var(--r-md, 8px);
    padding: 8px 10px;
    display: grid;
    gap: 3px;
  }
  .dlog-stage[data-status='failed'] { border-color: var(--danger, var(--line)); }
  .dlog-stage-head { display: flex; align-items: baseline; gap: 8px; font-size: 13px; }
  .dlog-stage-name { color: var(--ink-strong); font-weight: 500; }
  .dlog-stage-status { color: var(--ink-mute); font-size: 12px; }
  .dlog-stage-time { margin-left: auto; font-family: var(--mono); font-size: 12px; color: var(--ink-soft); font-variant-numeric: tabular-nums; }
  .dlog-raw { margin: 4px 0 0; padding-top: 6px; border-top: 1px dashed var(--line); display: grid; gap: 2px; font-family: var(--mono); font-size: 11.5px; }
  .dlog-raw div { display: flex; gap: 10px; }
  .dlog-raw dt { color: var(--ink-mute); min-width: 92px; }
  .dlog-raw dd { margin: 0; color: var(--ink-soft); overflow-wrap: anywhere; min-width: 0; }
  .dlog-stage-meta { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 12px; color: var(--ink-mute); }
</style>
