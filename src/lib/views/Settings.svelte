<script lang="ts">
  import { appStore } from '../stores';
  import { onDestroy, onMount } from 'svelte';
  import { listen } from '../tauri';
  import { fade } from 'svelte/transition';
  import { MOTION_MS, MOTION_PX, SETTINGS_SECTION_ORDER, directionFromOrder, modalBackdrop, motionMs, motionPx, pageSwap, reducedMotionEnabled } from '../motion';
  import { isSettingsSectionId, visibleSettingsSections } from '../settingsSections';
  import { icons } from '../icons';
  import { scrollEdges, type ScrollEdgeCallback } from '../scrollFade';
  import { clearSettingsSearchNavigation, requestSettingsSearchNavigation, searchSettings, settingsSearchNavigation, type SettingsSearchEntry } from '../settingsSearch.svelte';

  import DeferredView from '../components/DeferredView.svelte';
  import { lazyComponent } from '../lazyComponent.svelte';
  import AboutSection from '../components/settings/AboutSection.svelte';
  import { isAndroid, isMac } from '../platform';

  const sections = {
    general: lazyComponent(() => import('../components/settings/GeneralSection.svelte')),
    apps: lazyComponent(() => import('../components/settings/AppMappingsSection.svelte')),
    keys: lazyComponent(() => import('../components/settings/ApiKeysSection.svelte')),
    models: lazyComponent(() => import('../components/settings/ModelsSection.svelte')),
    privacy: lazyComponent(() => import('../components/settings/PrivacySection.svelte')),
    integrations: lazyComponent(() => import('../components/settings/IntegrationsSection.svelte')),
    sync: lazyComponent(() => import('../components/settings/SyncSection.svelte')),
    advanced: lazyComponent(() => import('../components/settings/AudioSection.svelte')),
    permissions: lazyComponent(() => import('../components/settings/PermissionsSection.svelte')),
    developer: lazyComponent(() => import('../components/settings/DeveloperSection.svelte')),
    subapps: lazyComponent(() => import('../components/settings/SubAppsSection.svelte')),
  };

  let settingsPageEl = $state<HTMLDivElement | null>(null);
  let settingsPanelEl = $state<HTMLDivElement | null>(null);
  let previousFocusEl: HTMLElement | null = null;
  let searchHighlightTimer: ReturnType<typeof setTimeout> | null = null;

  const section = $derived(appStore.settingsSection);
  const sectionReady = $derived(section === 'about' || !!sections[section].component);
  const animDir = $derived(appStore.settingsAnimDir);
  const appVersion = $derived(appStore.appVersion);

  // The Android list, section labels, and search share one filtered grouping.
  // Keep the flat projection derived from those groups so visibility rules run once.
  // Android shows the section list as its own screen on phones and beside the
  // section on wide windows (both are laid out in CSS from the width class).
  const mobileGroups = $derived(
    visibleSettingsSections({
      isMac,
      devMode: appStore.devModeEnabled,
      legacyMode: appStore.legacyFeaturesEnabled,
      syncEnabled: appStore.syncEnabled,
    })
  );
  const mobileSections = $derived(mobileGroups.flatMap((group) => group.items));
  const activeSectionLabel = $derived(
    mobileSections.find((entry) => entry.id === section)?.label ?? 'Settings'
  );

  let mobileQuery = $state('');
  const mobileResults = $derived(
    searchSettings(mobileQuery, mobileSections.map((entry) => entry.id), 12)
  );

  function openSearchResult(entry: SettingsSearchEntry) {
    mobileQuery = '';
    openSection(entry.section);
    requestSettingsSearchNavigation(entry);
  }

  function openSection(next: (typeof mobileSections)[number]['id']) {
    appStore.settingsMobileList = false;
    selectSection(next);
  }

  function selectSection(next: (typeof mobileSections)[number]['id']) {
    if (next === appStore.settingsSection) return;
    appStore.settingsAnimDir = directionFromOrder(
      appStore.settingsSection,
      next,
      SETTINGS_SECTION_ORDER
    );
    appStore.settingsSection = next;
  }

  /*
   * The settings page enters and leaves on the vertical axis while the sidebar
   * rail moves horizontally, so the two halves of the transition stay legible
   * as separate motions. Mirrors --content-swap-y / --content-swap-ms on
   * .content in App.svelte, which animates the outgoing page — keep in sync.
   */
  const SETTINGS_SWAP_PX = 26;
  const SETTINGS_SWAP_MS = 320;

  onMount(() => {
    const unlistenPromise = listen<string>('open-flow:open-settings-section', (event) => {
      const target = event.payload;
      const hasTarget = !!target && isSettingsSectionId(target);
      const nextSection = hasTarget ? target : 'general';
      if (nextSection !== appStore.settingsSection) {
        appStore.settingsAnimDir = directionFromOrder(
          appStore.settingsSection,
          nextSection,
          SETTINGS_SECTION_ORDER
        );
      }
      appStore.settingsSection = nextSection;
      if (isAndroid) appStore.settingsMobileList = !hasTarget;
      appStore.settingsOpen = true;
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  });

  function close() { appStore.settingsOpen = false; }

  // Moving between the phone's list and a section moves focus with it, so a
  // keyboard or TalkBack user lands on the screen that is now showing.
  let lastMobileList = appStore.settingsMobileList;
  $effect(() => {
    const list = appStore.settingsMobileList;
    if (list === lastMobileList) return;
    lastMobileList = list;
    if (!isAndroid || !appStore.mobileCompact || !appStore.settingsOpen) return;
    requestAnimationFrame(() => {
      if (list) settingsPageEl?.focus({ preventScroll: true });
      else settingsPageEl?.querySelector<HTMLElement>('.m-bar-back')?.focus({ preventScroll: true });
    });
  });

  $effect(() => {
    // An unqualified re-entry should start from the section list.
    if (!appStore.settingsOpen && isAndroid) appStore.settingsMobileList = true;
  });

  // A settings search hit or deep link names a section: show it, not the list.
  $effect(() => {
    if (settingsSearchNavigation.request) appStore.settingsMobileList = false;
  });

  // Soft fades at the top and bottom of the scroll area, shown only when there
  // is actually more content in that direction — so a scrolled-to-top page keeps
  // its heading crisp and the fade never obscures anything at rest.
  let fadeTop = $state(false);
  let fadeBottom = $state(false);
  const setFades: ScrollEdgeCallback = (top, bottom, node) => {
    const apply = () => {
      if (node !== settingsPanelEl) return;
      fadeTop = top;
      fadeBottom = bottom;
    };
    // Svelte actions run before bind:this is assigned on initial mount. Defer
    // only that first callback so the active panel can claim the state, while
    // callbacks from an outgoing keyed panel remain ignored.
    if (node !== settingsPanelEl) queueMicrotask(apply);
    else apply();
  };

  $effect(() => {
    if (!appStore.devModeEnabled && appStore.settingsSection === 'developer') {
      appStore.settingsSection = 'about';
    }
  });

  $effect(() => {
    if (!appStore.legacyFeaturesEnabled && appStore.settingsSection === 'apps') {
      appStore.settingsSection = 'general';
    }
  });

  $effect(() => {
    if (!appStore.syncEnabled && appStore.settingsSection === 'sync') {
      appStore.settingsSection = 'general';
    }
  });

  $effect(() => {
    if (typeof document === 'undefined') return;

    if (!appStore.settingsOpen) {
      const target = previousFocusEl;
      previousFocusEl = null;
      if (target?.isConnected) {
        requestAnimationFrame(() => target.focus());
      }
      return;
    }

    if (!previousFocusEl && document.activeElement instanceof HTMLElement) {
      previousFocusEl = document.activeElement;
    }

    let cancelled = false;
    let attempts = 0;
    const focusSettingsPage = () => {
      if (cancelled || !appStore.settingsOpen) return;
      const active = document.activeElement;
      const dialog = document.querySelector('[role="dialog"]');
      if (active instanceof HTMLElement && dialog?.contains(active)) {
        // A dialog from the outgoing page may still be running its close
        // transition. Let it finish if it still owns focus before moving focus
        // into Settings.
        if (attempts++ < 60) requestAnimationFrame(focusSettingsPage);
        return;
      }
      // Don't steal focus if the user already moved inside the shell while it
      // was opening (keyboard flows race the entrance transition).
      if (active instanceof HTMLElement && settingsPageEl?.contains(active)) return;
      // On Android the first control is the search field; focusing it opens the
      // keyboard the moment Settings appears.
      (isAndroid ? settingsPageEl : (firstFocusableInShell() ?? settingsPageEl))?.focus();
    };
    requestAnimationFrame(focusSettingsPage);
    return () => { cancelled = true; };
  });

  // Announce the section that just loaded: move focus to the panel heading
  // (falling back to the first focusable control) so keyboard and screen-reader
  // users land on the content they switched to instead of staying on the rail
  // button while the panel swaps underneath them. Settings is a page, so Tab
  // still reaches the rail — this is context, not a trap.
  $effect(() => {
    const currentSection = section;
    if (!appStore.settingsOpen || !settingsPanelEl || !currentSection || !sectionReady) return;
    // The section is parked behind the list on a phone: do not pull focus into it.
    if (appStore.mobileCompact && appStore.settingsMobileList) return;
    const panel = settingsPanelEl;
    requestAnimationFrame(() => {
      if (!panel.isConnected) return;
      if (document.querySelector('[role="dialog"]')) return;
      // Never fight the user: if focus is already inside the panel (keyboard
      // flow racing the section swap), leave it alone.
      const active = document.activeElement;
      if (active instanceof HTMLElement && panel.contains(active)) return;
      const heading = panel.querySelector<HTMLElement>('.settings-h, .settings-subhead, h2');
      const target = heading ?? panel.querySelector<HTMLElement>([
        'button:not([disabled])',
        'input:not([disabled])',
        'select:not([disabled])',
        'textarea:not([disabled])',
        'a[href]',
        '[tabindex]:not([tabindex="-1"])',
      ].join(','));
      if (heading && !heading.hasAttribute('tabindex')) heading.setAttribute('tabindex', '-1');
      if (target instanceof HTMLElement && panel.contains(target)) {
        target.focus({ preventScroll: true });
      }
    });
  });

  $effect(() => {
    const request = settingsSearchNavigation.request;
    const currentSection = section;
    const panel = settingsPanelEl;
    if (!request || !appStore.settingsOpen || request.section !== currentSection || !panel || !sectionReady) return;

    let cancelled = false;
    let attempts = 0;

    const revealTarget = () => {
      if (cancelled || settingsSearchNavigation.request?.nonce !== request.nonce) return;
      if (!panel.isConnected || settingsPanelEl !== panel) {
        if (attempts++ < 40) requestAnimationFrame(revealTarget);
        return;
      }

      const exactTarget = panel.querySelector<HTMLElement>(`[data-setting-target="${request.target}"]`);
      if (!exactTarget && attempts++ < 40) {
        requestAnimationFrame(revealTarget);
        return;
      }
      const fallbackTarget = request.fallbackTarget
        ? panel.querySelector<HTMLElement>(`[data-setting-target="${request.fallbackTarget}"]`)
        : null;
      const target = exactTarget ?? fallbackTarget ?? panel.querySelector<HTMLElement>('.settings-h');
      if (!target) {
        clearSettingsSearchNavigation(request.nonce);
        return;
      }

      panel.querySelector<HTMLElement>('.settings-search-hit')?.classList.remove('settings-search-hit');
      target.classList.add('settings-search-hit');
      if (!target.matches('button, input, select, textarea, a[href], [tabindex]')) {
        target.setAttribute('tabindex', '-1');
      }
      target.scrollIntoView({
        block: 'center',
        behavior: reducedMotionEnabled() ? 'auto' : 'smooth',
      });
      target.focus({ preventScroll: true });

      if (searchHighlightTimer) clearTimeout(searchHighlightTimer);
      searchHighlightTimer = setTimeout(() => {
        target.classList.remove('settings-search-hit');
        searchHighlightTimer = null;
      }, 1600);
      clearSettingsSearchNavigation(request.nonce);
    };

    requestAnimationFrame(revealTarget);
    return () => {
      cancelled = true;
    };
  });

  onDestroy(() => {
    if (searchHighlightTimer) clearTimeout(searchHighlightTimer);
  });

  /**
   * Settings is a page, not a dialog, so there is no focus trap — Tab is free to
   * reach the rail in the sidebar. Focus still moves into the shell on open so
   * keyboard users land on the content they just asked for.
   */
  function firstFocusableInShell(): HTMLElement | null {
    if (!settingsPageEl) return null;
    const selector = [
      'button:not([disabled])',
      'input:not([disabled])',
      'select:not([disabled])',
      'textarea:not([disabled])',
      'a[href]',
      '[tabindex]:not([tabindex="-1"])',
    ].join(',');

    return Array.from(settingsPageEl.querySelectorAll<HTMLElement>(selector))
      .find((el) => !el.hasAttribute('inert') && el.offsetParent !== null) ?? null;
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (appStore.settingsOpen && e.key === 'Escape') {
      // An inner layer already consumed the key (dropdown, disclosure, tile,
      // hotkey capture). Svelte-delegated element handlers flush state before
      // this window listener runs, so the DOM guards below can no longer see
      // the layer that handled it — defaultPrevented is the reliable signal.
      if (e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, [contenteditable="true"]')) {
        return;
      }
      // A dropdown menu or an open dialog owns Escape while it is up: closing
      // Settings underneath the confirm dialogs (beta updates, delete history,
      // cleanup off) would skip a layer and drop the user out of Settings
      // entirely instead of just dismissing the dialog.
      if (typeof document !== 'undefined' && document.querySelector('[aria-expanded="true"], [role="dialog"]')) {
        return;
      }
      close();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if appStore.settingsOpen}
  <div class="settings-overlay-wrap">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <!-- Fades on roughly the same clock as .content's exit, so the outgoing
         page's rise stays visible through the wash instead of being snapped away. -->
    <div class="settings-overlay" aria-hidden="true" onclick={close} in:modalBackdrop={{ duration: motionMs(MOTION_MS.panel) }} out:modalBackdrop={{ duration: motionMs(MOTION_MS.base) }}></div>
    <div
      bind:this={settingsPageEl}
      class="settings-page"
      role="region"
      aria-label="Settings"
      tabindex="-1"
      data-mobile-view={appStore.settingsMobileList ? 'list' : 'detail'}
      in:pageSwap={{ axis: 'y', distance: motionPx(SETTINGS_SWAP_PX), duration: motionMs(SETTINGS_SWAP_MS) }}
      out:pageSwap={{ axis: 'y', distance: motionPx(SETTINGS_SWAP_PX), duration: motionMs(SETTINGS_SWAP_MS) }}
    >
      {#if appStore.setupComplete === false}
        <div class="setup-return-bar">
          <button type="button" class="setup-return" onclick={close}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"/></svg>
            Back to setup
          </button>
        </div>
      {/if}
      <!-- The section rail lives in Sidebar.svelte, which morphs into it.
           Normal app use closes Settings from the sidebar. Setup has a return
           control above because the app body is inert while onboarding runs. -->
      {#if isAndroid}
        <nav class="m-list" aria-label="Settings sections">
          <h1 class="m-list-title">Settings</h1>
          <label class="m-search">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
            <input type="search" bind:value={mobileQuery} placeholder="Search settings" aria-label="Search settings" autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="search" />
          </label>
          {#if mobileQuery.trim()}
            <div class="m-list-group" role="group" aria-label="Search results">
              {#each mobileResults as entry (entry.id)}
                <button type="button" class="m-list-row m-result" onclick={() => openSearchResult(entry)}>
                  <span class="m-list-label">
                    {entry.label}
                    <span class="m-result-desc">{entry.description}</span>
                  </span>
                  <svg class="m-list-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>
                </button>
              {:else}
                <p class="m-empty">No settings match “{mobileQuery.trim()}”.</p>
              {/each}
            </div>
          {:else}
          {#each mobileGroups as group (group.group)}
            <div class="m-list-group" role="group" aria-label={group.group}>
              {#each group.items as entry (entry.id)}
                <button
                  type="button"
                  class="m-list-row"
                  class:active={section === entry.id}
                  aria-current={section === entry.id ? 'page' : undefined}
                  onclick={() => openSection(entry.id)}
                >
                  <span class="m-list-icon" aria-hidden="true">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{@html icons[entry.icon]}</svg>
                  </span>
                  <span class="m-list-label">{entry.label}</span>
                  <svg class="m-list-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>
                </button>
              {/each}
            </div>
          {/each}
          {/if}
        </nav>
        <div class="m-bar">
          <button type="button" class="m-bar-back" aria-label="Back to settings" onclick={() => { appStore.settingsMobileList = true; }}>
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 6l-6 6 6 6"/></svg>
          </button>
          <h1 class="m-bar-title">{activeSectionLabel}</h1>
        </div>
      {/if}
      <div class="settings-tabs" role="tablist" aria-label="Settings sections">
        {#each mobileSections as entry (entry.id)}
          <button
            class="settings-tab"
            class:active={section === entry.id}
            role="tab"
            aria-selected={section === entry.id}
            onclick={() => selectSection(entry.id)}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={section === entry.id ? '2.2' : '1.6'} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons[entry.icon]}</svg>
            {entry.label}
          </button>
        {/each}
      </div>

      <!-- Parked off-screen (not display:none, so controls can still measure) while
           the phone shows the section list; inert keeps keyboard focus and screen
           readers off it. -->
      <div class="settings-body" inert={appStore.mobileCompact && appStore.settingsMobileList}>
        <div class="fade-edge fade-edge-top" class:visible={fadeTop} aria-hidden="true"></div>
        <div class="fade-edge fade-edge-bottom" class:visible={fadeBottom} aria-hidden="true"></div>
        {#key section}
          <div
            bind:this={settingsPanelEl}
            class="panel scroll-styled"
            use:scrollEdges={setFades}
            in:pageSwap={{ axis: 'y', distance: animDir * motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel) }}
            out:pageSwap={{ axis: 'y', distance: -animDir * motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.base) }}
          >
            <div class="panel-inner">
              {#if section === 'about'}
                <AboutSection {appVersion} />
              {:else if (section !== 'permissions' || isMac || isAndroid) && (section !== 'developer' || appStore.devModeEnabled)}
                <DeferredView view={sections[section]} />
              {/if}
            </div>
          </div>
        {/key}
      </div>

      <!--
        The page's sign-off, on About only. The bar keeps its height on every
        section so moving on and off About doesn't resize the panel underneath.
      -->
      <div class="settings-footbar">
        {#if section === 'about'}
          <div class="settings-foot" transition:fade={{ duration: motionMs(MOTION_MS.base) }}>
            Verenu v{appVersion} · MIT
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .settings-overlay-wrap {
    position: absolute;
    inset: 0;
    z-index: 60;
  }

  /*
   * Full-bleed opaque wash rather than a dim scrim: settings reads as the page
   * clearing, not as a card floating over it. It still fades (the smoke tests
   * probe it mid-opacity on open and close) and it stays exposed in the
   * --app-gutter strip that .body leaves on the left and bottom, which is what
   * keeps click-outside-to-dismiss working now that the shell fills the window.
   */
  .settings-overlay {
    position: absolute;
    inset: 0;
    background: var(--paper);
    border: 0;
    padding: 0;
  }

  .setup-return-bar {
    flex: 0 0 auto;
    padding: 8px var(--page-pad-x) 0;
  }

  .setup-return {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 30px;
    padding: 4px 7px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--ink-mute);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .setup-return:hover { background: var(--control-hover); color: var(--ink-strong); }
  .setup-return:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }

  /*
   * Geometry deliberately mirrors .content in App.svelte, and there is no card
   * treatment — no elevated background, border or radius. Settings is a page in
   * the same content region as Home/Dictionary/Style, not a panel floating on
   * top of one, so it should be indistinguishable from them as a surface.
   */
  .settings-page {
    position: absolute;
    top: var(--safe-top);
    bottom: max(var(--app-gutter), var(--mobile-nav-h));
    left: calc(var(--sidebar-w) + var(--app-gutter));
    right: 0;
    z-index: 1;
    background: transparent;
    border: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* Settings is rooted directly in .app, unlike the regular views inside
     .body. Reserve the custom Windows caption row so its shared page padding
     starts at the same visual height as Home, Insights, and the libraries. */

  /*
   * Reserves the same gutter .panel does via `scrollbar-gutter: stable`, so the
   * centred footer lines up with the centred content column instead of sitting
   * half a scrollbar to its right.
   */
  .settings-footbar {
    flex-shrink: 0;
    padding: 0 calc(var(--page-pad-x) + var(--scrollbar-w)) 16px var(--page-pad-x);
  }

  /* Centred across the settings page, not aligned to the content column's left
     edge — as a page sign-off it should read as centred, and column-aligned it
     just looked adrift somewhere right of middle. */
  .settings-foot {
    text-align: center;
    font-family: var(--sans);
    font-size: 11px;
    letter-spacing: 0.01em;
    color: var(--ink-faint);
  }

  /*
   * Phone-only section switcher. Hidden wherever the sidebar rail is doing this
   * job; :global on the ancestor because the attribute lives on .app.
   */
  .settings-tabs { display: none; }

    /* Android navigates sections with the list in mobile.css instead. */

  /* Matches the underline tabs the Contexts page already uses — same measure,
     same active rule, no extra chrome. */
  .settings-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
    position: relative;
    min-height: 40px;
    padding: 0 0 9px;
    border: 0;
    background: transparent;
    color: var(--ink-mute);
    font-family: var(--sans);
    font-size: 13px;
    white-space: nowrap;
  }

  .settings-tab.active { color: var(--ink); font-weight: 500; }

  .settings-tab.active::after {
    content: '';
    position: absolute;
    bottom: -1px;
    left: 0;
    right: 0;
    height: 1px;
    background: var(--ink);
  }

  /* Panel area */
  .settings-body {
    flex: 1;
    position: relative;
    overflow: hidden;
    display: grid;
  }

  /*
   * Soft top/bottom scroll fades. Overlays (not a mask on the scroller) so the
   * scrollbar and the per-row entrance animations are untouched. They fade from
   * the page background to transparent and only appear when there's more to
   * scroll in that direction. Right edge stops short of the scrollbar gutter.
   */
  .fade-edge {
    position: absolute;
    left: 0;
    right: var(--scrollbar-w, 0);
    height: 30px;
    pointer-events: none;
    z-index: 3;
    opacity: 0;
    transition: opacity 180ms ease;
  }

  .fade-edge.visible { opacity: 1; }

  .fade-edge-top {
    top: 0;
    background: linear-gradient(to bottom, var(--paper), transparent);
  }

  .fade-edge-bottom {
    bottom: 0;
    background: linear-gradient(to top, var(--paper), transparent);
  }

  @media (prefers-reduced-motion: reduce) {
    .fade-edge { transition: none; }
  }

  .panel {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    padding: var(--page-pad-y) var(--page-pad-x) 36px;
    overflow-y: auto;
    scrollbar-gutter: stable;
  }

  /*
   * .setting-row is a space-between flex row with no intrinsic max width, so
   * without a measure every label ends up marooned from its control once the
   * shell fills the window. container-type lets the sections swap their old
   * viewport media queries for container queries against this column.
   */
  /* Android uses the whole content column: the 680px measure left the right
     half of an unfolded screen empty and pushed the controls off the edge. */
  :global(.app[data-android='true']) .panel-inner { width: 100%; }

  .panel-inner {
    width: min(100%, 680px);
    margin-inline: auto;
    container-type: inline-size;
    container-name: settings-panel;
  }

  /*
   * Per-row entrance. .panel remounts on every section change ({#key section}),
   * so a plain CSS animation on the children runs exactly once per swap — no
   * per-section markup changes needed. The panel's own pageSwap distance is
   * deliberately small (MOTION_PX.nudge) so the two layers don't fight.
   *
   * fill-mode is `backwards`, never `forwards`/`both`: an animated transform or
   * opacity makes each row its own stacking context, which would trap the
   * absolutely-positioned dropdown menus (language, mic, models) inside their
   * row so the following row paints over them. `backwards` covers the delay
   * window and then releases the row back to its natural styles.
   */
  /* Settings should settle as one page. Staggering every row makes a routine
     preferences screen feel like a presentation. */

  /* Shared styles for all section components — scoped to .settings-body */
  /* Matches .page-h on the app views so a section heading reads at the same
     level as "Home" or "Style" rather than as a panel title. */
  .settings-body :global(.settings-h) {
    font-family: var(--sans);
    font-size: 23px;
    font-weight: 600;
    margin: 0 0 var(--settings-h-mb, 20px);
    letter-spacing: -0.025em;
    line-height: 1.1;
    color: var(--ink);
  }

  .settings-body :global(.settings-subhead) {
    font-family: var(--sans);
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-mute);
    letter-spacing: 0;
    margin: 28px 0 6px;
  }

  .settings-body :global(.settings-subhead.first) { margin-top: 4px; }

  .settings-body :global([data-setting-target]) {
    position: relative;
    scroll-margin-block: 72px;
  }

  .settings-body :global(.settings-search-hit) {
    outline: none;
  }

  .settings-body :global(.settings-search-hit::before) {
    content: '';
    position: absolute;
    top: 50%;
    left: -12px;
    width: 3px;
    height: min(28px, calc(100% - 12px));
    border-radius: 999px;
    background: var(--accent);
    pointer-events: none;
    animation: settings-search-locator 1600ms var(--ui-ease-out);
  }

  @keyframes settings-search-locator {
    0% {
      opacity: 0;
      transform: translate3d(-4px, -50%, 0) scaleY(0.45);
    }
    16%, 55% {
      opacity: 0.9;
      transform: translate3d(0, -50%, 0) scaleY(1);
    }
    100% {
      opacity: 0;
      transform: translate3d(0, -50%, 0) scaleY(0.72);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .settings-body :global(.settings-search-hit::before) {
      animation: none;
      opacity: 0.9;
      transform: translate3d(0, -50%, 0);
    }
  }

  .settings-body :global(.setting-row) {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 24px;
    padding: 14px 0;
    border-top: 1px solid var(--line);
  }

  .settings-body :global(.setting-row:last-of-type) { border-bottom: 1px solid var(--line); }

  /*
   * Phone rows stack. Side-by-side label/control only works while the label has
   * room to breathe; under ~520px the description squeezes into a 3-4 line
   * ribbon beside a lonely control. Stacking gives the text the full measure and
   * puts the control on its own line at a comfortable thumb size.
   */
  @container settings-panel (max-width: 440px) {
    .settings-body :global(.setting-row) {
      grid-template-columns: minmax(0, 1fr);
      align-items: stretch;
      gap: 10px;
      padding: 16px 0;
    }

    .settings-body :global(.setting-row) > :global(*:last-child) {
      justify-self: start;
      margin-left: 0;
    }

    /* A switch is a one-line control: keep it beside its label, on the right,
       instead of dropping it under the description like a wide control. */
    .settings-body :global(.setting-row:has(> .toggle:last-child)) {
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: center;
      gap: 16px;
      padding: 14px 0;
    }

    .settings-body :global(.setting-row:has(> .toggle:last-child)) > :global(.toggle) {
      justify-self: end;
    }

    .settings-body :global(.desc) { max-width: none; }

    .settings-body :global(.notification-test-controls),
    .settings-body :global(.simulation-actions),
    .settings-body :global(.actions) {
      justify-content: flex-start;
    }
  }

  /* Keep menus inside their settings panel on narrow windows. Anchoring a
     wider menu to the trigger's right edge can make it overlap the sidebar. */
  @container settings-panel (max-width: 440px) {
    .settings-body :global(.ui-dropdown-menu),
    .settings-body :global(.mic-menu) {
      left: 0;
      right: auto;
      max-width: 100cqw;
    }
  }

  .settings-body :global(.label) { font-size: 13px; font-weight: 500; color: var(--ink-strong); }
  .settings-body :global(.desc)  { font-size: 12px; color: var(--ink-mute); margin-top: 4px; max-width: 56ch; line-height: 1.45; }

  .settings-body :global(.panel-note) {
    font-size: 12px;
    color: var(--ink-mute);
    margin: 0 0 16px;
    line-height: 1.5;
  }

  .settings-body :global(.btn-ghost) {
    background: transparent;
    border: 1px solid var(--line-strong);
    border-radius: 6px;
    padding: 5px 12px;
    font-size: 12px;
    color: var(--ink-strong);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }

  .settings-body :global(.btn-ghost:hover) { background: var(--control-hover); }
  .settings-body :global(.btn-ghost:disabled) { opacity: var(--ui-disabled-opacity); cursor: default; }

  .settings-body :global(.badge) {
    background: transparent;
    border: 1px solid var(--line-strong);
    border-radius: 6px;
    padding: 5px 12px;
    font-size: 12px;
    color: var(--ink-mute);
    font-weight: 500;
    user-select: none;
    cursor: default;
    white-space: nowrap;
  }

  .settings-body :global(.key-badge) {
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.04em;
  }
</style>
