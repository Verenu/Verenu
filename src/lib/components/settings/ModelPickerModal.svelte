<script lang="ts">
  import Callout from '../Callout.svelte';
  import { refreshAppleIntelligence } from '../../appleIntelligence.svelte';
  import { onMount, untrack } from 'svelte';
  import { customProviderStore } from '../../customProviders.svelte';
  import { fade, fly } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { isTrustworthy, CLOUD_PROVIDERS, modelCatalogStore, refreshCatalog, trackedIds } from '../../modelCatalogStore.svelte';
  import { expandFromOrigin, modalBackdrop, MOTION_MS, motionMs } from '../../motion';
  import { isAndroid } from '../../platform';
  import type { ProviderId } from '../../settings';
  import { modelId, providerDisplayLabel, splitModelId, taskLabel, type TaskType } from './models';
  import {
    curatedRows,
    localModelOffered,
    discoveredRows,
    unverifiedRows,
    appleIntelligenceVisible,
    rowForSelection,
    type LocalControls,
    type ModelRow,
    type PickerContext,
  } from './modelStates';

  let {
    mode,
    task,
    context,
    defaultModel,
    fallbackModels,
    advancedModelUi,
    onSelect,
    onAddFallback,
    onRemoveFallback,
    onAddCustomModel,
    onOpenApiKeys,
    onClose,
    local,
    origin = null,
  }: {
    mode: 'select' | 'fallback';
    task: TaskType;
    context: PickerContext;
    defaultModel: string;
    fallbackModels: string[];
    advancedModelUi: boolean;
    onSelect: (id: string) => void;
    onAddFallback: (id: string) => void;
    onRemoveFallback: (id: string) => void;
    onAddCustomModel: (id: string) => void;
    onOpenApiKeys: () => void;
    onClose: () => void;
    local: LocalControls;
    origin?: { x: number; y: number } | null;
  } = $props();

  const localModel = (id: string) => local.models.find((m) => m.id === id);
  const isDownloading = (id: string) => localModel(id)?.is_downloading === true;
  const isDownloaded = (id: string) => localModel(id)?.is_downloaded === true;
  const downloadPercent = (id: string) => {
    const progress = local.downloadProgress[id]?.progress;
    return progress == null ? null : Math.round(progress * 100);
  };
  const sizeLabel = (mb?: number) =>
    mb ? (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${Math.round(mb)} MB`) : '';

  const runtimePending = $derived(
    !!local.runtime && !local.runtime.info?.installed && !!local.runtime.info,
  );

  let modalEl = $state<HTMLElement | null>(null);
  let query = $state('');
  let providerFilter = $state<ProviderId | 'all'>(
    isAndroid
      ? untrack(() => {
          const provider = splitModelId(defaultModel)?.provider;
          return provider && provider !== 'apple-intelligence' ? provider : 'all';
        })
      : 'all',
  );

  const cardMotion = (node: Element, params: { origin?: { x: number; y: number }; duration?: number }) =>
    isAndroid
      ? fly(node, { y: 36, duration: motionMs(params.duration ?? MOTION_MS.base), easing: cubicOut })
      : expandFromOrigin(node, params);
  let showUnverified = $state(false);
  let railEl = $state<HTMLElement | null>(null);
  let ink = $state({ x: 0, w: 0, ready: false });
  $effect(() => {
    filter;
    tabProviders;
    const active = railEl?.querySelector<HTMLElement>('.rail-active');
    if (!active) return;
    const ready = untrack(() => ink.ready);
    ink = { x: active.offsetLeft, w: active.offsetWidth, ready };
    if (!ready) requestAnimationFrame(() => (ink.ready = true));
  });
  let panelLeft = $state(0);

  onMount(() => {
    if (task === 'cleanup') void refreshAppleIntelligence();
    const updatePanelLeft = () => {
      const panel = document.querySelector('.settings-page');
      if (panel) panelLeft = panel.getBoundingClientRect().left;
    };
    updatePanelLeft();
    window.addEventListener('resize', updatePanelLeft);
    requestAnimationFrame(() => {
      // Android: do not raise the keyboard on open; it resizes the whole page
      // while the dialog is still animating in.
      if (!isAndroid && modalEl?.isConnected) {
        modalEl.querySelector<HTMLElement>('.picker-search')?.focus();
      }
    });
    return () => window.removeEventListener('resize', updatePanelLeft);
  });

  const RAIL_ORDER = $derived<ProviderId[]>(['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai', 'local', ...(task === 'cleanup' && appleIntelligenceVisible(context) ? ['apple-intelligence' as const] : []),
    ...customProviderStore.providers.filter(p => task === 'transcription' ? p.supports_transcription : p.supports_cleanup).map(p => p.id)]);

  const pinned = $derived([defaultModel, ...fallbackModels].filter(Boolean));
  const supportedHere = (row: ModelRow) => row.provider !== 'local' || localModelOffered(local, task, row.id);
  const curated = $derived([...curatedRows(context, pinned), ...discoveredRows(context)].filter(supportedHere));
  const unverified = $derived(unverifiedRows(context).filter(supportedHere));
  // Providers without a saved key stay out of the picker; "+ Provider" is the way in.
  const usable = (row: ModelRow) => row.remedy !== 'add-key';

  const tabProviders = $derived(
    RAIL_ORDER.filter((provider) => curated.some((row) => row.provider === provider && usable(row))),
  );
  const filter = $derived(
    providerFilter === 'all' || tabProviders.includes(providerFilter) ? providerFilter : 'all',
  );

  let refreshingApple = $state(false);
  const canRefreshApple = $derived(task === 'cleanup' && (filter === 'all' || filter === 'apple-intelligence'));
  const refreshProviders = $derived(CLOUD_PROVIDERS.filter((provider) =>
    (filter === 'all' || filter === provider) && (context.apiKeyStatus[provider] || provider === 'openrouter')));
  const refreshing = $derived((canRefreshApple && refreshingApple) || refreshProviders.some((provider) => modelCatalogStore.refreshing[provider]));
  async function refreshAppleModels() {
    refreshingApple = true;
    try { await refreshAppleIntelligence(); }
    finally { refreshingApple = false; }
  }
  async function refreshModels() {
    await Promise.all([
      ...(canRefreshApple ? [refreshAppleModels()] : []),
      ...refreshProviders.map((provider) => refreshCatalog(provider, trackedIds(pinned, []))),
    ]);
  }
  const catalogNote = $derived(refreshing
    ? { severity: 'info' as const, text: 'Refreshing model lists… You can still choose a model.' }
    : refreshProviders.some((provider) => context.cache[provider]?.lastError)
      ? { severity: 'warning' as const, text: 'Refresh unavailable. Keeping cached models.' }
      : refreshProviders.some((provider) => context.cache[provider]?.warning)
        ? { severity: 'warning' as const, text: 'Some capability metadata is unavailable. Keeping known models.' }
        : null);

  const needle = $derived(query.trim().toLowerCase());
  function matches(row: ModelRow): boolean {
    if (!needle) return true;
    return (
      row.id.toLowerCase().includes(needle) ||
      row.label.toLowerCase().includes(needle) ||
      providerDisplayLabel(row.provider).toLowerCase().includes(needle)
    );
  }
  const inFilter = (row: ModelRow) => filter === 'all' || row.provider === filter;
  const wanted = (row: ModelRow) => matches(row) && inFilter(row);

  const RANK: Record<ModelRow['state'], number> = {
    ready: 0,
    unverified: 1,
    'needs-setup': 2,
    unavailable: 3,
    'not-found': 4,
  };

  const phonePreferred = (row: ModelRow) =>
    isAndroid && row.provider === 'local' && row.id === 'qwen2.5-0.5b-instruct' ? 0 : 1;

  const everyRow = $derived([...curated, ...(showUnverified ? unverified : [])]);
  const pinnedRows = $derived(
    pinned
      .map((key) => everyRow.find((row) => row.key === key) ?? rowForSelection(key, context))
      .filter((row): row is ModelRow => !!row && wanted(row)
        && (row.provider !== 'apple-intelligence' || appleIntelligenceVisible(context))),
  );
  const restRows = $derived(
    everyRow
      .filter((row) => usable(row) && wanted(row) && !pinned.includes(row.key))
      .sort((a, b) => RAIL_ORDER.indexOf(a.provider) - RAIL_ORDER.indexOf(b.provider)
        || RANK[a.state] - RANK[b.state] || phonePreferred(a) - phonePreferred(b) || a.label.localeCompare(b.label)),
  );

  const PER_PROVIDER = 8;
  const PAGE = 60;
  let limit = $state(PAGE);
  $effect(() => {
    query;
    providerFilter;
    showUnverified;
    limit = PAGE;
  });

  const collapsed = $derived(filter === 'all' && !needle);
  const byProvider = $derived(
    RAIL_ORDER.map((provider) => restRows.filter((row) => row.provider === provider)).filter((rows) => rows.length > 0),
  );
  const listed = $derived(collapsed ? byProvider.flatMap((rows) => rows.slice(0, PER_PROVIDER)) : restRows.slice(0, limit));
  const overflow = $derived(
    collapsed
      ? byProvider.filter((rows) => rows.length > PER_PROVIDER).map((rows) => ({ provider: rows[0].provider, count: rows.length - PER_PROVIDER }))
      : [],
  );
  const showProvider = $derived(filter === 'all' || !!needle);
  const hiddenCount = $derived(
    unverified.filter((row) => matches(row) && inFilter(row) && usable(row)).length,
  );
  const empty = $derived(pinnedRows.length + listed.length === 0);

  function activate(row: ModelRow) {
    if (row.remedy === 'add-key') return onOpenApiKeys();
    if (row.provider === 'local' && row.state === 'unavailable') return;
    if (row.remedy === 'download') return local.onDownload(row.id);
    if (row.provider === 'local' && runtimePending) {
      if (!local.runtime?.info?.is_downloading) local.runtime?.onDownload();
      return;
    }
    if (mode === 'fallback') {
      if (row.key === defaultModel || fallbackModels.includes(row.key)) return;
      onAddFallback(row.key);
    } else {
      onSelect(row.key);
    }
    onClose();
  }

  function addFallback(row: ModelRow) {
    if (row.provider === 'local' && runtimePending) {
      if (!local.runtime?.info?.is_downloading) local.runtime?.onDownload();
      return;
    }
    onAddFallback(row.key);
  }

  const customProvider = $derived<ProviderId>(
    filter !== 'all' ? filter : (splitModelId(defaultModel)?.provider ?? 'groq'),
  );
  const canAddCustom = $derived(
    advancedModelUi && customProvider !== 'local' && customProvider !== 'apple-intelligence',
  );

  let customId = $state('');
  let customError = $state('');

  function submitCustom() {
    const value = customId.trim();
    if (!value) return;

    const cache = context.cache[customProvider];
    if (isTrustworthy(cache) && !cache!.ids.includes(value)) {
      customError = `${providerDisplayLabel(customProvider)} doesn't list a model called “${value}”.`;
      return;
    }

    customError = '';
    const selected = modelId(customProvider, value);
    if (mode === 'fallback') {
      onAddFallback(selected);
    } else {
      onAddCustomModel(selected);
    }
    onClose();
  }

  $effect(() => {
    customId;
    customError = '';
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && modalEl) {
      const stops = [...modalEl.querySelectorAll<HTMLElement>('.picker-search, .row-main')];
      const next = stops[stops.indexOf(document.activeElement as HTMLElement) + (event.key === 'ArrowDown' ? 1 : -1)];
      if (!next) return;
      event.preventDefault();
      next.focus();
      next.scrollIntoView({ block: 'nearest' });
    }
  }

  function onSearchKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    modalEl?.querySelector<HTMLElement>('.row-main')?.click();
  }
