<script lang="ts">
  import { onMount } from 'svelte';
  import { appStore } from './lib/stores';
  import { cleanupPromptEditor } from './lib/stores.svelte';
  import { isWindows } from './lib/platform';
  import Sidebar from './lib/components/layout/Sidebar.svelte';
  import Home from './lib/views/Home.svelte';
  import AgentAccessibilityDump from './lib/components/AgentAccessibilityDump.svelte';
  import Insights from './lib/views/Insights.svelte';
  import Contexts from './lib/views/Contexts.svelte';
  import Dictionary from './lib/views/Dictionary.svelte';
  import Snippets from './lib/views/Snippets.svelte';
  import Style from './lib/views/Style.svelte';
  import Settings from './lib/views/Settings.svelte';
  import CleanupPromptModal from './lib/components/settings/CleanupPromptModal.svelte';
  import SyncPairModal from './lib/components/settings/SyncPairModal.svelte';
  import { startSyncListeners, syncStore } from './lib/syncStore.svelte';
  import DictationPill from './lib/components/layout/DictationPill.svelte';
  import Setup from './lib/views/Setup.svelte';
  import { getVersion, invoke, isTauriRuntime, listen } from './lib/tauri';
  import { startAutomaticUpdateChecks } from './lib/updates';
  import { startPolling } from './lib/polling';
  import { startLocalSttListeners } from './lib/localSttStore.svelte';
  import { startLocalLlmListeners } from './lib/localLlmStore.svelte';
  import { startDownloadManagerListeners } from './lib/downloadManager.svelte';
  import { refreshTranscriptionModel } from './lib/transcriptionModelStore.svelte';
  import { startProviderStatusChecks } from './lib/serviceStatus';
  import { classifyIpcError, settingsSectionForKind, type ErrorKind } from './lib/errors';
  import { SETTINGS_SAVE_ERROR_EVENT } from './lib/settings';
  import type { SettingsSectionId } from './lib/settingsSections';
  import { scrollEdges } from './lib/scrollFade';
  import { fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { MOTION_MS, MOTION_PX, NAV_ORDER, SETTINGS_SECTION_ORDER, directionFromOrder, motionMs, motionPx, pageSwap, reducedMotionEnabled } from './lib/motion';
  import { applyAccentTheme, normalizeAccentColor } from './lib/accentTheme';
  import { isAndroid } from './lib/platform';
  import MobileNav from './lib/components/layout/MobileNav.svelte';
  import {
    snapshotForSize,
    trackViewport,
    type ViewportSnapshot,
  } from './lib/android/viewport';

  type EffectiveTheme = 'light' | 'dark';
  type NativeTitleBarMetrics = { height: number; leftInset: number; rightInset: number; scaleFactor: number };

  function applyNativeTitleBarMetrics(metrics: NativeTitleBarMetrics | null) {
    // Browser dev mode and the test harness have no Windows caption; skip
    // silently instead of logging a crash for a missing metrics payload.
    if (!metrics) return;
    const root = document.documentElement;
    root.style.setProperty('--native-titlebar-height', `${metrics.height}px`);
    root.style.setProperty('--native-caption-left-inset', `${metrics.leftInset}px`);
    root.style.setProperty('--native-caption-right-inset', `${metrics.rightInset}px`);
  }

  function systemTheme(): EffectiveTheme {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  function effectiveTheme(mode: 'system' | 'light' | 'dark'): EffectiveTheme {
    return mode === 'system' ? systemTheme() : mode;
  }

  function applyTheme() {
    const theme = effectiveTheme(appStore.appearanceMode);
    document.documentElement.dataset.theme = theme;
    if (isWindows && isTauriRuntime()) {
      invoke('set_native_titlebar_theme', { dark: theme === 'dark' }).catch((error) => {
        console.error('Failed to sync native title bar theme:', error);
      });
    }
  }

  $effect(() => {
    appStore.appearanceMode;
    if (typeof document !== 'undefined') applyTheme();
  });

  $effect(() => {
    const accentColor = appStore.accentColor;
    if (typeof document !== 'undefined') applyAccentTheme(document.documentElement, accentColor);
  });

  // Adaptive shell: live window-size class for foldables, rotation,
  // split-screen, and freeform windows. Defaults to expanded so desktop
  // first paint is unchanged; the tracker corrects it on mount.
  let viewport = $state<ViewportSnapshot>(
    snapshotForSize(1280, 800),
  );
  // Compact Android windows collapse the desktop rail to the bottom bar.
  // Desktop keeps its rail at every width.
  const compactNav = $derived(isAndroid && viewport.widthClass === 'compact');

  // Error toast
  let errorToast = $state('');
  let errorToastKind = $state<ErrorKind | null>(null);
  let toastTimer: ReturnType<typeof setTimeout>;
  let pageDir = $state<1 | -1>(1);
  let prevPage = $state<string>('home');
  let contentEl = $state<HTMLDivElement | null>(null);
  let fadeTop = $state(false);
  let fadeBottom = $state(false);

  $effect(() => {
    const next = appStore.currentPage;
    pageDir = directionFromOrder(prevPage, next, NAV_ORDER);
    prevPage = next;
  });

  $effect(() => {
    appStore.currentPage;
    requestAnimationFrame(() => {
      contentEl?.scrollTo({ top: 0, behavior: reducedMotionEnabled() ? 'auto' : 'smooth' });
    });
  });

  $effect(() => {
    if (
      !appStore.legacyFeaturesEnabled &&
      (appStore.currentPage === 'dictionary' || appStore.currentPage === 'snippets')
    ) {
      appStore.currentPage = 'contexts';
    } else if (appStore.legacyFeaturesEnabled && appStore.currentPage === 'contexts') {
      appStore.currentPage = 'home';
    }
  });

  // The wizard is the first thing in the app and unmounts when it finishes;
  // without this, focus drops to <body> and a keyboard user lands nowhere.
  // Land on the first rail entry (Home) only on a real completion transition
  // (wizard → app), so a normal app start never gets an unexpected focus ring.
  let prevSetupComplete = appStore.setupComplete;
  $effect(() => {
    const complete = appStore.setupComplete;
    if (prevSetupComplete === false && complete === true) {
      requestAnimationFrame(() => {
        document.querySelector<HTMLElement>('.sidebar .nav-item')?.focus();
      });
    }
    prevSetupComplete = complete;
  });

  let connectivityInFlight = false;
  async function pingConnectivity() {
    if (connectivityInFlight) return;
    connectivityInFlight = true;
    try {
      const online = await invoke<boolean>('check_connectivity');
      appStore.isOnline = online;
    } catch {
      appStore.isOnline = false;
    } finally {
      connectivityInFlight = false;
    }
  }

  function openNotificationDestination(destination: string) {
    if (destination === 'models') {
      appStore.settingsAnimDir = directionFromOrder(
        appStore.settingsSection,
        'models',
        SETTINGS_SECTION_ORDER,
      );
      appStore.settingsSection = 'models';
      appStore.settingsOpen = true;
      return;
    }

    appStore.settingsOpen = false;
    appStore.currentPage = 'home';
  }

  function openSettingsSection(section: SettingsSectionId) {
    appStore.settingsAnimDir = directionFromOrder(
      appStore.settingsSection,
      section,
      SETTINGS_SECTION_ORDER,
    );
    appStore.settingsSection = section;
    appStore.settingsOpen = true;
    errorToast = '';
    errorToastKind = null;
    clearTimeout(toastTimer);
  }

  function showErrorToast(raw: string) {
    const classified = classifyIpcError(raw);
    errorToast = raw ? classified.message : 'Something went wrong';
    errorToastKind = raw ? classified.kind : null;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { errorToast = ''; errorToastKind = null; }, 5000);
  }

  onMount(() => {
    let mounted = true;
    let cleanupFn: (() => void) | undefined;
    let stopNotificationClickListener: (() => void) | undefined;
    let stopConnectivityRecheckListener: (() => void) | undefined;
    let stopRecoveryWriteListener: (() => void) | undefined;
    let stopSettingsImportedListener: (() => void) | undefined;
    let stopAutomaticUpdateChecks: (() => void) | undefined;
    let stopLocalSttListeners: (() => void) | undefined;
    let stopLocalLlmListeners: (() => void) | undefined;
    let stopDownloadManagerListeners: (() => void) | undefined;
    let stopProviderStatusChecks: (() => void) | undefined;
    let stopSyncListeners: (() => void) | undefined;
    let stopTitleBarMetricsListener: (() => void) | undefined;
    const onSettingsSaveError = (event: Event) => {
      const message = (event as CustomEvent<unknown>).detail;
      showErrorToast(typeof message === 'string' ? message : 'Something went wrong');
    };
    window.addEventListener(SETTINGS_SAVE_ERROR_EVENT, onSettingsSaveError);

    if (isWindows && isTauriRuntime()) {
      invoke<NativeTitleBarMetrics>('get_native_titlebar_metrics')
        .then(applyNativeTitleBarMetrics)
        .catch((error) => console.error('Failed to read native title bar metrics:', error));
      listen<NativeTitleBarMetrics>('verenu:native-titlebar-metrics', (event) => applyNativeTitleBarMetrics(event.payload))
        .then((unlisten) => { stopTitleBarMetricsListener = unlisten; })
        .catch((error) => console.error('Failed to listen for native title bar metrics:', error));
    }

    // Re-reads the settings the app mirrors globally. Called once on mount and
    // again whenever a backup import lands, so an import can't leave these
    // stores disagreeing with what import_data actually wrote to disk.
    async function reloadGlobalSettings() {
      try {
        const [done, appearance, accentColor, forceSetupOnLaunch, cleanupEnabled, betaUpdatesEnabled, legacyFeaturesEnabled, syncEnabled, ruinAccessibility, devModeOnStartup] = await Promise.all([
          invoke<boolean | null>('get_setting', { key: 'setup_complete' }),
          invoke<'system' | 'light' | 'dark' | null>('get_setting', { key: 'appearance_mode' }),
          invoke<string | null>('get_setting', { key: 'accent_color' }),
          invoke<boolean | null>('get_setting', { key: 'force_setup_on_launch' }),
          invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'beta_updates_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'legacy_features_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'sync_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' }),
          invoke<boolean | null>('get_setting', { key: 'dev_mode_on_startup' }),
        ]);
        appStore.setupComplete = forceSetupOnLaunch ? false : done === true;
        if (appearance === 'light' || appearance === 'dark' || appearance === 'system') {
          appStore.appearanceMode = appearance;
        }
        appStore.accentColor = normalizeAccentColor(accentColor);
        appStore.cleanupEnabled = cleanupEnabled ?? true;
        appStore.betaUpdatesEnabled = betaUpdatesEnabled ?? false;
        appStore.legacyFeaturesEnabled = legacyFeaturesEnabled ?? false;
        appStore.syncEnabled = syncEnabled ?? false;
        appStore.ruinAccessibility = ruinAccessibility ?? false;
        appStore.devModeOnStartup = devModeOnStartup ?? false;
        if (appStore.ruinAccessibility || appStore.devModeOnStartup) appStore.devModeEnabled = true;
      } catch {
        appStore.setupComplete = false;
      }
    }

    (async () => {
      await reloadGlobalSettings();
      if (!mounted) return;
      if (appStore.syncEnabled) {
        try { stopSyncListeners = startSyncListeners(); }
        catch (error) { console.error('Failed to start sync listeners:', error); }
      }

      const unlisten = await listen<string>('verenu:error', (ev) => {
        showErrorToast(ev.payload ?? '');
      });
      cleanupFn = unlisten;
    })();

    listen('verenu:settings-imported', () => {
      void reloadGlobalSettings();
    })
      .then((unlisten) => {
        if (!mounted) {
          unlisten();
          return;
        }
        stopSettingsImportedListener = unlisten;
      })
      .catch((error) => { console.warn('Failed to listen for settings-imported events:', error); });

    listen<string>('verenu:notification-clicked', (event) => {
      openNotificationDestination(event.payload);
    })
      .then((unlisten) => {
        if (!mounted) {
          unlisten();
          return;
        }
        stopNotificationClickListener = unlisten;
      })
      .catch((error) => { console.warn('Failed to listen for notification clicks:', error); });

    // The backend fires this after a transcription request fails with a
    // connection error and it has actively confirmed the connection is down —
    // re-ping immediately so the persistent offline toast shows up now instead
    // of on the next 60s interval.
    listen('verenu:recheck-connectivity', () => void pingConnectivity())
      .then((unlisten) => {
        if (!mounted) {
          unlisten();
          return;
        }
        stopConnectivityRecheckListener = unlisten;
      })
      .catch((error) => { console.warn('Failed to listen for connectivity rechecks:', error); });

    listen('verenu:storage-full', () => {
      appStore.recoveryStorageWarning = true;
    })
      .then((unlisten) => {
        if (!mounted) {
          unlisten();
          return;
        }
        stopRecoveryWriteListener = unlisten;
      })
      .catch((error) => { console.warn('Failed to listen for storage-full events:', error); });

    // Synchronous: startAutomaticUpdateChecks fires its first check in the
    // background and returns the cleanup immediately, so there's no unmount
    // race to guard and the interval is always registered before we return.
    try {
      stopAutomaticUpdateChecks = startAutomaticUpdateChecks();
    } catch (error) {
      console.error('Failed to start automatic update checks:', error);
    }

    try {
      stopLocalSttListeners = startLocalSttListeners();
      stopLocalLlmListeners = startLocalLlmListeners();
      stopDownloadManagerListeners = startDownloadManagerListeners();
      stopProviderStatusChecks = startProviderStatusChecks();
    } catch (error) {
      console.error('Failed to start listeners and status checks:', error);
    }

    refreshTranscriptionModel().catch((error) => {
      console.error('Failed to load transcription model:', error);
    });

    // Shown in the sidebar footer and About; fetched once here since both read it.
    getVersion()
      .then((version) => { appStore.appVersion = version; })
      .catch((error) => { console.error('Failed to read app version:', error); });

    const media = window.matchMedia?.('(prefers-color-scheme: dark)');
    const onSystemThemeChange = () => {
      if (appStore.appearanceMode === 'system') applyTheme();
    };
    media?.addEventListener?.('change', onSystemThemeChange);

    const connectivityPoll = startPolling(pingConnectivity, 60_000);

    // Live window dimensions drive data-width-class (fold/unfold, rotation,
    // split-screen, freeform). No restart, no lost state on reclassification.
    const stopViewport = trackViewport(
      () => ({ width: window.innerWidth, height: window.innerHeight }),
      (snapshot) => { viewport = snapshot; },
    );

    return () => {
      mounted = false;
      if (cleanupFn) cleanupFn();
      if (stopNotificationClickListener) stopNotificationClickListener();
      if (stopConnectivityRecheckListener) stopConnectivityRecheckListener();
      if (stopRecoveryWriteListener) stopRecoveryWriteListener();
      if (stopSettingsImportedListener) stopSettingsImportedListener();
      if (stopAutomaticUpdateChecks) stopAutomaticUpdateChecks();
      if (stopLocalSttListeners) stopLocalSttListeners();
      if (stopLocalLlmListeners) stopLocalLlmListeners();
      if (stopDownloadManagerListeners) stopDownloadManagerListeners();
      if (stopProviderStatusChecks) stopProviderStatusChecks();
      if (stopSyncListeners) stopSyncListeners();
      if (stopTitleBarMetricsListener) stopTitleBarMetricsListener();
      window.removeEventListener(SETTINGS_SAVE_ERROR_EVENT, onSettingsSaveError);
      media?.removeEventListener?.('change', onSystemThemeChange);
      connectivityPoll.stop();
      stopViewport();
    };
  });
