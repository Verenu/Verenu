<script lang="ts">
  import { openCleanupPromptEditor, cleanupPromptStore } from '../../stores.svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../../motion';
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

  function chipLabel(id: string): string {
    const parsed = splitModelId(id);
    return parsed ? qualifiedModelLabel(parsed.provider, parsed.model) : id;
  }
</script>

<section class="task-tile" class:is-off={!enabled} data-setting-target={`models-${type}`}>
  <header class="tile-head">
    <span class="head-title">{taskLabel(type)}</span>
    {#if !enabled}<span class="state-chip">Off</span>{/if}
    {#if type === 'cleanup' && advancedModelUi && parsedDefault}
      <button
        class="tile-btn"
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

  <div class="tile-row">
    <span class="row-label">Model</span>
    <div class="active-row">
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
            <span class="summary-item model-chip">{parsedDefault.model}</span>
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
          <span class="active-sub">Pick a setup above or choose a model</span>
        </span>
      {/if}
    </div>
    <div class="row-action">
      {#if advancedModelUi}
        <button class="tile-btn tile-btn-primary" type="button" aria-label="Change model" onclick={(event) => openPicker('select', event)}>
          Change
        </button>
      {/if}
    </div>
  </div>

  {#if advancedModelUi}
    <div class="tile-row tile-row-top">
      <span class="row-label">Fallbacks</span>
      <div class="fallback-block">
        <ul class="fallback-list">
          {#each fallbackModels as id, index (id)}
            {@const parsed = splitModelId(id)}
            <li
              class="fallback-chip-item"
              in:fly={{ y: -6, duration: motionMs(MOTION_MS.base), easing: cubicOut }}
              out:fly={{ y: -6, duration: motionMs(MOTION_MS.fast), easing: cubicOut }}
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
              <span class="chip-name">{chipLabel(id)}</span>
              <span class="chip-controls">
                <button
                  class="chip-btn"
                  type="button"
                  disabled={index === 0}
                  aria-label="Move {chipLabel(id)} earlier"
                  onclick={() => onMoveFallback(type, id, -1)}>↑</button
                >
                <button
                  class="chip-btn"
                  type="button"
                  disabled={index === fallbackModels.length - 1}
                  aria-label="Move {chipLabel(id)} later"
                  onclick={() => onMoveFallback(type, id, 1)}>↓</button
                >
                <button
                  class="chip-btn chip-remove"
                  type="button"
                  aria-label="Remove {chipLabel(id)} from the fallback chain"
                  onclick={() => onRemoveFallback(type, id)}>×</button
                >
              </span>
            </li>
          {:else}
            <li class="fallback-empty">None. This task fails if the model above can’t run.</li>
          {/each}
        </ul>
      </div>
      <div class="row-action">
        <button class="tile-btn add-fallback" type="button" aria-label="Add fallback" onclick={(event) => openPicker('fallback', event)}>Add</button>
      </div>
    </div>
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
    --tile-label-col: 88px;
    --tile-action-col: 76px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    padding: 0 16px;
    margin-bottom: 12px;
  }

  .tile-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    border-bottom: 1px solid var(--line);
  }

  .head-title {
    flex: 1;
    font-family: var(--sans);
    font-size: 14px;
    font-weight: 600;
    color: var(--ink);
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

  .tile-btn {
    flex: none;
    min-width: 64px;
    min-height: 28px;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    padding: 0 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--ui-dropdown-radius);
    background: transparent;
    color: var(--ink-soft);
    cursor: pointer;
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    white-space: nowrap;
  }

  .tile-btn:hover {
    background: var(--control-hover);
    color: var(--ink);
  }

  .tile-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    box-shadow: var(--ui-focus-ring);
  }

  .tile-btn-primary {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--line));
    color: var(--accent-ink);
  }

  .tile-row {
    display: grid;
    grid-template-columns: var(--tile-label-col) minmax(0, 1fr) var(--tile-action-col);
    align-items: center;
    column-gap: 12px;
    min-height: 56px;
    padding: 8px 0;
  }

  .tile-row + .tile-row {
    border-top: 1px solid var(--line);
  }

  .tile-row-top {
    align-items: start;
  }

  .tile-row-top .row-label,
  .tile-row-top .row-action {
    padding-top: 6px;
  }

  .row-label {
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    color: var(--ink-mute);
  }

  .row-action {
    display: flex;
    justify-content: flex-end;
  }

  .active-row {
    display: flex;
    align-items: center;
    gap: 11px;
    min-width: 0;
  }

  .is-off .active-row {
    opacity: 0.55;
  }

  .active-logo,
  .chip-logo {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 8px;
    overflow: hidden;
    color: var(--ink-soft);
  }

  .active-logo {
    width: 32px;
    height: 32px;
  }

  .active-logo :global(svg) {
    width: 22px;
    height: 22px;
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
    font-size: 13.5px;
    font-weight: 500;
    color: var(--ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .active-none {
    color: var(--ink-mute);
  }

  .active-sub {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
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

  .tile-warnings { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }

  .fallback-block {
    min-width: 0;
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
    gap: 9px;
    min-height: 32px;
    margin-inline: -6px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    font-family: var(--sans);
    font-size: 12px;
    color: var(--ink-soft);
  }

  .fallback-chip-item:hover {
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
    gap: 1px;
    flex-shrink: 0;
  }

  .chip-btn {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: var(--ink-faint);
    font-size: 11px;
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .fallback-chip-item:hover .chip-btn,
  .fallback-chip-item:focus-within .chip-btn {
    opacity: 1;
  }

  .chip-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .chip-btn:hover:not(:disabled) {
    color: var(--ink);
  }

  .chip-btn:disabled {
    opacity: 0;
    cursor: default;
  }

  .fallback-chip-item:hover .chip-btn:disabled {
    opacity: 0.25;
  }

  .chip-remove:hover {
    color: var(--danger);
  }

  .fallback-empty {
    min-height: 32px;
    display: flex;
    align-items: center;
    font-family: var(--sans);
    font-size: 12px;
    color: var(--ink-mute);
  }

  @container settings-panel (max-width: 480px) {
    .task-tile {
      --tile-label-col: 1fr;
      --tile-action-col: auto;
    }

    .tile-row {
      grid-template-columns: minmax(0, 1fr) auto;
      row-gap: 4px;
    }

    .row-label {
      grid-column: 1 / -1;
    }

    .tile-row-top .row-label,
    .tile-row-top .row-action {
      padding-top: 0;
    }

    .tile-btn {
      min-height: 36px;
    }
  }
</style>