</script>

{#snippet modelRow(row: ModelRow)}
  {@const position = pinned.indexOf(row.key)}
  {@const isMain = row.key === defaultModel}
  {@const isLocal = row.provider === 'local'}
  {@const localManaged = isLocal && row.state !== 'unavailable' && localModel(row.id)?.install_kind !== 'system_managed'}
  <div class="model-row" class:row-pinned={position >= 0} class:row-dim={row.state === 'needs-setup' && row.remedy !== 'download'}>
    <button
      class="row-main"
      type="button"
      title={row.note ? `${row.id} · ${row.note}` : row.id}
      aria-current={isMain ? 'true' : undefined}
      onclick={() => activate(row)}
    >
      <span class="row-num" class:row-state={position >= 0} aria-hidden="true">{position >= 0 ? position + 1 : ''}</span>
      <span class="row-name">{row.label}</span>
      {#if position >= 0}
        <span class="sr-only">{isMain ? 'Main model' : `Fallback ${position}`}</span>
      {/if}
    </button>

    <span class="row-meta">
      {#if localManaged && isDownloading(row.id)}
        <span>{downloadPercent(row.id) === null ? '…' : `${downloadPercent(row.id)}%`}</span>
        <button class="row-tool" data-testid="cancel-model-download" type="button" onclick={() => local.onCancel(row.id)}>Cancel</button>
      {:else if localManaged && !isDownloaded(row.id)}
        <button class="row-tool row-tool-strong" data-testid="download-model" type="button" onclick={() => local.onDownload(row.id)}>
          Download{row.sizeMb ? ` ${sizeLabel(row.sizeMb)}` : ''}
        </button>
      {:else}
        <span class="row-swap">
          <span class="row-provider">{showProvider ? providerDisplayLabel(row.provider) : ''}</span>
          <span class="row-acts">
            {#if localManaged}
              <button class="row-tool" data-testid="delete-model" type="button" onclick={() => local.onDelete(row.id)}>Delete</button>
            {/if}
            {#if mode === 'select' && !isMain && row.remedy === 'none'}
              {#if fallbackModels.includes(row.key)}
                <button class="row-tool" type="button" onclick={() => onRemoveFallback(row.key)}>Remove</button>
              {:else}
                <button class="row-tool" type="button" onclick={() => addFallback(row)}>Add fallback</button>
              {/if}
            {/if}
          </span>
        </span>
      {/if}
    </span>
    {#if isLocal && isDownloading(row.id)}
      <span class="row-bar" aria-hidden="true"><i style="width: {downloadPercent(row.id) ?? 8}%"></i></span>
    {/if}
  </div>
{/snippet}

<div class="picker-wrap" use:portal style="padding-left: {panelLeft}px">
  <div
    class="picker-backdrop"
    onclick={onClose}
    onkeydown={(event) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onClose();
      }
    }}
    role="button"
    tabindex="0"
    aria-label="Close model picker"
    in:modalBackdrop={{ duration: MOTION_MS.fast }}
    out:modalBackdrop={{ duration: MOTION_MS.fast }}
  ></div>

  <div
    bind:this={modalEl}
    class="picker-card"
    use:modalFocusTrap={{
      active: true,
      initialFocus: () => (isAndroid ? modalEl : modalEl?.querySelector<HTMLElement>('.picker-search') ?? modalEl),
    }}
    role="dialog"
    aria-modal="true"
    aria-label={mode === 'fallback'
      ? `Add a ${taskLabel(task).toLowerCase()} fallback`
      : `Choose a ${taskLabel(task).toLowerCase()} model`}
    tabindex="-1"
    onkeydown={onKeydown}
    in:cardMotion={{ origin: origin ?? undefined, duration: MOTION_MS.base }}
    out:cardMotion={{ origin: origin ?? undefined, duration: MOTION_MS.fast }}
  >
    <div class="picker-search-row">
      <svg class="search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
        <circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" />
      </svg>
      <input
        class="picker-search"
        type="search"
        placeholder={mode === 'fallback' ? `Search ${taskLabel(task).toLowerCase()} fallbacks` : `Search ${taskLabel(task).toLowerCase()} models`}
        bind:value={query}
        onkeydown={onSearchKeydown}
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
        aria-label="Search models"
      />
      <button type="button" class="picker-close" aria-label="Close" onclick={onClose}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>

    <div class="picker-tabs">
      <nav class="picker-rail" aria-label="Filter by provider" bind:this={railEl}>
        {#each ['all', ...tabProviders] as provider (provider)}
          <button
            type="button"
            class="rail-item"
            class:rail-active={filter === provider}
            aria-pressed={filter === provider}
            onclick={() => (providerFilter = provider as ProviderId | 'all')}
          >{provider === 'all' ? 'All' : providerDisplayLabel(provider as ProviderId)}</button>
        {/each}
        <span class="rail-ink" class:rail-ink-ready={ink.ready} style="transform: translateX({ink.x}px) scaleX({ink.w})" aria-hidden="true"></span>
      </nav>
      <button type="button" class="tab-tool tab-add" aria-label="Add provider" onclick={onOpenApiKeys}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M6 1.5v9M1.5 6h9" /></svg>
        <span class="tab-add-label">Provider</span>
      </button>
      <button
        type="button"
        class="tab-tool tab-icon"
        class:spinning={refreshing}
        aria-label="Refresh models"
        title="Refresh models"
        onclick={refreshModels}
        disabled={refreshing || (!canRefreshApple && refreshProviders.length === 0)}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M20 11a8 8 0 10-2.3 5.7M20 4v7h-7" />
        </svg>
      </button>
    </div>

    {#if catalogNote}
      <div class="catalog-note"><Callout severity={catalogNote.severity} compact>{catalogNote.text}</Callout></div>
    {/if}

    <div class="picker-list scroll-styled scroll-thumb-elev">
      {#key filter}
      <div in:fade={{ duration: motionMs(MOTION_MS.fast) }}>
      {#if filter === 'local' && local.runtime?.info && runtimePending}
        <p class="list-note runtime-note">
          {#if local.runtime.info.is_downloading}
            Fetching the on-device runtime…
            <button class="row-tool" type="button" onclick={() => local.runtime?.onCancel()}>Cancel</button>
          {:else}
            On-device models need a one-time runtime (~{local.runtime.info.approx_download_mb} MB).
            <button class="row-tool row-tool-strong" type="button" onclick={() => local.runtime?.onDownload()}>Install engine</button>
          {/if}
        </p>
      {/if}

      {#each pinnedRows as row (row.key)}
        <div animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
          {@render modelRow(row)}
        </div>
      {/each}
      {#if pinnedRows.length > 0 && listed.length > 0}
        <div class="list-sep"></div>
      {/if}
      {#each listed as row (row.key)}
        <div animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
          {@render modelRow(row)}
        </div>
      {/each}

      {#each overflow as group (group.provider)}
        <button type="button" class="list-more" onclick={() => (providerFilter = group.provider)}>
          {group.count} more on {providerDisplayLabel(group.provider)}
        </button>
      {/each}
      {#if !collapsed && restRows.length > limit}
        <button type="button" class="list-more" onclick={() => (limit += PAGE)}>
          Show next {Math.min(PAGE, restRows.length - limit)}{filter === 'all' ? '' : ` ${providerDisplayLabel(filter)}`} models
        </button>
      {/if}
      {#if hiddenCount > 0 || showUnverified}
        <button type="button" class="list-more" aria-expanded={showUnverified} onclick={() => (showUnverified = !showUnverified)}>
          {showUnverified ? 'Hide unverified models' : `${hiddenCount} unverified, listed by the provider`}
        </button>
      {/if}

      {#if empty}
        <p class="list-empty">{needle ? `No model called “${query.trim()}”` : 'No models yet'}</p>
      {/if}

      {#if filter === 'local' && local.supported && localModel('fluid-english-booster')}
        {@const booster = localModel('fluid-english-booster')!}
        <p class="list-note" data-testid="vocabulary-booster">
          <span>English vocabulary booster, optional{booster.size_mb ? ` · ${sizeLabel(booster.size_mb)}` : ''}. Re-checks Context vocabulary spelling after Parakeet CoreML, English only.</span>
          {#if booster.is_downloading}
            <button class="row-tool" type="button" onclick={() => local.onCancel(booster.id)}>Cancel</button>
          {:else if booster.is_downloaded}
            <button class="row-tool" type="button" aria-label="Remove vocabulary booster" onclick={() => local.onDelete(booster.id)}>Remove</button>
          {:else}
            <button class="row-tool row-tool-strong" type="button" aria-label="Install vocabulary booster" onclick={() => local.onDownload(booster.id)}>Install</button>
          {/if}
        </p>
      {/if}
      </div>
      {/key}
    </div>

    {#if canAddCustom}
      <div class="picker-foot">
        <input
          class="custom-input"
          class:custom-invalid={!!customError}
          placeholder="Custom {providerDisplayLabel(customProvider)} model ID"
          bind:value={customId}
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          aria-label="Custom model ID"
          aria-invalid={!!customError}
          onkeydown={(event) => {
            if (event.key === 'Enter') {
              event.stopPropagation();
              submitCustom();
            }
          }}
        />
        {#if customError}<p class="list-error" role="alert">{customError}</p>{/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .picker-wrap {
    position: fixed;
    inset: 0;
    box-sizing: border-box;
    z-index: 60;
    display: grid;
    place-items: center;
  }

  .picker-backdrop {
    position: absolute;
    inset: 0;
    border: none;
    background: var(--overlay);
    backdrop-filter: blur(2px);
  }

  .picker-card {
    position: relative;
    z-index: 1;
    width: min(640px, calc(100% - 32px));
    max-width: 100%;
    height: min(520px, 88vh);
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-elev);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-family: var(--sans);
  }

  .picker-search-row {
    position: relative;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    height: 58px;
    padding: 0 22px;
  }

  .search-icon {
    flex-shrink: 0;
    color: var(--ink-mute);
  }

  .picker-search {
    flex: 1;
    min-width: 0;
    height: 100%;
    margin-left: 12px;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--ink);
    font-family: var(--sans);
    font-size: 16px;
    appearance: none;
    box-shadow: none;
  }

  .picker-search::placeholder { color: var(--ink-mute); }
  .picker-search::-webkit-search-cancel-button { appearance: none; }

  .picker-tabs {
    flex-shrink: 0;
    display: flex;
    align-items: stretch;
    gap: 18px;
    height: 38px;
    padding: 0 22px;
    border-bottom: 1px solid var(--line);
    font-size: 13px;
  }

  .picker-rail {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 20px;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .picker-rail::-webkit-scrollbar { display: none; }

  .rail-item,
  .tab-tool {
    position: relative;
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--ink-mute);
    font: inherit;
    white-space: nowrap;
    cursor: pointer;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .rail-item:hover,
  .tab-tool:hover:not(:disabled),
  .rail-active { color: var(--ink); }

  .rail-ink {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 1px;
    height: 1px;
    background: var(--ink);
    transform-origin: left;
    pointer-events: none;
  }

  .rail-ink-ready { transition: transform var(--ui-duration-base) var(--ui-ease-out); }

  .tab-icon {
    display: grid;
    place-items: center;
  }

  .tab-add {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .tab-tool:disabled { opacity: 0.4; cursor: default; }
  .spinning svg { animation: picker-spin 0.9s linear infinite; }
  @keyframes picker-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .spinning svg { animation: none; }
    .rail-ink-ready,
    .row-main,
    .row-provider,
    .row-acts { transition: none; }
  }

  .rail-item:focus-visible,
  .tab-tool:focus-visible,
  .row-main:focus-visible,
  .row-tool:focus-visible,
  .list-more:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 6px;
  }

  .picker-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 10px 10px;
    scrollbar-gutter: stable;
  }

  .model-row {
    position: relative;
    display: flex;
    align-items: center;
    min-height: 38px;
    padding-right: 12px;
    border-radius: 9px;
    transition: background var(--ui-duration-fast) var(--ui-ease-out);
  }

  .model-row:hover,
  .model-row:focus-within { background: var(--control-hover); }
  .row-dim .row-name { color: var(--ink-faint); }

  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    align-self: stretch;
    padding: 0;
    border: none;
    background: none;
    color: var(--ink);
    font-family: var(--sans);
    font-size: 14px;
    text-align: left;
    cursor: pointer;
  }

  .row-num {
    flex-shrink: 0;
    width: 34px;
    text-align: center;
    font-size: 12px;
    color: var(--accent);
  }

  .row-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row-pinned .row-name { font-weight: 500; }

  .row-meta {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    font-size: 12px;
    color: var(--ink-mute);
  }

  .row-tool {
    padding: 0;
    border: none;
    background: none;
    color: var(--ink);
    font: inherit;
    cursor: pointer;
  }

  .row-tool:hover { text-decoration: underline; text-underline-offset: 3px; }
  .row-tool-strong { color: var(--ink); }

  .row-swap {
    display: grid;
    justify-items: end;
    align-items: center;
  }

  .row-swap > * { grid-area: 1 / 1; }

  .row-provider,
  .row-acts { transition: opacity var(--ui-duration-fast) var(--ui-ease-out); }

  .row-acts {
    display: flex;
    gap: 14px;
    opacity: 0;
    pointer-events: none;
  }

  .model-row:hover .row-provider,
  .model-row:focus-within .row-provider { opacity: 0; }

  .model-row:hover .row-acts,
  .model-row:focus-within .row-acts {
    opacity: 1;
    pointer-events: auto;
  }

  @media (hover: none) {
    .row-acts { display: none; }
    .model-row:hover .row-provider { opacity: 1; }
  }

  .row-main { transition: transform var(--ui-duration-fast) var(--ui-ease-out); }
  .row-main:active { transform: scale(0.995); }

  .row-bar {
    position: absolute;
    left: 34px;
    right: 12px;
    bottom: 3px;
    height: 1px;
    background: var(--line);
  }

  .row-bar i {
    display: block;
    height: 1px;
    background: var(--accent);
    transition: width var(--ui-duration-base) var(--ui-ease-out);
  }

  .list-sep {
    height: 1px;
    margin: 6px 12px;
    background: var(--line);
  }

  .list-more {
    display: block;
    width: 100%;
    min-height: 38px;
    padding: 0 12px 0 34px;
    border: none;
    border-radius: 9px;
    background: none;
    color: var(--ink-mute);
    font-family: var(--sans);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }

  .list-more:hover { background: var(--control-hover); color: var(--ink); }
  .picker-close {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: 8px;
    background: none;
    color: var(--ink-mute);
    cursor: pointer;
    transition: color var(--ui-duration-fast) var(--ui-ease-out), background var(--ui-duration-fast) var(--ui-ease-out);
  }

  .picker-close:hover { background: var(--control-hover); color: var(--ink); }
  .picker-close:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

  .catalog-note {
    flex-shrink: 0;
    padding: 8px 22px;
    border-bottom: 1px solid var(--line);
  }

  .picker-foot {
    flex-shrink: 0;
    padding: 0 22px;
    border-top: 1px solid var(--line);
  }

  .custom-input {
    width: 100%;
    height: 44px;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--ink);
    font-family: var(--mono);
    font-size: 13px;
  }

  .custom-input::placeholder { color: var(--ink-mute); font-family: var(--sans); }
  .custom-invalid { color: var(--danger); }
  .picker-foot .list-error { padding: 0 0 8px; }

  .list-note,
  .list-empty,
  .list-error {
    margin: 0;
    padding: 8px 12px 8px 34px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-mute);
  }

  .list-note { display: flex; align-items: baseline; justify-content: space-between; gap: 14px; }
  .list-empty { padding-top: 56px; text-align: center; font-size: 13px; }
  .list-error { color: var(--danger); }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  @media (max-height: 520px) {
    .picker-card { height: calc(100vh - 16px); }
    .picker-search-row { height: 46px; }
  }

  @media (max-width: 640px) {
    .picker-search-row,
    .picker-tabs { padding-left: 16px; padding-right: 16px; }
    .picker-tabs { gap: 14px; }
    .picker-rail { gap: 16px; }
    .tab-add-label { display: none; }
  }
</style>