</script>

<div
  class="app"
  class:app-windows={isWindows}
  data-android={isAndroid ? 'true' : 'false'}
  data-width-class={viewport.widthClass}
  data-compact-nav={compactNav ? 'true' : 'false'}
>
  {#if appStore.setupComplete === false}
    <Setup />
  {/if}
  <div class="body" inert={appStore.setupComplete === false}>
    <div class="rail">
      <Sidebar />
    </div>
    <div class="content-fade content-fade-top" class:visible={fadeTop && !appStore.settingsOpen} aria-hidden="true"></div>
    <div class="content-fade content-fade-bottom" class:visible={fadeBottom && !appStore.settingsOpen} aria-hidden="true"></div>
    <div
      class="content scroll-styled"
      class:content-behind={appStore.settingsOpen}
      inert={appStore.settingsOpen}
      bind:this={contentEl}
      use:scrollEdges={(top, bottom) => { fadeTop = top; fadeBottom = bottom; }}
    >
      {#key appStore.currentPage}
        <div
          class="page-wrapper"
          in:pageSwap={{ axis: 'y', distance: pageDir * motionPx(MOTION_PX.page), duration: motionMs(MOTION_MS.panel) }}
          out:pageSwap={{ axis: 'y', distance: -pageDir * motionPx(MOTION_PX.page), duration: motionMs(MOTION_MS.base + 40) }}
        >
          {#if appStore.currentPage === 'home'}
            <Home />
          {:else if appStore.currentPage === 'insights'}
            <Insights />
          {:else if appStore.currentPage === 'contexts'}
            <Contexts />
          {:else if appStore.currentPage === 'dictionary'}
            <Dictionary />
          {:else if appStore.currentPage === 'snippets'}
            <Snippets />
          {:else if appStore.currentPage === 'style'}
            <Style />
          {/if}
        </div>
      {/key}
    </div>
  </div>
  <Settings />
  {#if cleanupPromptEditor.open}
    <CleanupPromptModal />
  {/if}
  {#if syncStore.status?.pairing?.kind === 'incoming' && syncStore.status.pairing.phase !== 'failed'}
    <SyncPairModal />
  {/if}
  <DictationPill />

  {#if errorToast}
    <div
      class="error-toast"
      role="alert"
      style:bottom={!appStore.isOnline ? '66px' : '18px'}
      transition:fly={{ y: motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.base), easing: expoOut }}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink:0">
        <circle cx="12" cy="12" r="10"/><path d="M12 8v4M12 16h.01"/>
      </svg>
      <span>{errorToast}</span>
      {#if errorToastKind && settingsSectionForKind(errorToastKind)}
        <button class="toast-action ui-focus-ring" onclick={() => openSettingsSection(settingsSectionForKind(errorToastKind!)!)}>
          Fix in Settings
        </button>
      {/if}
      <button class="toast-close ui-focus-ring" onclick={() => { errorToast = ''; errorToastKind = null; clearTimeout(toastTimer); }}>✕</button>
    </div>
  {/if}
  {#if !appStore.isOnline}
    <div class="offline-toast" role="status" transition:fly={{ y: 6, duration: motionMs(180), easing: expoOut }}>
      <span class="offline-dot"></span>
      No internet connection
    </div>
  {/if}
  {#if compactNav}
    <MobileNav />
  {/if}
</div>

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(html, body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
  }

  :global(body) {
    font-family: var(--sans);
    color: var(--ink-soft);
    background: var(--paper);
    font-size: 13.5px;
    line-height: 1.5;
    -webkit-font-smoothing: antialiased;
    font-feature-settings: 'ss01', 'cv11';
  }

  :global(button) {
    font-family: inherit;
    cursor: pointer;
  }

  .app {
    width: 100%;
    height: 100vh;
    height: 100dvh;
    background: var(--paper);
    display: flex;
    flex-direction: column;
    font-family: var(--sans);
    position: relative;
    /* Edge-to-edge: draw behind system bars; children consume safe-area
       insets individually so desktop (where env() is 0) is unaffected. */
    padding-top: var(--safe-top);
    padding-left: var(--safe-left);
    padding-right: var(--safe-right);
  }

  /* The native Windows caption is non-client chrome, so it does not consume
     space inside this app shell. Keep Windows pages close to that boundary
     without changing the established macOS page rhythm. */
  .app.app-windows {
    --page-pad-y: calc(var(--native-titlebar-height, 32px) + 10px);
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
    padding: 0 0 var(--app-gutter) 0;
    gap: var(--app-gutter);
    position: relative;
  }

  /* Layout-neutral wrapper so compact Android can hide the desktop rail
     without touching Sidebar's own layout. */
  .rail {
    display: contents;
  }

  .app[data-compact-nav='true'] .rail {
    display: none;
  }

  .app[data-compact-nav='true'] .body {
    gap: 0;
    padding-bottom: 0;
  }

  /* The rail is gone, so nothing should offset for its width anymore, and
     any full-bleed overlay (Settings, modals, toasts) needs to know how much
     room the sticky bottom nav actually takes so its content isn't hidden
     behind it. */
  .app[data-compact-nav='true'] {
    --sidebar-w: 0px;
    --mobile-nav-h: calc(60px + var(--safe-bottom));
  }

  /* MainActivity applies the real Android WindowInsets to the WebView content
     root. Do not add the WebView's CSS env() values again on Android, since
     some devices expose them inconsistently and would otherwise double-pad. */
  .app[data-android='true'] {
    --safe-top: 0px;
    --safe-bottom: 0px;
    --safe-left: 0px;
    --safe-right: 0px;
  }

  /*
   * The scroll fades are cut to the desktop shell: inset from the left for the
   * rail, from the right for the scrollbar gutter, and lifted off the bottom by
   * --app-gutter. None of those exist in the compact layout, which left the
   * bottom fade hanging in mid-air above the nav bar with unfaded content on
   * either side of it. Run them edge to edge and flush to the bar instead.
   */
  .app[data-compact-nav='true'] .content-fade {
    left: 0;
    right: 0;
  }

  .app[data-compact-nav='true'] .content-fade-bottom {
    bottom: 0;
  }

  /* Compact windows (phones, narrow foldables, snapped split-screen):
     tighten page rhythm and keep the gesture bar clear of content. */
  .app[data-width-class='compact'] {
    --page-pad-x: 16px;
    --page-pad-y: 16px;
  }

  .app[data-width-class='compact'] .content {
    padding-bottom: env(safe-area-inset-bottom, 0px);
  }

  .app[data-width-class='compact'] .page-wrapper {
    padding-right: 0;
  }

  /*
   * Phones get a full-height page column so views can distribute themselves
   * down the screen instead of stacking into the top third and leaving the
   * rest of a tall display empty. .page-wrapper already resolves a definite
   * min-height against the content grid area, so flex children can claim it.
   */
  .app[data-compact-nav='true'] .page-wrapper {
    display: flex;
    flex-direction: column;
  }

  /*
   * Soft top/bottom scroll fades over the main content column (same treatment as
   * the settings panel). Geometry mirrors the content region: it starts to the
   * right of the sidebar and stops short of the scrollbar gutter and the bottom
   * gutter. They fade to the page background and only appear when there's more
   * to scroll in that direction — and never while settings covers the content.
   */
  .content-fade {
    position: absolute;
    left: calc(var(--sidebar-w) + var(--app-gutter));
    right: var(--scrollbar-w, 0);
    height: 30px;
    pointer-events: none;
    z-index: 5;
    opacity: 0;
    transition: opacity 180ms ease;
  }

  .content-fade.visible { opacity: 1; }

  .content-fade-top {
    top: 0;
    background: linear-gradient(to bottom, var(--paper), transparent);
  }

  .content-fade-bottom {
    bottom: var(--app-gutter);
    background: linear-gradient(to top, var(--paper), transparent);
  }

  @media (prefers-reduced-motion: reduce) {
    .content-fade { transition: none; }
  }

  .content {
    flex: 1;
    background: transparent;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: stable;
    position: relative;
    display: grid;
    justify-items: center;
    min-width: 0;
  }

  .content::-webkit-scrollbar-thumb { border: 3px solid var(--paper); }

  /*
   * Opening settings is a page change, not a panel appearing over a frozen
   * page: the current view rises and fades out with the same vocabulary the
   * Home/Dictionary/Style swaps use, while the settings page enters beneath the
   * fading wash. Without this the underlying page just sat there and the
   * transition read as nothing happening.
   */
  .content {
    /* Both properties on one curve — cubic-bezier(0.33, 1, 0.68, 1) is the CSS
       form of cubicOut, matching pageSwap. Opacity was on `ease` before, which
       is what made the exit read as slightly out of step with the movement.
       Keep --content-swap-y in sync with SETTINGS_SWAP_PX in Settings.svelte. */
    transition:
      opacity var(--content-swap-ms) cubic-bezier(0.33, 1, 0.68, 1),
      transform var(--content-swap-ms) cubic-bezier(0.33, 1, 0.68, 1);
    --content-swap-ms: 320ms;
    --content-swap-y: 26px;
  }

  .content.content-behind {
    opacity: 0;
    transform: translate3d(0, calc(var(--content-swap-y) * -1), 0);
    pointer-events: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .content {
      --content-swap-ms: 190ms;
      --content-swap-y: 10px;
    }
  }

  .page-wrapper {
    grid-area: 1 / 1;
    width: 100%;
    max-width: 100%;
    min-width: 0;
    min-height: calc(100% + 1px);
    padding-right: 14px;
  }

  .error-toast {
    position: absolute;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--danger-bg);
    border: 1px solid var(--danger-line);
    border-radius: var(--r-sm);
    padding: 9px 14px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--danger);
    box-shadow: var(--shadow-popover);
    /* Settings sits above the app content; errors must remain visible while a
       setting change fails inside that page. */
    z-index: 80;
    max-width: 480px;
    transition: bottom 0.15s ease;
  }

  .toast-close {
    background: transparent;
    border: none;
    color: var(--danger);
    opacity: 0.6;
    font-size: 11px;
    cursor: pointer;
    margin-left: 4px;
    padding: 0;
    line-height: 1;
  }
  .toast-close:hover { opacity: 1; }

  .toast-action {
    background: transparent;
    border: 1px solid var(--danger-line);
    border-radius: 6px;
    color: var(--danger);
    font-size: 11.5px;
    font-weight: 600;
    padding: 2px 9px;
    cursor: pointer;
    margin-left: 6px;
    flex-shrink: 0;
    transition: background 0.12s;
  }
  .toast-action:hover { background: var(--danger-bg); }

  .offline-toast {
    position: absolute;
    bottom: 18px;
    left: 0;
    right: 0;
    margin-inline: auto;
    width: fit-content;
    background: var(--danger-bg);
    border: 1px solid var(--danger-line);
    border-radius: var(--r-sm);
    padding: 9px 14px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--danger);
    box-shadow: var(--shadow-popover);
    z-index: 20;
    max-width: 480px;
  }

  .offline-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    flex-shrink: 0;
    animation: dot-pulse 2s ease-in-out infinite;
  }

  @keyframes dot-pulse {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0.35; }
  }

  @media (max-width: 720px) {
    .app { --sidebar-w: 58px; }
  }
</style>
