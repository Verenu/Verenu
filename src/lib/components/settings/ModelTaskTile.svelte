<script lang="ts">
  import { openCleanupPromptEditor, cleanupPromptStore } from '../../stores.svelte';
  import { fade, fly } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, motionMs, motionPx } from '../../motion';
  import { getProviderLogo, getProviderPlate } from '../../setup/ProviderLogos';
  import type { ProviderId } from '../../settings';
  import { customProvider } from '../../customProviders.svelte';
  import {
    providerDisplayLabel,
    qualifiedModelLabel,
    splitModelId,
    taskLabel,
    type TaskType,
  } from './models';
  import {
    rowForSelection,
    unavailableMessages,
    type LocalControls,
    type PickerContext,
  } from './modelStates';
  import ModelPickerModal from './ModelPickerModal.svelte';
  import Callout from '../Callout.svelte';

  let {
    type,
    step,
    advancedModelUi,
    enabled = true,
    apiKeyStatus,
    context,
    defaultModel,
    fallbackModels,
    onSelectModel,
    onAddFallback,
    onRemoveFallback,
    onMoveFallback,
    onAddCustomModel,
    onOpenApiKeys,
    local,
  }: {
    type: TaskType;
    /** Position in the dictation flow: speech is transcribed first, then cleaned up. */
    step: 1 | 2;
    advancedModelUi: boolean;
    enabled?: boolean;
    apiKeyStatus: Record<ProviderId, boolean>;
    context: PickerContext;
    defaultModel: string;
    fallbackModels: string[];
    onSelectModel: (type: TaskType, id: string) => void;
    onAddFallback: (type: TaskType, id: string) => void;
    onRemoveFallback: (type: TaskType, id: string) => void;
    onMoveFallback: (type: TaskType, id: string, delta: -1 | 1) => void;
    onAddCustomModel: (type: TaskType, id: string) => void;
    onOpenApiKeys: () => void;
    local: LocalControls;
  } = $props();

  let picker = $state<'select' | 'fallback' | null>(null);
  let pickerOrigin = $state<{ x: number; y: number } | null>(null);

  function openPicker(mode: 'select' | 'fallback', event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    pickerOrigin = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    picker = mode;
  }

  const parsedDefault = $derived(splitModelId(defaultModel));
  const activeRow = $derived(rowForSelection(defaultModel, context));

  const missingKeys = $derived(
    [defaultModel, ...fallbackModels]
      .map((id) => splitModelId(id)?.provider)
      .filter((provider): provider is ProviderId => !!provider)
      .filter((provider) => provider !== 'local' && provider !== 'apple-intelligence')
      .filter((provider, index, all) => all.indexOf(provider) === index)
      .filter((provider) => !apiKeyStatus[provider] && customProvider(provider)?.requires_key !== false),
  );

  const missingKeysMessage = $derived(
    missingKeys.length ? `Missing API keys for: ${missingKeys.map(providerDisplayLabel).join(', ')}` : '',
  );
  const unavailable = $derived(unavailableMessages(type, defaultModel, fallbackModels, context));

  const promptCustomized = $derived(!!cleanupPromptStore.override.trim());

  // A new model rises into the same row. One element is animated in place, so the tile never holds two copies of a model.
  let activeEl = $state<HTMLElement | null>(null);
  let shownModel: string | null = null;
  $effect(() => {
    const next = defaultModel;
    if (shownModel !== null && shownModel !== next) {
      activeEl?.animate?.(
        [
          { opacity: 0, transform: `translate3d(0, ${motionPx(8)}px, 0)` },
          { opacity: 1, transform: 'translate3d(0, 0, 0)' },
        ],
        { duration: motionMs(MOTION_MS.panel), easing: 'cubic-bezier(0.22, 1, 0.36, 1)' },
      );
    }
    shownModel = next;
  });

  const role = $derived(
    type === 'transcription'
      ? 'Turns your speech into text.'
      : enabled ? 'Fixes punctuation, filler words, and formatting.' : 'Off. Text is inserted as transcribed.',
  );

  function chipLabel(id: string): string {
    const parsed = splitModelId(id);
    return parsed ? qualifiedModelLabel(parsed.provider, parsed.model) : id;
  }
</script>

