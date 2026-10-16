<script lang="ts">
  import ContextIcon from '../ContextIcon.svelte';
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import { startPolling } from '../../polling';
  import { MEMORY_BASELINE_KEY, MEMORY_BASELINE_SAMPLES, MEMORY_SAMPLE_INTERVAL_MS,
    parseMemoryBaseline, sampleMemoryBaseline, memoryMeterPercent } from '../../memoryBaseline';
  import { appStore } from '../../stores';
  import { icons } from '../../icons';
  import { isAndroid, isMac, isWindows } from '../../platform';
  import { MOTION_MS, SETTINGS_SECTION_ORDER, directionFromOrder, motionMs, motionPx } from '../../motion';
  import { visibleSettingsSections, type SettingsSectionId } from '../../settingsSections';
  import {
    requestSettingsSearchNavigation,
    searchSettings,
    type SettingsSearchEntry,
  } from '../../settingsSearch.svelte';
  import Brand from './Brand.svelte';
  import AppIcon from '../AppIcon.svelte';
  import SiteIcon from '../SiteIcon.svelte';
  import { classifyIpcError } from '../../errors';
  import {
    contextsStore,
    loadContexts,
    orderedContexts,
    selectContext,
    isPinned,
    compactAge,
    EVERYWHERE_ID,
  } from '../../contextsStore.svelte';
  import type { Context } from '../../stores';
  import LocalDownloadProgress from '../settings/LocalDownloadProgress.svelte';
  import {
    getActiveDownloads,
    downloadUi,
    cancelDownload,
    acknowledgeDownloads,
  } from '../../downloadManager.svelte';
  import { tweened } from 'svelte/motion';
  import { cubicOut, expoOut } from 'svelte/easing';
  import { fade, fly, slide } from 'svelte/transition';
  import { flip } from 'svelte/animate';

  let rawMemoryMb = $state(0);
  let memoryBaseline = $state(parseMemoryBaseline(null));
  let memoryDir = $state(1);
  let memoryMb = tweened(0, { duration: motionMs(800), easing: expoOut });

  onMount(() => {
    let active = true;
    let lastSampleAt = -Infinity;
    try { memoryBaseline = parseMemoryBaseline(localStorage.getItem(MEMORY_BASELINE_KEY)); } catch {}
    const refresh = async () => {
      try {
        const next = await invoke<number>('get_memory_mb');
        if (!active || !Number.isFinite(next) || next <= 0) return;
        const now = performance.now();
        if (now - lastSampleAt >= MEMORY_SAMPLE_INTERVAL_MS) {
          const updated = sampleMemoryBaseline(memoryBaseline, next);
          lastSampleAt = now;
          if (updated !== memoryBaseline) {
            memoryBaseline = updated;
            try { localStorage.setItem(MEMORY_BASELINE_KEY, JSON.stringify(updated)); } catch {}
          }
        }
        if (active && next !== rawMemoryMb) {
          memoryDir = next > rawMemoryMb ? 1 : -1;
          rawMemoryMb = next;
          memoryMb.set(next);
        }
      } catch {}
    };
    const poll = startPolling(refresh, MEMORY_SAMPLE_INTERVAL_MS);
    return () => { active = false; poll.stop(); };
  });

  const HOME_NAV_ITEMS = [
    { id: 'home',       label: 'Home',       icon: 'home',  locked: false },
    { id: 'insights',   label: 'Insights',   icon: 'chart', locked: false },
  ] as const;

  const STYLE_NAV_ITEMS = [
    { id: 'style',      label: 'Style',      icon: 'pencil', locked: false },
  ] as const;

  const LEGACY_NAV_ITEMS = [
    { id: 'dictionary', label: 'Dictionary', icon: 'book',     locked: false },
    { id: 'snippets',   label: 'Snippets',   icon: 'scissors', locked: false },
  ] as const;

  const navItems = $derived(
    appStore.legacyFeaturesEnabled
      ? [...HOME_NAV_ITEMS, ...LEGACY_NAV_ITEMS, ...STYLE_NAV_ITEMS]
      : [...HOME_NAV_ITEMS, ...STYLE_NAV_ITEMS]
  );

  const settingsGroups = $derived(
    visibleSettingsSections({ isMac, devMode: appStore.devModeEnabled, legacyMode: appStore.legacyFeaturesEnabled, syncEnabled: appStore.syncEnabled })
  );

  type RailEntry =
    | { kind: 'label'; key: string; label: string }
    | { kind: 'section'; key: string; id: SettingsSectionId; label: string; icon: keyof typeof icons };

  const settingsEntries = $derived<RailEntry[]>(
    settingsGroups.flatMap((group) => [
      { kind: 'label' as const, key: `label:${group.group}`, label: group.group },
      ...group.items.map((item) => ({
        kind: 'section' as const,
        key: `section:${item.id}`,
        id: item.id,
        label: item.label,
        icon: item.icon,
      })),
    ])
  );

  let settingsQuery = $state('');
  const settingsSearchResults = $derived(
    searchSettings(
      settingsQuery,
      settingsGroups.flatMap((group) => group.items.map((item) => item.id)),
    ),
  );
  const settingsSearchGroups = $derived.by(() => {
    const resultsBySection = new Map<SettingsSectionId, SettingsSearchEntry[]>();
    for (const result of settingsSearchResults) {
      const results = resultsBySection.get(result.section) ?? [];
      results.push(result);
      resultsBySection.set(result.section, results);
    }

    return settingsGroups.flatMap((group) =>
      group.items.flatMap((item) => {
        const results = resultsBySection.get(item.id);
        return results?.length ? [{ section: item, results }] : [];
      }),
    );
  });

  const RAIL_IN_MS = 260;
  const RAIL_OUT_MS = 180;
  const RAIL_IN_DELAY_MS = 40;
  const RAIL_TRAVEL_PX = 10;
  const RAIL_STAGGER_MS = 20;
  const RAIL_OUT_STAGGER_MS = 9;
  const RAIL_STAGGER_CAP = 6;

  function railDelay(index: number, base: number, step = RAIL_STAGGER_MS): number {
    return motionMs(base + Math.min(index, RAIL_STAGGER_CAP) * step);
  }

  function searchResultReorder(
    _node: Element,
    { from, to }: { from: DOMRect; to: DOMRect },
  ) {
    const dx = from.left - to.left;
    const dy = from.top - to.top;
    return {
      duration: motionMs(180),
      easing: cubicOut,
      css: (t: number) => {
        const u = 1 - t;
        return `transform: translate3d(${u * dx}px, ${u * dy}px, 0);`;
      },
    };
  }

  function nav(id: string) {
    if (id === 'settings') { appStore.settingsOpen = true; return; }
    appStore.currentPage = id as typeof appStore.currentPage;
  }

  function goToSection(id: SettingsSectionId) {
    if (id === appStore.settingsSection) return;
    appStore.settingsAnimDir = directionFromOrder(
      appStore.settingsSection,
      id,
      SETTINGS_SECTION_ORDER
    );
    appStore.settingsSection = id;
  }

  function openSettingsSearchResult(result: SettingsSearchEntry) {
    goToSection(result.section);
    requestSettingsSearchNavigation(result);
    settingsQuery = '';
  }

  function backToApp() { appStore.settingsOpen = false; }

  const activeDownloads = $derived(getActiveDownloads());
  const doneDownloads = $derived.by(() => {
    const activeKeys = new Set(activeDownloads.map((item) => item.key));
    return downloadUi.completed.filter((entry) => !activeKeys.has(entry.key));
  });
  const showDownloadPanel = $derived(activeDownloads.length > 0 || doneDownloads.length > 0);

  let prevSettingsOpen = appStore.settingsOpen;
  $effect(() => {
    const open = appStore.settingsOpen;
    if (prevSettingsOpen && !open) acknowledgeDownloads();
    prevSettingsOpen = open;
  });

  let sidebarEl = $state<HTMLElement | null>(null);
  let navSectionEl = $state<HTMLElement | null>(null);
  let pillTop = $state(0);
  let pillHeight = $state(0);
  let pillSnap = $state(true);

  function activeRailButton(): HTMLElement | null {
    if (!sidebarEl) return null;
    const selector = appStore.settingsOpen
      ? '.rail-list .settings-nav-item.active'
      : '.rail-list .nav-item.active';
    return sidebarEl.querySelector<HTMLElement>(selector);
  }

  function movePillTo(el: HTMLElement | null, { snap = false } = {}) {
    if (!el || !sidebarEl) return;
    if (snap) pillSnap = true;
    const itemRect = el.getBoundingClientRect();
    const railRect = navSectionEl?.getBoundingClientRect() ?? sidebarEl.getBoundingClientRect();
    pillTop = itemRect.top - railRect.top;
    pillHeight = itemRect.height;
    if (snap) {
      requestAnimationFrame(() => { pillSnap = false; });
    } else {
      pillSnap = false;
    }
  }

  function openSettings(event: MouseEvent) {
    movePillTo(event.currentTarget as HTMLElement, { snap: true });
    settingsQuery = '';
    appStore.settingsOpen = true;
  }

  $effect(() => {
    appStore.settingsOpen;
    appStore.currentPage;
    appStore.settingsSection;
    settingsEntries;
    settingsSearchResults;
    requestAnimationFrame(() => movePillTo(activeRailButton()));
  });

  const showContexts = $derived(!appStore.legacyFeaturesEnabled);
  const contextRows = $derived(orderedContexts(contextsStore.contexts));
  let ctxSectionEl = $state<HTMLElement | null>(null);
  let ctxWasInSettings = appStore.settingsOpen;
  let ctxEntrance: Animation | null = null;

  $effect(() => {
    const open = appStore.settingsOpen;
    if (open) {
      ctxEntrance?.cancel();
      ctxEntrance = null;
    } else if (ctxWasInSettings) {
      if (ctxSectionEl) {
        ctxEntrance = ctxSectionEl.animate(
          [
            { opacity: 0, transform: `translate3d(${-motionPx(12)}px, 0, 0)` },
            { opacity: 1, transform: 'translate3d(0, 0, 0)' },
          ],
          {
            duration: motionMs(280),
            easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
          },
        );
      }
    }
    ctxWasInSettings = open;
    return () => {
      ctxEntrance?.cancel();
      ctxEntrance = null;
    };
  });

  onMount(() => { void loadContexts(); });

  const STACK_COMPACT = 4;
  const STACK_EXPANDED = 8;
  const STACK_REVEAL_STEP = 35;

  type StackEntry = { key: string; kind: 'app' | 'site'; value: string };

  function stackEntries(contextId: number): StackEntry[] {
    return [
      ...contextsStore.targets
        .filter((target) => target.context_id === contextId)
        .map((target) => ({ key: `app:${target.id}`, kind: 'app' as const, value: target.executable })),
      ...contextsStore.websites
        .filter((site) => site.context_id === contextId)
        .map((site) => ({ key: `site:${site.id}`, kind: 'site' as const, value: site.domain })),
    ];
  }

  const pinnedRows = $derived(contextRows.filter((context) => isPinned(context)));
  const unpinnedRows = $derived(contextRows.filter((context) => !isPinned(context)));

  function openContext(id: number) {
    selectContext(id);
    appStore.currentPage = 'contexts';
    closeContextMenu();
  }

  function createContext() {
    contextsStore.modalRequest = { mode: 'create' };
    appStore.currentPage = 'contexts';
    closeContextMenu();
  }

  let contextMenu = $state<{ id: number; top: number; left: number; alignRight: boolean } | null>(null);
  let contextMenuEl = $state<HTMLElement | null>(null);
  let contextDeleteArmed = $state(false);
  let contextError = $state('');
  const MENU_VIEWPORT_MARGIN = 8;

  const menuContext = $derived(
    contextMenu ? contextsStore.contexts.find((c) => c.id === contextMenu!.id) ?? null : null
  );

  function closeContextMenu() {
    contextMenu = null;
    contextDeleteArmed = false;
  }

  function openContextMenu(id: number, top: number, left: number, alignRight: boolean) {
    if (contextMenu?.id === id) {
      closeContextMenu();
      return;
    }
    contextMenu = { id, top, left, alignRight };
    contextDeleteArmed = false;
  }

  function toggleContextMenuFromKebab(id: number, trigger: HTMLElement) {
    const rect = trigger.getBoundingClientRect();
    openContextMenu(id, rect.bottom + 4, rect.right, true);
  }

  function handleContextRowContextMenu(id: number, event: MouseEvent) {
    event.preventDefault();
    contextMenu = null;
    openContextMenu(id, event.clientY + 4, event.clientX, false);
  }

  $effect(() => {
    const el = contextMenuEl;
    const menu = contextMenu;
    if (!el || !menu) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const desiredLeft = menu.alignRight ? menu.left - width : menu.left;
    const left = Math.max(
      MENU_VIEWPORT_MARGIN,
      Math.min(desiredLeft, window.innerWidth - width - MENU_VIEWPORT_MARGIN),
    );
    const top = Math.max(
      MENU_VIEWPORT_MARGIN,
      Math.min(menu.top, window.innerHeight - height - MENU_VIEWPORT_MARGIN),
    );
    if (left === menu.left && top === menu.top && !menu.alignRight) return;
    contextMenu = { ...menu, left, top, alignRight: false };
  });

  $effect(() => {
    if (!contextMenu) return;
    const handleClose = (event: Event) => {
      if (event.target instanceof Element && event.target.closest('.ctx-kebab, .ctx-menu')) return;
      closeContextMenu();
    };
    const timeout = window.setTimeout(() => {
      window.addEventListener('pointerdown', handleClose);
      window.addEventListener('scroll', handleClose, { capture: true, passive: true });
    });
    return () => {
      window.clearTimeout(timeout);
      window.removeEventListener('pointerdown', handleClose);
      window.removeEventListener('scroll', handleClose, { capture: true });
    };
  });

  function editContext(context: Context) {
    contextsStore.selectedId = context.id;
    contextsStore.modalRequest = { mode: 'edit', id: context.id };
    appStore.currentPage = 'contexts';
    closeContextMenu();
  }

  function editContextById(id: number) {
    const context = contextsStore.contexts.find((candidate) => candidate.id === id);
    if (context) editContext(context);
  }

  function duplicateContextName(sourceName: string) {
    const names = new Set(contextsStore.contexts.map((context) => context.name.toLowerCase()));
    for (let copyNumber = 1; copyNumber < 10_000; copyNumber += 1) {
      const suffix = copyNumber === 1 ? ' copy' : ` copy ${copyNumber}`;
      const baseLength = Math.max(1, 30 - [...suffix].length);
      const base = [...sourceName].slice(0, baseLength).join('').trimEnd() || 'Context';
      const candidate = `${base}${suffix}`;
      if (!names.has(candidate.toLowerCase())) return candidate;
    }
    throw new Error('Could not find an available name for the duplicated context');
  }

  async function duplicateContext(context: Context) {
    if (context.is_everywhere) return;
    closeContextMenu();
    try {
      const [dictionary, snippets] = await Promise.all([
        invoke<{ id: number }[]>('get_context_dictionary', { contextId: context.id }),
        invoke<{ id: number }[]>('get_context_snippets', { contextId: context.id }),
      ]);
      const duplicate = await invoke<Context>('create_context', {
        name: duplicateContextName(context.name),
        icon: context.icon,
        tone: context.tone,
        cleanupIntensity: context.cleanup_intensity,
        customInstructions: context.custom_instructions,
        contextualFormattingDisabled: context.contextual_formatting_disabled,
      });
      try {
        if (context.color) {
          await invoke('update_context_color', { contextId: duplicate.id, color: context.color });
          duplicate.color = context.color;
        }
        await Promise.all([
          ...dictionary.map((entry) => invoke('set_dictionary_context_assignment', {
            contextId: duplicate.id,
            dictionaryId: entry.id,
            assigned: true,
          })),
          ...snippets.map((snippet) => invoke('set_snippet_context_assignment', {
            contextId: duplicate.id,
            snippetId: snippet.id,
            assigned: true,
          })),
        ]);
      } catch (error) {
        await invoke('delete_context', { contextId: duplicate.id }).catch(() => undefined);
        throw error;
      }
      contextsStore.contexts = [...contextsStore.contexts, duplicate];
      selectContext(duplicate.id);
      appStore.currentPage = 'contexts';
    } catch (error) {
      contextError = classifyIpcError(error).message;
    }
  }

  async function togglePin(context: Context) {
    const pinned = !context.pinned_at;
    closeContextMenu();
    try {
      await invoke('set_context_pinned', { contextId: context.id, pinned });
      const contexts = await invoke<Context[]>('get_contexts');
      contextsStore.contexts = contexts ?? [];
    } catch (error) {
      contextError = classifyIpcError(error).message;
    }
  }

  async function deleteContext(context: Context) {
    if (context.is_everywhere) return;
    if (!contextDeleteArmed) {
      contextDeleteArmed = true;
      return;
    }
    try {
      await invoke('delete_context', { contextId: context.id });
      contextsStore.contexts = contextsStore.contexts.filter((c) => c.id !== context.id);
      contextsStore.targets = contextsStore.targets.filter((t) => t.context_id !== context.id);
      contextsStore.websites = contextsStore.websites.filter((w) => w.context_id !== context.id);
      if (contextsStore.selectedId === context.id) contextsStore.selectedId = EVERYWHERE_ID;
      window.dispatchEvent(new Event('verenu:context-saved'));
    } catch (error) {
      contextError = classifyIpcError(error).message;
    } finally {
      closeContextMenu();
    }
  }