<section class="task-tile" class:is-off={!enabled} class:is-advanced={advancedModelUi} data-setting-target={`models-${type}`}>
  <header class="tile-head">
    <span class="head-step" aria-hidden="true">{step}</span>
    <span class="head-text">
      <span class="head-line">
        <span class="head-title">{taskLabel(type)}</span>
        {#if !enabled}<span class="state-chip">Off</span>{/if}
      </span>
      <span class="head-role">{role}</span>
    </span>
  </header>

  {#if missingKeysMessage || unavailable.length}
    <div class="tile-warnings">
      {#if missingKeysMessage}
        <Callout class="warn-banner" severity="warning" compact action={{ label: 'Add API key', onclick: onOpenApiKeys }}>{missingKeysMessage}</Callout>
      {/if}
      {#each unavailable as message (message)}
        <Callout class="warn-banner" severity="warning" compact action={{ label: 'Choose a model', onclick: (event) => openPicker('select', event) }}>{message}</Callout>
      {/each}
    </div>
  {/if}

  <div class="active-stack">
      <div class="active-row" bind:this={activeEl}>
        {#if parsedDefault && activeRow}
          <span
            class="active-logo"
            class:plate-bleed={getProviderPlate(parsedDefault.provider, parsedDefault.model) === 'bleed'}
          >
            {@html getProviderLogo(parsedDefault.provider, parsedDefault.model)}
          </span>
          <span class="active-text">
            <span class="active-name" title={activeRow.label}>{activeRow.label}</span>
            <span class="active-sub">
              <span class="summary-item provider-chip">{providerDisplayLabel(parsedDefault.provider)}</span>
              <span class="summary-item model-chip" title={parsedDefault.model}>{parsedDefault.model}</span>
              {#if fallbackModels.length > 0}
                <span class="summary-item fallback-chip">
                  {fallbackModels.length} fallback{fallbackModels.length !== 1 ? 's' : ''}
                </span>
              {/if}
            </span>
          </span>
        {:else}
          <span class="active-logo"></span>
          <span class="active-text">
            <span class="active-name active-none">No model selected</span>
            <span class="active-sub">Pick a setup below or choose a model</span>
          </span>
        {/if}
      </div>
  </div>

  {#if advancedModelUi}
    <div class="fallback-block">
      <span class="row-label">If it fails, try</span>
      <ul class="fallback-list">
        {#each fallbackModels as id, index (id)}
          {@const parsed = splitModelId(id)}
          <li
            class="fallback-chip-item"
            animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}
            in:fly={{ y: motionPx(-6), duration: motionMs(MOTION_MS.base), easing: cubicOut }}
            out:fade={{ duration: motionMs(MOTION_MS.fast) }}
          >
            <span class="chip-index">{index + 1}</span>
            {#if parsed}
              <span
                class="chip-logo"
                class:plate-bleed={getProviderPlate(parsed.provider, parsed.model) === 'bleed'}
              >
                {@html getProviderLogo(parsed.provider, parsed.model)}
              </span>
            {/if}
            <span class="chip-name" title={chipLabel(id)}>{chipLabel(id)}</span>
            <span class="chip-controls">
              <button
                class="chip-btn"
                type="button"
                disabled={index === 0}
                aria-label="Move {chipLabel(id)} earlier"
                onclick={() => onMoveFallback(type, id, -1)}
                ><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M4 10l4-4 4 4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></button
              >
              <button
                class="chip-btn"
                type="button"
                disabled={index === fallbackModels.length - 1}
                aria-label="Move {chipLabel(id)} later"
                onclick={() => onMoveFallback(type, id, 1)}
                ><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></button
              >
              <button
                class="chip-btn chip-remove"
                type="button"
                aria-label="Remove {chipLabel(id)} from the fallback chain"
                onclick={() => onRemoveFallback(type, id)}
                ><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg></button
              >
            </span>
          </li>
        {:else}
          <li class="fallback-empty">Nothing. This step fails if the model above can’t run.</li>
        {/each}
      </ul>
    </div>

    <footer class="tile-actions">
      <button class="tile-btn tile-btn-primary" type="button" aria-label="Change model" onclick={(event) => openPicker('select', event)}>
        Change
      </button>
      <button class="tile-btn add-fallback" type="button" aria-label="Add fallback" onclick={(event) => openPicker('fallback', event)}>Add fallback</button>
      {#if type === 'cleanup' && parsedDefault}
        <button
          class="tile-btn tile-btn-quiet"
          type="button"
          onclick={(event) =>
            openCleanupPromptEditor(
              parsedDefault.provider,
              parsedDefault.model,
              (event.currentTarget as HTMLButtonElement).getBoundingClientRect(),
            )}
        >
          Edit prompt{promptCustomized ? ' •' : ''}
        </button>
      {/if}
    </footer>
  {/if}
</section>

{#if picker}
  <ModelPickerModal
    mode={picker}
    origin={pickerOrigin}
    task={type}
    {context}
    {defaultModel}
    {fallbackModels}
    {advancedModelUi}
    onSelect={(id) => onSelectModel(type, id)}
    onAddFallback={(id) => onAddFallback(type, id)}
    onRemoveFallback={(id) => onRemoveFallback(type, id)}
    onAddCustomModel={(id) => onAddCustomModel(type, id)}
    {local}
    onOpenApiKeys={() => {
      picker = null;
      onOpenApiKeys();
    }}
    onClose={() => (picker = null)}
  />
{/if}

<style>
  .task-tile {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-lg, 14px);
    background: var(--settings-card-bg, var(--bg-elev));
    padding: 14px 14px 12px;
    transition: border-color var(--ui-duration-base) var(--ui-ease-out);
  }

  .tile-head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }

  .head-step {
    flex: none;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--ink);
    color: var(--bg-elev);
    font-family: var(--sans);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    transition: background-color var(--ui-duration-base) var(--ui-ease-out), color var(--ui-duration-base) var(--ui-ease-out);
  }

  .is-off .head-step {
    background: var(--paper-3);
    color: var(--ink-mute);
  }

  .head-text {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }

  .head-line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 20px;
  }

  .head-title {
    font-family: var(--sans);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--ink);
  }

  .head-role {
    font-family: var(--sans);
    font-size: 12px;
    line-height: 1.4;
    color: var(--ink-mute);
  }

  .state-chip {
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--paper-2);
    color: var(--ink-mute);
    font-family: var(--sans);
    font-size: 11px;
    font-weight: 500;
  }

  .tile-warnings { display: flex; flex-direction: column; gap: 8px; margin-top: 12px; }

  .active-stack {
    margin-top: 14px;
    border-radius: var(--r-md);
    background: var(--paper-2);
  }

  .active-row {
    display: flex;
    align-items: center;
    gap: 11px;
    min-width: 0;
    padding: 10px;
  }

  .is-off .active-row > * {
    opacity: 0.55;
  }

  .active-logo,
  .chip-logo {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 9px;
    overflow: hidden;
    color: var(--ink-soft);
  }

  .active-logo {
    width: 36px;
    height: 36px;
  }

  .active-logo :global(svg) {
    width: 24px;
    height: 24px;
  }

  .chip-logo {
    width: 18px;
    height: 18px;
    border-radius: 5px;
  }

  .chip-logo :global(svg) {
    width: 13px;
    height: 13px;
  }

  .plate-bleed :global(svg) {
    width: 100%;
    height: 100%;
  }

  .active-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .active-name {
    font-family: var(--sans);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .active-none {
    color: var(--ink-mute);
    font-weight: 500;
  }

  .active-sub {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    font-family: var(--sans);
    font-size: 11.5px;
    color: var(--ink-mute);
  }

  .summary-item {
    font-family: var(--sans);
    font-size: 11.5px;
    color: var(--ink-mute);
    white-space: nowrap;
  }

  .summary-item:not(:first-child)::before {
    content: '·';
    margin-right: 6px;
    color: var(--ink-faint);
  }

  .provider-chip {
    flex: none;
    color: var(--ink-soft);
  }

  .model-chip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-size: 11px;
  }

  .fallback-chip {
    flex: none;
  }

  .fallback-block {
    min-width: 0;
    margin-top: 14px;
  }

  .row-label {
    display: block;
    margin: 0 0 4px 2px;
    font-family: var(--sans);
    font-size: 11.5px;
    font-weight: 500;
    color: var(--ink-mute);
  }

  .fallback-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .fallback-chip-item {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    margin-inline: -4px;
    padding: 0 2px 0 6px;
    border-radius: var(--r-sm);
    font-family: var(--sans);
    font-size: 12.5px;
    color: var(--ink-soft);
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .fallback-chip-item:hover,
  .fallback-chip-item:focus-within {
    background: var(--control-hover);
  }

  .chip-index {
    width: 12px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--ink-faint);
  }

  .chip-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-controls {
    display: flex;
    flex-shrink: 0;
  }

  .chip-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out);
  }

  .chip-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .chip-btn:hover:not(:disabled) {
    background: var(--paper-3);
    color: var(--ink);
  }

  .chip-btn:active:not(:disabled) {
    transform: scale(0.9);
  }

  .chip-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .chip-remove:hover:not(:disabled) {
    background: var(--danger-bg);
    color: var(--danger);
  }

  .fallback-empty {
    min-height: 32px;
    display: flex;
    align-items: center;
    padding-left: 2px;
    font-family: var(--sans);
    font-size: 12px;
    line-height: 1.4;
    color: var(--ink-mute);
  }

  .tile-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: auto;
    padding-top: 12px;
  }

  .tile-btn {
    flex: none;
    min-height: 30px;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    padding: 0 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--ui-dropdown-radius);
    background: transparent;
    color: var(--ink-soft);
    cursor: pointer;
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out);
    white-space: nowrap;
  }

  .tile-btn:hover {
    background: var(--control-hover);
    color: var(--ink);
  }

  .tile-btn:active {
    transform: scale(0.97);
  }

  .tile-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    box-shadow: var(--ui-focus-ring);
  }

  .tile-btn-primary {
    border-color: var(--ink);
    background: var(--ink);
    color: var(--bg-elev);
  }

  .tile-btn-primary:hover {
    background: var(--ink-strong);
    color: var(--bg-elev);
  }

  .tile-btn-quiet {
    margin-left: auto;
  }

  @container settings-panel (max-width: 480px) {
    .tile-btn { min-height: 36px; }
    .tile-btn-quiet { margin-left: 0; }
    .chip-btn { width: 32px; height: 32px; }
    .fallback-chip-item { min-height: 40px; }
  }

  @media (prefers-reduced-motion: reduce) {
    .task-tile, .head-step, .fallback-chip-item, .chip-btn, .tile-btn { transition: none; }
  }
</style>