</script>

<aside class="sidebar" class:rail-settings={appStore.settingsOpen} class:sidebar-windows={isWindows} bind:this={sidebarEl}>
  <Brand />

  <div class="nav-section" bind:this={navSectionEl}>
    <div
      class="rail-pill"
      class:rail-pill-snap={pillSnap}
      class:rail-pill-hidden={
        (!appStore.settingsOpen && appStore.currentPage === 'contexts') ||
        (appStore.settingsOpen && Boolean(settingsQuery.trim()))
      }
      style="top:{pillTop}px; height:{pillHeight}px"
    ></div>

    {#if appStore.settingsOpen}
      <div class="settings-rail-content" class:searching={Boolean(settingsQuery.trim())}>
        <div
          class="settings-search"
          in:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_IN_MS), delay: 0, easing: cubicOut }}
          out:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_OUT_MS), delay: 0, easing: cubicOut }}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></svg>
          <input type="search" bind:value={settingsQuery} placeholder="Search settings..." aria-label="Search settings" />
          {#if settingsQuery}
            <button
              type="button"
              class="settings-search-clear"
              aria-label="Clear settings search"
              onclick={() => settingsQuery = ''}
              in:fly={{ x: motionPx(3), duration: motionMs(MOTION_MS.fast), easing: cubicOut }}
              out:fly={{ x: motionPx(3), duration: motionMs(120), easing: cubicOut }}
            >×</button>
          {/if}
        </div>
        {#if settingsQuery.trim()}
          <div
            class="settings-search-results scroll-styled"
            aria-live="polite"
          >
            {#each settingsSearchGroups as group (group.section.id)}
              <div
                class="settings-search-group"
                animate:searchResultReorder
                in:fade={{ duration: motionMs(120) }}
                out:fade={{ duration: motionMs(120) }}
              >
                <div class="settings-search-section">{group.section.label}</div>
                {#each group.results as result (result.id)}
                  <button
                    type="button"
                    class="settings-search-result"
                    onclick={() => openSettingsSearchResult(result)}
                    animate:searchResultReorder
                    in:fade={{ duration: motionMs(120) }}
                    out:fade={{ duration: motionMs(120) }}
                  >
                    <span class="settings-search-result-title">{result.label}</span>
                    <span class="settings-search-result-hint">Open in {group.section.label}</span>
                  </button>
                {/each}
              </div>
            {/each}
            {#if settingsSearchResults.length === 0}
              <p class="settings-search-empty">No matching settings</p>
            {/if}
          </div>
        {:else}
          <div class="rail-list rail-list-settings">
            {#each settingsEntries as entry, i (entry.key)}
              {#if entry.kind === 'label'}
                <div
                  class="settings-section-label"
                  in:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_IN_MS), delay: 0, easing: cubicOut }}
                  out:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_OUT_MS), delay: 0, easing: cubicOut }}
                >{entry.label}</div>
              {:else}
                <button
                  type="button"
                  class="settings-nav-item"
                  class:active={appStore.settingsSection === entry.id}
                  data-debug-id={`settings.${entry.id}`}
                  onclick={() => goToSection(entry.id)}
                  in:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_IN_MS), delay: 0, easing: cubicOut }}
                  out:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_OUT_MS), delay: 0, easing: cubicOut }}
                >
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={appStore.settingsSection === entry.id ? '2.2' : '1.6'} stroke-linecap="round" stroke-linejoin="round">{@html icons[entry.icon]}</svg>
                  <span>{entry.label}</span>
                  {#if entry.id === 'advanced' && import.meta.env.DEV}
                    <span class="legacy-label" aria-hidden="true">Microphone</span>
                  {/if}
                </button>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <div class="rail-list rail-list-app">
        {#each navItems as entry, i (entry.id)}
          <button
            type="button"
            class="nav-item"
            class:active={appStore.currentPage === entry.id}
            data-debug-id={`nav.${entry.id}`}
            disabled={entry.locked}
            onclick={() => nav(entry.id)}
            in:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_IN_MS), delay: railDelay(i, RAIL_IN_DELAY_MS), easing: cubicOut }}
            out:fly|global={{ x: -motionPx(RAIL_TRAVEL_PX), duration: motionMs(RAIL_OUT_MS), delay: railDelay(i, 0, RAIL_OUT_STAGGER_MS), easing: cubicOut }}
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={appStore.currentPage === entry.id ? '2.2' : '1.6'} stroke-linecap="round" stroke-linejoin="round">{@html icons[entry.icon]}</svg>
            <span>{entry.label}</span>
            {#if entry.locked}
              <span class="lock-tag">Soon</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  {#if showContexts}
    <div bind:this={ctxSectionEl} class="ctx-section" class:ctx-section-hidden={appStore.settingsOpen} inert={appStore.settingsOpen}>
      <div class="ctx-head">
        <span class="ctx-head-label">Contexts</span>
        <span class="ctx-head-rule" aria-hidden="true"></span>
        <button type="button" class="ctx-add" aria-label="New context group" title="New context group" onclick={createContext}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        </button>
      </div>

      {#if contextError}
        <p class="ctx-error" role="alert">{contextError}</p>
      {/if}

      {#snippet contextRow(context: Context)}
        {@const pinned = isPinned(context)}
        {@const stack = pinned ? stackEntries(context.id) : []}
        {@const shown = stack.slice(0, STACK_EXPANDED)}
        {@const restCount = stack.length - STACK_EXPANDED}
            <button
              type="button"
              class="ctx-row"
              class:has-stack={stack.length > 0}
              class:active={appStore.currentPage === 'contexts' && contextsStore.selectedId === context.id}
              data-debug-id={`context.${context.id}`}
              onclick={() => openContext(context.id)}
            >
              <span class="ctx-icon" style={context.color ? `color: ${context.color}` : ''} aria-hidden="true">
                {#if context.is_everywhere}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><circle cx="12" cy="12" r="8"/><path d="M4 12h16M12 4c2 2.3 3 5 3 8s-1 5.7-3 8c-2-2.3-3-5-3-8s1-5.7 3-8Z"/></svg>
                {:else}
                  <ContextIcon icon={context.icon} />
                {/if}
              </span>
              <span class="ctx-name">{context.name}</span>
              {#if stack.length > 0}
                <span class="ctx-stack" aria-hidden="true">
                  {#each shown as entry, iconIndex (entry.key)}
                    <span
                      class="ctx-stack-item"
                      class:is-extra={iconIndex >= STACK_COMPACT}
                      style={iconIndex >= STACK_COMPACT ? `--reveal-delay: ${(iconIndex - STACK_COMPACT) * STACK_REVEAL_STEP}ms` : ''}
                    >
                      {#if entry.kind === 'app'}
                        <AppIcon exe={entry.value} size={15} />
                      {:else}
                        <SiteIcon domain={entry.value} size={15} />
                      {/if}
                    </span>
                  {/each}
                  {#if stack.length > STACK_COMPACT}
                    <span class="ctx-stack-more">+{stack.length - STACK_COMPACT}</span>
                  {/if}
                  {#if restCount > 0}
                    <span class="ctx-stack-more ctx-stack-more-rest">+{restCount}</span>
                  {/if}
                </span>
              {/if}
            </button>

            <span class="ctx-tail">
              {#if pinned}
                <span class="ctx-pin" title="Pinned">
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M18 3v2h-1v6l2 3v2h-6v5h-2v-5H5v-2l2-3V5H6V3z"/></svg>
                </span>
              {/if}
              <span class="ctx-age">{compactAge(context.created_at)}</span>
              <button
                  type="button"
                  class="ctx-kebab"
                  aria-label={`More actions for ${context.name}`}
                  aria-haspopup="menu"
                  aria-expanded={contextMenu?.id === context.id}
                  onclick={(event) => toggleContextMenuFromKebab(context.id, event.currentTarget)}
                >
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>
                </button>
            </span>
      {/snippet}

      <div class="ctx-list">
        {#each pinnedRows as context (context.id)}
          <div
            class="ctx-row-wrap"
            class:menu-open={contextMenu?.id === context.id}
            oncontextmenu={(event) => handleContextRowContextMenu(context.id, event)}
            role="presentation"
            animate:flip={{ duration: motionMs(300), easing: cubicOut }}
            in:slide={{ duration: motionMs(200), easing: cubicOut }}
            out:slide={{ duration: motionMs(160), easing: cubicOut }}
          >
            {@render contextRow(context)}
          </div>
        {/each}

        {#if unpinnedRows.length > 0 && pinnedRows.length > 0}
          <div class="ctx-subhead">
            <span class="ctx-subhead-label">More contexts</span>
            <span class="ctx-subhead-rule" aria-hidden="true"></span>
          </div>
        {/if}
        {#each unpinnedRows as context (context.id)}
          <div
            class="ctx-row-wrap"
            class:menu-open={contextMenu?.id === context.id}
            oncontextmenu={(event) => handleContextRowContextMenu(context.id, event)}
            role="presentation"
            animate:flip={{ duration: motionMs(300), easing: cubicOut }}
            in:slide={{ duration: motionMs(200), easing: cubicOut }}
            out:slide={{ duration: motionMs(160), easing: cubicOut }}
          >
            {@render contextRow(context)}
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <div class="sidebar-spacer"></div>
  {/if}

  {#if showDownloadPanel}
    <div
      class="dl-panel"
      in:fly={{ x: -motionPx(14), duration: motionMs(260), easing: cubicOut }}
      out:slide={{ duration: motionMs(200), easing: cubicOut }}
    >
      {#each activeDownloads as item (item.key)}
        <div class="dl-item" in:slide={{ duration: motionMs(200), easing: cubicOut }}>
          <div class="dl-item-top">
            <span class="dl-item-name" title={item.name}>{item.name}</span>
            <button
              type="button"
              class="dl-cancel"
              aria-label={`Cancel ${item.name} download`}
              title="Cancel download"
              onclick={() => cancelDownload(item)}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
            </button>
          </div>
          <LocalDownloadProgress
            stage={item.stage}
            label={item.label}
            percent={item.percent}
            indeterminate={item.indeterminate}
          />
        </div>
      {/each}
      {#each doneDownloads as entry (entry.key)}
        <div class="dl-done" in:slide={{ duration: motionMs(200), easing: cubicOut }}>
          <span class="dl-dot" aria-hidden="true"></span>
          <span class="dl-done-name" title={entry.name}>{entry.name} ready</span>
        </div>
      {/each}
    </div>
  {/if}

  <div class="sidebar-foot">
    <button
      type="button"
      class={appStore.settingsOpen ? 'settings-back' : 'nav-item'}
      data-debug-id={appStore.settingsOpen ? 'nav.back' : 'nav.settings'}
      onclick={appStore.settingsOpen ? backToApp : openSettings}
    >
      {#if appStore.settingsOpen}
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 18l-6-6 6-6"/></svg>
        <span>Back to app</span>
      {:else}
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{@html icons.settings}</svg>
        <span>Settings</span>
      {/if}
    </button>
  </div>

  {#if !isAndroid}
  <div class="local-bar">
    <div class="local-bar-row">
      <span class="local-dot"></span>
      <span>Running locally</span>
      <div class="meta-wrapper">
        <span class="meta">
          {#each String(rawMemoryMb).split('') as digit, i (i)}
            <span class="digit-slot">
              {#key digit}
                <span
                  class="digit-char"
                  in:fly={{ y: memoryDir * 10, duration: motionMs(400), easing: expoOut }}
                  out:fly={{ y: -memoryDir * 10, duration: motionMs(400), easing: expoOut }}
                >{digit}</span>
              {/key}
            </span>
          {/each}<span class="meta-unit"> MB</span>
        </span>
      </div>
    </div>
    <div class="local-meter-thin"
      title={memoryBaseline.samples === 0 ? 'Waiting for memory usage' : `Memory usage relative to your ${memoryBaseline.samples < MEMORY_BASELINE_SAMPLES ? 'learning' : 'saved'} average (${Math.round(memoryBaseline.averageMb)} MB). Average usage fills half the bar.`}
    ><span style="width:{memoryMeterPercent($memoryMb, memoryBaseline.averageMb)}%; background:{$memoryMb >= 150 ? 'var(--accent)' : 'var(--line-strong)'}"></span></div>
  </div>
  {/if}
</aside>

{#if contextMenu && menuContext}
  <div
    class="ui-dropdown-menu ctx-menu"
    role="menu"
    tabindex="-1"
    bind:this={contextMenuEl}
    style="top: {contextMenu.top}px; left: {contextMenu.left}px;"
    onpointerdown={(event) => event.stopPropagation()}
    in:fly={{ y: motionPx(6), duration: motionMs(MOTION_MS.fast) }}
    out:fly={{ y: motionPx(4), duration: motionMs(120) }}
  >
    <button class="ui-dropdown-option" type="button" role="menuitem" onclick={() => editContextById(menuContext.id)}>Edit</button>
    {#if !menuContext.is_everywhere}
      <button class="ui-dropdown-option" type="button" role="menuitem" onclick={() => void duplicateContext(menuContext)}>Duplicate</button>
    {/if}
    <button class="ui-dropdown-option" type="button" role="menuitem" onclick={() => void togglePin(menuContext)}>
      {menuContext.pinned_at ? 'Unpin' : 'Pin'}
    </button>
    {#if !menuContext.is_everywhere}
      <button
        class="ui-dropdown-option ctx-menu-delete"
        class:is-armed={contextDeleteArmed}
        type="button"
        role="menuitem"
        onclick={() => void deleteContext(menuContext)}
      >
        {contextDeleteArmed ? 'Confirm delete' : 'Delete'}
      </button>
    {/if}
  </div>
{/if}

<style>
  .sidebar {
    width: var(--sidebar-w);
    background: var(--sidebar-bg);
    border-right: 1px solid var(--line);
    margin-bottom: calc(-1 * var(--app-gutter));
    position: relative;
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    overflow: hidden;
  }

  .sidebar {
    z-index: 61;
  }

  /* Android: the status bar can't be drawn under, so the rail reads as a
     rounded panel sitting below it instead of a slab that is cut off, and it
     clears the gesture bar at the bottom. */
  :global(.app[data-android='true']) .sidebar {
    margin-top: 6px;
    margin-left: 6px;
    border-radius: 22px 22px 0 0;
    border: 1px solid var(--line);
    border-bottom: 0;
    padding-bottom: calc(var(--safe-bottom) + 20px);
  }

  .nav-section {
    padding: 12px 8px 4px;
    display: grid;
    position: relative;
  }

  .sidebar:not(.rail-settings) .nav-section { padding-top: 18px; }
  .rail-settings .nav-section { padding-top: 20px; }

  .rail-list {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  /* Svelte keeps the outgoing branch in the DOM until its outro completes.
     Remove only that inactive rail from layout so the incoming rail determines
     .nav-section's height immediately. Its exit animation still paints from
     the same coordinates. */
  .rail-settings .rail-list-app,
  .sidebar:not(.rail-settings) .rail-list-settings,
  .sidebar:not(.rail-settings) .settings-rail-content {
    position: absolute;
    left: 8px;
    right: 8px;
  }

  .rail-settings .rail-list-app { top: 18px; }
  .sidebar:not(.rail-settings) .rail-list-settings { top: 12px; }
  .sidebar:not(.rail-settings) .settings-rail-content { top: 0; }
  .rail-settings .rail-list-settings { position: static; }

  .settings-rail-content {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    position: relative;
  }

  .settings-rail-content.searching .rail-list-settings {
    visibility: hidden;
    pointer-events: none;
  }

  .settings-search {
    align-items: center;
    display: flex;
    gap: 7px;
    margin: 0 2px;
    min-height: 30px;
    padding: 0 8px;
    border: 1px solid var(--line);
    border-radius: 7px;
    color: var(--ink-faint);
    transition:
      border-color var(--ui-duration-fast) var(--ui-ease-out),
      color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .settings-search:focus-within {
    border-color: var(--line-strong);
    color: var(--ink-mute);
  }

  .settings-search input {
    background: transparent;
    border: 0;
    color: var(--ink);
    flex: 1;
    font: 12px var(--sans);
    min-width: 0;
    outline: 0;
  }

  .settings-search input::placeholder { color: var(--ink-faint); }

  .settings-search input::-webkit-search-cancel-button,
  .settings-search input::-webkit-search-decoration {
    appearance: none;
    -webkit-appearance: none;
  }

  .settings-search input::-ms-clear { display: none; }

  .settings-search-clear {
    background: transparent;
    border: 0;
    color: var(--ink-mute);
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    padding: 0 1px;
    transition:
      color var(--ui-duration-fast) var(--ui-ease-out),
      transform var(--ui-duration-fast) var(--ui-ease-out);
  }

  .settings-search-clear:hover { color: var(--ink); }
  .settings-search-clear:active { transform: scale(0.9); }

  .settings-search-empty {
    color: var(--ink-mute);
    font-size: 11.5px;
    margin: 6px 10px;
  }

  .settings-search-results {
    animation: settings-search-results-enter var(--ui-duration-base) var(--ui-ease-out);
    display: flex;
    flex-direction: column;
    gap: 12px;
    left: 0;
    max-height: calc(100vh - 190px);
    overflow-y: auto;
    padding: 0 2px 8px;
    position: absolute;
    right: 0;
    top: 40px;
  }

  @keyframes settings-search-results-enter {
    from {
      opacity: 0;
      transform: translate3d(-6px, 0, 0);
    }
    to {
      opacity: 1;
      transform: translate3d(0, 0, 0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .settings-search-results { animation: none; }
  }

  .settings-search-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .settings-search-section {
    color: var(--ink-mute);
    font-size: 11px;
    font-weight: 500;
    padding: 2px 8px 4px;
  }

  .settings-search-result {
    width: 100%;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--ink);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 7px 8px;
    text-align: left;
    transition:
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .settings-search-result:hover { background: var(--control-hover); }

  .settings-search-result:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .settings-search-result-title {
    font-size: 12px;
    font-weight: 500;
    line-height: 1.3;
  }

  .settings-search-result-hint {
    color: var(--ink-mute);
    font-size: 10.5px;
    line-height: 1.3;
  }

  .rail-pill {
    position: absolute;
    left: 8px;
    right: 8px;
    border-radius: 7px;
    background: var(--control-hover);
    pointer-events: none;
    transition:
      top var(--rail-pill-ms) cubic-bezier(0.33, 1, 0.68, 1),
      height var(--rail-pill-ms) cubic-bezier(0.33, 1, 0.68, 1);
  }

  .rail-pill.rail-pill-snap { transition: none; }

  .rail-pill.rail-pill-hidden { opacity: 0; }
  .rail-pill { opacity: 1; transition: opacity 160ms ease, top var(--rail-pill-ms) cubic-bezier(0.33, 1, 0.68, 1), height var(--rail-pill-ms) cubic-bezier(0.33, 1, 0.68, 1); }

  .sidebar { --rail-pill-ms: 300ms; }

  @media (prefers-reduced-motion: reduce) {
    .sidebar { --rail-pill-ms: 170ms; }
  }

  .nav-item,
  .settings-nav-item,
  .settings-back {
    border: 0;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 9px;
    min-height: 28px;
    padding: 5px 6px;
    border-radius: 7px;
    color: var(--ink-soft);
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 450;
    user-select: none;
    position: relative;
    text-align: left;
    width: 100%;
  }

  .nav-item :global(svg),
  .settings-nav-item :global(svg),
  .settings-back :global(svg) {
    opacity: 0.75;
    flex-shrink: 0;
    transition: opacity var(--rail-pill-ms) ease, stroke-width var(--rail-pill-ms) ease;
  }

  .sidebar-windows .nav-item :global(svg),
  .sidebar-windows .settings-nav-item :global(svg),
  .sidebar-windows .settings-back :global(svg) {
    width: 15px;
    height: 15px;
  }

  .nav-item:hover,
  .settings-nav-item:hover,
  .settings-back:hover { color: var(--ink-strong); background: var(--control-hover); }

  .nav-item:focus-visible,
  .settings-nav-item:focus-visible,
  .settings-back:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .nav-item.active,
  .settings-nav-item.active {
    color: var(--ink);
    font-weight: 500;
  }
  .nav-item.active :global(svg),
  .settings-nav-item.active :global(svg) { opacity: 1; }

  .settings-section-label {
    font-family: var(--sans);
    font-size: 11px;
    font-weight: 500;
    color: var(--ink-mute);
    padding: 6px 10px 3px;
  }

  .legacy-label {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .nav-item:disabled {
    color: var(--ink-faint);
    cursor: default;
    opacity: 1;
  }
  .nav-item:disabled:hover { background: transparent; color: var(--ink-faint); }
  .nav-item:disabled :global(svg) { opacity: 0.5; }

  .lock-tag {
    margin-left: auto;
    font-family: var(--sans);
    font-size: 9px;
    color: var(--ink-mute);
    padding: 1px 6px;
    border-radius: 999px;
    font-weight: 500;
    letter-spacing: 0.04em;
    border: 1px solid var(--line);
  }

  .sidebar-spacer { flex: 1; }

  .ctx-section {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 10px 8px 0;
    opacity: 1;
    transform: translateX(0);
    visibility: visible;
  }

  .ctx-section-hidden {
    visibility: hidden;
    pointer-events: none;
    opacity: 0;
    transform: translateX(-12px);
  }

  .ctx-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px 4px;
  }

  .ctx-head-label {
    flex: none;
    font-size: 10.5px;
    font-weight: 550;
    color: var(--ink-mute);
  }

  .ctx-head-rule {
    flex: 1;
    min-width: 8px;
    height: 1px;
    background: var(--line-soft);
  }

  .ctx-add {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-mute);
    cursor: pointer;
    transition: background-color 140ms ease, color 140ms ease;
  }
  .ctx-add:hover { background: var(--control-hover); color: var(--ink-strong); }
  .ctx-add:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }

  .ctx-error {
    margin: 0 6px 6px;
    font-size: 10.5px;
    color: var(--danger);
  }

  .ctx-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    display: flex;
    flex-direction: column;
    gap: 1px;
    scrollbar-width: thin;
    scrollbar-color: var(--line-strong) transparent;
  }
  .ctx-list::-webkit-scrollbar { width: 6px; }
  .ctx-list { padding-bottom: 4px; }
  .ctx-list::-webkit-scrollbar-track { background: transparent; }
  .ctx-list::-webkit-scrollbar-thumb { background: var(--line); border-radius: 999px; }
  .ctx-list:hover::-webkit-scrollbar-thumb { background: var(--line-strong); }

  .ctx-row-wrap { position: relative; }

  .ctx-subhead {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 14px 6px 4px;
  }

  .ctx-subhead-label {
    flex: none;
    font-size: 10px;
    font-weight: 500;
    color: var(--ink-faint);
    opacity: 0.9;
  }

  .ctx-subhead-rule {
    flex: 1;
    min-width: 8px;
    height: 1px;
    background: color-mix(in srgb, var(--line-soft) 70%, transparent);
  }

  .ctx-row {
    width: 100%;
    border: 0;
    background: transparent;
    border-radius: 7px;
    color: var(--ink-soft);
    cursor: pointer;
    user-select: none;
    text-align: left;
    display: grid;
    grid-template-columns: var(--ctx-gutter) minmax(0, 1fr);
    column-gap: var(--ctx-gutter-gap);
    row-gap: 1px;
    align-items: center;
    padding: var(--ctx-row-pad-y) 40px var(--ctx-row-pad-y) var(--ctx-row-pad-x);
    --stack-ring: var(--bg-elev);
    transition:
      background-color 170ms cubic-bezier(0.33, 1, 0.68, 1),
      color 170ms cubic-bezier(0.33, 1, 0.68, 1);
  }

  .ctx-section {
    --ctx-gutter: 20px;
    --ctx-gutter-gap: 7px;
    --ctx-row-pad-x: 6px;
    --ctx-row-pad-y: 5px;
    --ctx-line-h: 18px;
  }

  .ctx-row.has-stack { padding-bottom: calc(var(--ctx-row-pad-y) + 3px); }
  .ctx-row:hover { background: var(--control-hover); color: var(--ink-strong); --stack-ring: var(--control-hover); }
  .ctx-row.active {
    background: var(--control-active);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--ink);
    --stack-ring: var(--control-active);
  }
  .ctx-row.active .ctx-name { font-weight: 500; color: var(--ink-strong); }
  .ctx-row:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

  .ctx-icon {
    display: grid;
    place-items: center;
    width: var(--ctx-gutter);
    height: var(--ctx-line-h);
    color: var(--ink-faint);
    opacity: 0.7;
    transition: opacity 170ms ease, color 170ms ease;
  }
  .ctx-icon :global(svg) { width: 14px; height: 14px; }
  .ctx-row:hover .ctx-icon,
  .ctx-row.active .ctx-icon { opacity: 1; color: var(--ink-mute); }

  .ctx-name {
    min-width: 0;
    color: var(--ink);
    font-size: 12px;
    font-weight: 450;
    line-height: var(--ctx-line-h);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ctx-stack {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    justify-self: start;
    max-width: 100%;
    height: var(--ctx-line-h);
    --stack-dwell: 500ms;
  }

  .ctx-stack-item {
    display: grid;
    place-items: center;
    border-radius: 5px;
    box-shadow: 0 0 0 1.5px var(--stack-ring);
    background: var(--stack-ring);
  }
  .ctx-stack-item + .ctx-stack-item { margin-left: -4px; }

  .ctx-stack-item.is-extra {
    max-width: 0;
    opacity: 0;
    margin-left: 0 !important;
    overflow: hidden;
    transition:
      max-width 220ms cubic-bezier(0.33, 1, 0.68, 1),
      opacity 150ms ease,
      margin-left 220ms cubic-bezier(0.33, 1, 0.68, 1);
  }
  .ctx-stack:hover .ctx-stack-item.is-extra {
    max-width: 18px;
    opacity: 1;
    margin-left: -4px !important;
    transition-delay: calc(var(--stack-dwell) + var(--reveal-delay, 0ms));
  }

  .ctx-stack-more {
    margin-left: 5px;
    font-family: var(--sans);
    font-size: 8px;
    font-variant-numeric: tabular-nums;
    color: var(--ink-faint);
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    transition: opacity 150ms ease, max-width 200ms ease, margin-left 200ms ease;
  }
  .ctx-stack:hover .ctx-stack-more {
    opacity: 0;
    max-width: 0;
    margin-left: 0;
    transition-delay: var(--stack-dwell);
  }
  .ctx-stack-more-rest {
    opacity: 0;
    max-width: 0;
    margin-left: 0;
  }
  .ctx-stack:hover .ctx-stack-more-rest {
    opacity: 0.7;
    max-width: 30px;
    margin-left: 5px;
    transition-delay: calc(var(--stack-dwell) + 140ms);
  }

  .ctx-tail {
    position: absolute;
    top: var(--ctx-row-pad-y);
    right: var(--ctx-row-pad-x);
    height: var(--ctx-line-h);
    display: flex;
    align-items: center;
    gap: 3px;
    pointer-events: none;
  }

  .ctx-pin {
    display: grid;
    place-items: center;
    width: 10px;
    height: 10px;
    color: color-mix(in srgb, var(--ink-faint) 70%, transparent);
    transition: opacity 150ms ease;
  }

  .ctx-age {
    width: 23px;
    text-align: right;
    font-family: var(--sans);
    font-size: 9.5px;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--ink-faint) 70%, transparent);
    letter-spacing: 0.02em;
    transition: opacity 150ms ease;
  }

  .ctx-kebab {
    position: absolute;
    right: -3px;
    top: 50%;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-mute);
    cursor: pointer;
    pointer-events: auto;
    opacity: 0;
    z-index: 1;
    transform: translateY(-50%) scale(0.82);
    transition: opacity 150ms ease, transform 170ms cubic-bezier(0.33, 1, 0.68, 1), background-color 140ms ease, color 140ms ease;
  }
  .ctx-kebab:hover { background: var(--control-hover); color: var(--ink-strong); }
  .ctx-kebab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    opacity: 1;
    pointer-events: auto;
    transform: translateY(-50%) scale(1);
  }

  .ctx-row-wrap:hover .ctx-age,
  .ctx-row-wrap:focus-within .ctx-age,
  .ctx-row-wrap.menu-open .ctx-age,
  .ctx-row-wrap:hover .ctx-pin,
  .ctx-row-wrap:focus-within .ctx-pin,
  .ctx-row-wrap.menu-open .ctx-pin { opacity: 0; }
  .ctx-row-wrap:hover .ctx-kebab,
  .ctx-row-wrap:focus-within .ctx-kebab,
  .ctx-row-wrap.menu-open .ctx-kebab {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(-50%) scale(1);
  }

  @media (prefers-reduced-motion: reduce) {
    .ctx-row,
    .ctx-kebab,
    .ctx-age,
    .ctx-pin,
    .ctx-icon,
    .ctx-stack-item.is-extra,
    .ctx-stack-more { transition-duration: 1ms; transition-delay: 0ms; }
    .ctx-stack { --stack-dwell: 0ms; }
  }

  .ctx-menu {
    position: fixed;
    right: auto;
    z-index: 70;
    min-width: 132px;
    max-width: 200px;
  }
  .ctx-menu-delete { transition: color .15s ease, background-color .15s ease; }
  .ctx-menu-delete:hover { color: var(--danger); }
  .ctx-menu-delete.is-armed { color: var(--danger); font-weight: 500; background: var(--danger-bg); }

  .dl-panel {
    margin: 0 8px 6px;
    padding: 10px 11px;
    max-height: min(30vh, 240px);
    overflow-y: auto;
    border-radius: 9px;
    background: var(--control-active);
    border: 1px solid var(--line-soft);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .dl-item-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .dl-item-name {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--ink-soft);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dl-cancel {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-mute);
    cursor: pointer;
    transition: background 140ms ease, color 140ms ease;
  }

  .dl-cancel:hover {
    background: var(--danger-bg, color-mix(in srgb, var(--danger) 12%, transparent));
    color: var(--danger);
  }

  .dl-cancel:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .dl-done {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 11.5px;
    color: var(--ink-soft);
  }

  .dl-dot {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }

  .dl-done-name {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sidebar-foot {
    padding: 6px 8px 8px;
    border-top: 1px solid color-mix(in srgb, var(--line-soft) 55%, transparent);
    margin: 0 8px;
  }

  .local-bar {
    margin: 4px 8px 10px;
    padding: 9px 10px;
    border-radius: 8px;
    background: var(--control-active);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .local-bar-row {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 11px;
    color: var(--ink-soft);
  }

  .local-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--arm-700);
    flex-shrink: 0;
    display: block;
  }

  .meta-wrapper {
    margin-left: auto;
    display: flex;
    align-items: center;
  }

  .meta {
    display: inline-flex;
    align-items: center;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--ink-mute);
    font-variant-numeric: tabular-nums;
  }

  .digit-slot {
    position: relative;
    display: inline-block;
    overflow: hidden;
    width: 1ch;
    height: 14px;
  }

  .digit-char {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    text-align: center;
    line-height: 14px;
  }

  .meta-unit {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--ink-mute);
  }

  .local-meter-thin {
    height: 2px;
    background: var(--amber-200);
    border-radius: 999px;
    overflow: hidden;
  }

  .local-meter-thin span {
    display: block;
    height: 100%;
    border-radius: 999px;
    transition: background 0.4s ease;
  }

  @media (max-width: 720px) {
    .sidebar { width: 58px; }
    .sidebar :global(.brand) { padding-inline: 0; justify-content: center; }
    .sidebar :global(.brand-name) { display: none; }
    .nav-section { padding-inline: 7px; }
    .sidebar:not(.rail-settings) .nav-section { padding-top: 18px; }
    .settings-search,
    .settings-search-results { display: none; }
    .nav-item,
    .settings-nav-item,
    .settings-back { justify-content: center; gap: 0; padding-inline: 0; }
    .nav-item > span,
    .settings-nav-item > span,
    .settings-back > span,
    .settings-section-label { display: none; }
    .sidebar-foot { margin-inline: 7px; padding-inline: 0; }
    .local-bar { margin-inline: 7px; padding-inline: 0; align-items: center; }
    .local-bar-row { justify-content: center; }
    .local-bar-row > span:not(.local-dot),
    .meta-wrapper,
    .local-meter-thin { display: none; }
    .dl-panel { margin-inline: 7px; padding-inline: 6px; }
    .dl-item-name,
    .dl-done-name { display: none; }
    .ctx-section { padding-inline: 7px; }
    .ctx-head { justify-content: center; padding-inline: 0; }
    .ctx-head-label,
    .ctx-head-rule { display: none; }
    .ctx-subhead { justify-content: center; padding-inline: 0; }
    .ctx-subhead-label { display: none; }
    .ctx-row {
      grid-template-columns: 1fr;
      column-gap: 0;
      justify-items: center;
      padding: 6px 0;
    }
    .ctx-name,
    .ctx-stack,
    .ctx-tail { display: none; }
  }
</style>
