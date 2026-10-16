<script lang="ts">
  import { onMount } from 'svelte';
  import { appStore } from './lib/stores';
  import { cleanupPromptEditor } from './lib/stores.svelte';
  import { isWindows, isLinux } from './lib/platform';
  import Sidebar from './lib/components/layout/Sidebar.svelte';
  import Home from './lib/views/Home.svelte';
  import DeferredView from './lib/components/DeferredView.svelte';
  import { lazyComponent } from './lib/lazyComponent.svelte';
  import { isBrowserDevSession } from './lib/devSession';
  import AgentAccessibilityDump from './lib/components/AgentAccessibilityDump.svelte';
  import Settings from './lib/views/Settings.svelte';
  import SubAppSheet from './lib/components/SubAppSheet.svelte';
  import { contextsStore, loadContexts } from './lib/contextsStore.svelte';
  import { SUB_APP_CAPTURED_EVENT, SUB_APP_CAPTURE_FAILED_EVENT, type SubAppCapture } from './lib/subApps';
  import { startSyncListeners, syncStore } from './lib/syncStore.svelte';
  import DictationPill from './lib/components/layout/DictationPill.svelte';
  import { getVersion, invoke, isTauriRuntime, listen } from './lib/tauri';
  import { startAutomaticUpdateChecks } from './lib/updates';
  import { startPolling } from './lib/polling';
  import { startGithubRefresh } from './lib/githubStore.svelte';
  import { startT3Refresh } from './lib/t3Store.svelte';
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
  import { OMARCHY_THEME_EVENT, applyOmarchyPalette, effectiveAccent, isOmarchyTheme, resolvePalette, type OmarchyTheme } from './lib/omarchyTheme';
  import { normalizeCustomTheme, normalizeSavedThemes } from './lib/customTheme';
  import { themeEditor } from './lib/themeEditor.svelte';
  import ThemeEditorDock from './lib/components/settings/ThemeEditorDock.svelte';
  import { isAndroid } from './lib/platform';
  import MobileNav from './lib/components/layout/MobileNav.svelte';
  import {
    snapshotForSize,
    trackViewport,
    type ViewportSnapshot,
  } from './lib/android/viewport';

  type EffectiveTheme = 'light' | 'dark';
  const pages = {
    insights: lazyComponent(() => import('./lib/views/Insights.svelte')),
    contexts: lazyComponent(() => import('./lib/views/Contexts.svelte')),
    dictionary: lazyComponent(() => import('./lib/views/Dictionary.svelte')),
    snippets: lazyComponent(() => import('./lib/views/Snippets.svelte')),
    style: lazyComponent(() => import('./lib/views/Style.svelte')),
  };
  const setup = lazyComponent(() => import('./lib/views/Setup.svelte'));
  const devPanel = lazyComponent(() => import('./lib/components/DevSessionPanel.svelte'));
  const cleanupModal = lazyComponent(() => import('./lib/components/settings/CleanupPromptModal.svelte'));
  const syncModal = lazyComponent(() => import('./lib/components/settings/SyncPairModal.svelte'));
  import type { AppearanceMode } from './lib/settings';
  type NativeTitleBarMetrics = { height: number; leftInset: number; rightInset: number; scaleFactor: number };

  function applyNativeTitleBarMetrics(metrics: NativeTitleBarMetrics | null) {
    if (!metrics) return;
    const root = document.documentElement;
    root.style.setProperty('--native-titlebar-height', `${metrics.height}px`);
    root.style.setProperty('--native-caption-left-inset', `${metrics.leftInset}px`);
    root.style.setProperty('--native-caption-right-inset', `${metrics.rightInset}px`);
  }

  function systemTheme(): EffectiveTheme {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  function activePalette() {
    return resolvePalette(appStore.appearanceMode, appStore.omarchyTheme, appStore.customTheme);
  }

  function effectiveTheme(mode: AppearanceMode): EffectiveTheme {
    if (mode === 'system' || mode === 'omarchy' || mode === 'custom') return activePalette()?.mode ?? systemTheme();
    return mode;
  }

  function applyTheme() {
    const theme = effectiveTheme(appStore.appearanceMode);
    document.documentElement.dataset.theme = theme;
    applyOmarchyPalette(document.documentElement, activePalette());
    // isLinux is UA-derived and Android's UA also contains "Linux", so it
    // must be paired with !isAndroid before it means "Linux desktop".
    if ((isWindows || (isLinux && !isAndroid)) && isTauriRuntime()) {
      const rootStyles = getComputedStyle(document.documentElement);
      const surface = rootStyles.getPropertyValue('--paper').trim();
      const text = rootStyles.getPropertyValue('--ink').trim();
      const border = rootStyles.getPropertyValue('--line').trim();
      const hover = rootStyles.getPropertyValue('--control-hover').trim();
      const sidebarSurface = rootStyles.getPropertyValue('--sidebar-bg').trim();
      const appEl = document.querySelector('.app');
      const sidebarWidth = (appEl ? getComputedStyle(appEl) : rootStyles)
        .getPropertyValue('--sidebar-w')
        .trim();
      invoke('set_native_titlebar_theme', {
        dark: theme === 'dark',
        surface: surface || null,
        text: text || null,
        border: border || null,
        hover: hover || null,
        sidebarSurface: sidebarSurface || null,
        sidebarWidth: sidebarWidth || null,
      }).catch((error) => {
        console.error('Failed to sync native title bar theme:', error);
      });
    }
  }

  $effect(() => {
    appStore.appearanceMode;
    appStore.omarchyTheme;
    appStore.customTheme;
    if (typeof document !== 'undefined') applyTheme();
  });

  $effect(() => {
    const accentColor = effectiveAccent(appStore.accentColor, activePalette());
    if (typeof document !== 'undefined') applyAccentTheme(document.documentElement, accentColor);
  });

  let viewport = $state<ViewportSnapshot>(
    snapshotForSize(1280, 800),
  );
  const compactNav = $derived(isAndroid && viewport.widthClass !== 'expanded');
  $effect(() => {
    appStore.mobileCompact = isAndroid && viewport.widthClass === 'compact';
  });

  let keyboardOpen = $state(false);
  let stableViewport = { width: 0, height: 0 };
  function nativeKeyboardOpen(): boolean | null {
    const value = (window as any).VerenuInsets?.imeVisible?.();
    return typeof value === 'boolean' ? value : null;
  }
  function trackKeyboard(snapshot: ViewportSnapshot) {
    if (!isAndroid) return;
    const native = nativeKeyboardOpen();
    if (native !== null) {
      keyboardOpen = native;
      return;
    }
    if (Math.abs(snapshot.widthDp - stableViewport.width) > 1) {
      stableViewport = { width: snapshot.widthDp, height: snapshot.heightDp };
    } else if (snapshot.heightDp > stableViewport.height) {
      stableViewport.height = snapshot.heightDp;
    }
    keyboardOpen = snapshot.heightDp < stableViewport.height - 140;
  }

  let errorToast = $state('');
  let errorToastKind = $state<ErrorKind | null>(null);
  let toastTimer: ReturnType<typeof setTimeout>;
  let modelNotice = $state('');
  let modelNoticeTimer: ReturnType<typeof setTimeout>;
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
      appStore.settingsMobileList = false;
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
    appStore.settingsMobileList = false;
    appStore.settingsOpen = true;
    errorToast = '';
    errorToastKind = null;
    clearTimeout(toastTimer);
  }

  function showErrorToast(raw: string) {
    const classified = classifyIpcError(raw);
    errorToast = classified.message;
    errorToastKind = raw ? classified.kind : null;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { errorToast = ''; errorToastKind = null; }, Math.min(20000, Math.max(8000, errorToast.length * 65)));
  }

  onMount(() => {
    const stopGithubRefresh = startGithubRefresh();
    const stopT3Refresh = startT3Refresh();
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
    let stopModelNoticeListener: (() => void) | undefined;
    void listen<string>('verenu:model-notice', (event) => {
      modelNotice = [modelNotice, event.payload].filter(Boolean).join(' ');
      clearTimeout(modelNoticeTimer);
      modelNoticeTimer = setTimeout(() => { modelNotice = ''; }, 12000);
    }).then(unlisten => { if (!mounted) unlisten(); else stopModelNoticeListener = unlisten; });
    const onSettingsSaveError = (event: Event) => {
      const message = (event as CustomEvent<unknown>).detail;
      showErrorToast(typeof message === 'string' ? message : '');
    };
    const reportFrontendException = (kind: 'frontend_unhandled' | 'frontend_handled', handled: boolean) => {
      if (isTauriRuntime()) {
        void invoke('analytics_frontend_exception', { kind, handled }).catch(() => {});
      }
    };
    const onWindowError = () => reportFrontendException('frontend_unhandled', false);
    const onUnhandledRejection = () => reportFrontendException('frontend_unhandled', false);
    window.addEventListener(SETTINGS_SAVE_ERROR_EVENT, onSettingsSaveError);
    window.addEventListener('error', onWindowError);
    window.addEventListener('unhandledrejection', onUnhandledRejection);

    if ((isWindows || (isLinux && !isAndroid)) && isTauriRuntime()) {
      invoke<NativeTitleBarMetrics>('get_native_titlebar_metrics')
        .then(applyNativeTitleBarMetrics)
        .catch((error) => console.error('Failed to read native title bar metrics:', error));
      listen<NativeTitleBarMetrics>('verenu:native-titlebar-metrics', (event) => applyNativeTitleBarMetrics(event.payload))
        .then((unlisten) => { stopTitleBarMetricsListener = unlisten; })
        .catch((error) => console.error('Failed to listen for native title bar metrics:', error));
    }

    async function readSettingsWithRetry<T>(read: () => Promise<T>, attempts = 5): Promise<T> {
      for (let attempt = 1; ; attempt++) {
        try {
          return await read();
        } catch (error) {
          if (attempt >= attempts || !mounted) throw error;
          await new Promise((resolve) => setTimeout(resolve, 300 * attempt));
        }
      }
    }

    async function reloadGlobalSettings() {
      try {
        const savedThemesRead = readSettingsWithRetry(() =>
          invoke<unknown>('get_setting', { key: 'custom_themes' }),
        ).catch(() => null);
        const [done, appearance, accentColor, customTheme, forceSetupOnLaunch, cleanupEnabled, betaUpdatesEnabled, legacyFeaturesEnabled, syncEnabled, ruinAccessibility, devModeOnStartup, subAppCaptureHotkey] = await readSettingsWithRetry(() => Promise.all([
          invoke<boolean | null>('get_setting', { key: 'setup_complete' }),
          invoke<AppearanceMode | null>('get_setting', { key: 'appearance_mode' }),
          invoke<string | null>('get_setting', { key: 'accent_color' }),
          invoke<unknown>('get_setting', { key: 'custom_theme' }),
          invoke<boolean | null>('get_setting', { key: 'force_setup_on_launch' }),
          invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'beta_updates_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'legacy_features_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'sync_enabled' }),
          invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' }),
          invoke<boolean | null>('get_setting', { key: 'dev_mode_on_startup' }),
          invoke<string | null>('get_setting', { key: 'sub_app_capture_hotkey' }),
        ]));
        appStore.setupComplete = forceSetupOnLaunch ? false : done === true;
        appStore.savedThemes = normalizeSavedThemes(await savedThemesRead);
        if (!themeEditor.open) {
          if (appearance === 'light' || appearance === 'dark' || appearance === 'system' || appearance === 'omarchy' || appearance === 'custom') {
            appStore.appearanceMode = appearance;
          }
          appStore.accentColor = normalizeAccentColor(accentColor);
          appStore.customTheme = normalizeCustomTheme(customTheme);
        }
        appStore.cleanupEnabled = cleanupEnabled ?? true;
        appStore.betaUpdatesEnabled = betaUpdatesEnabled ?? false;
        appStore.legacyFeaturesEnabled = legacyFeaturesEnabled ?? false;
        appStore.syncEnabled = syncEnabled ?? false;
        appStore.ruinAccessibility = ruinAccessibility ?? false;
        appStore.devModeOnStartup = devModeOnStartup ?? false;
        appStore.subAppCaptureHotkey = typeof subAppCaptureHotkey === 'string' ? subAppCaptureHotkey : null;
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

    try {
      if (!isBrowserDevSession()) stopAutomaticUpdateChecks = startAutomaticUpdateChecks();
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

    getVersion()
      .then((version) => { appStore.appVersion = version; })
      .catch((error) => { console.error('Failed to read app version:', error); });

    const openSubAppCapture = async (capture: SubAppCapture | null) => {
      if (!capture) return;
      await loadContexts();
      contextsStore.subAppSheet = { mode: 'capture', capture };
    };
    const stopSubAppListeners: Array<() => void> = [];
    if (isTauriRuntime()) {
      void listen<SubAppCapture>(SUB_APP_CAPTURED_EVENT, () => {
        void invoke<SubAppCapture | null>('take_pending_sub_app_capture').then(openSubAppCapture);
      }).then((unlisten) => stopSubAppListeners.push(unlisten));
      void listen<string>(SUB_APP_CAPTURE_FAILED_EVENT, (event) => {
        showErrorToast(event.payload || 'Nothing to capture.');
      }).then((unlisten) => stopSubAppListeners.push(unlisten));
      void invoke<SubAppCapture | null>('take_pending_sub_app_capture')
        .then(openSubAppCapture)
        .catch(() => {});
    }

    let stopOmarchyTheme: (() => void) | undefined;
    if (isLinux && !isAndroid && isTauriRuntime()) {
      const setOmarchyTheme = (value: unknown) => {
        appStore.omarchyTheme = isOmarchyTheme(value) ? value : null;
      };
      invoke<OmarchyTheme | null>('get_omarchy_theme').then(setOmarchyTheme).catch(() => {});
      void listen<OmarchyTheme | null>(OMARCHY_THEME_EVENT, (event) => setOmarchyTheme(event.payload))
        .then((unlisten) => { stopOmarchyTheme = unlisten; });
    }

    const media = window.matchMedia?.('(prefers-color-scheme: dark)');
    const onSystemThemeChange = () => {
      if (appStore.appearanceMode === 'system') applyTheme();
    };
    media?.addEventListener?.('change', onSystemThemeChange);

    const connectivityPoll = startPolling(pingConnectivity, 60_000);

    const stopViewport = trackViewport(
      () => ({ width: window.innerWidth, height: window.innerHeight }),
      (snapshot) => { viewport = snapshot; trackKeyboard(snapshot); },
    );

    let stopInsets = () => {};
    if (isAndroid) {
      const applyInsets = () => {
        const px = Number((window as any).VerenuInsets?.bottomInsetCssPx?.());
        document.documentElement.style.setProperty(
          '--android-safe-bottom',
          `${Number.isFinite(px) ? Math.max(0, px) : 0}px`,
        );
      };
      applyInsets();
      const timers = [150, 600, 1500].map((ms) => window.setTimeout(applyInsets, ms));
      window.addEventListener('resize', applyInsets);
      let focusTimer: ReturnType<typeof setTimeout> | undefined;
      const revealFocused = (event: FocusEvent) => {
        const target = event.target;
        if (!(target instanceof HTMLElement) || !target.matches('input, textarea, [contenteditable="true"]')) return;
        clearTimeout(focusTimer);
        focusTimer = setTimeout(() => {
          if (target.isConnected) target.scrollIntoView({ block: 'center', behavior: 'smooth' });
        }, 320);
      };
      document.addEventListener('focusin', revealFocused);
      const onNativeIme = () => {
        const native = nativeKeyboardOpen();
        if (native !== null) keyboardOpen = native;
      };
      window.addEventListener('verenu-ime', onNativeIme);
      onNativeIme();
      stopInsets = () => {
        window.removeEventListener('verenu-ime', onNativeIme);
        timers.forEach((t) => window.clearTimeout(t));
        window.removeEventListener('resize', applyInsets);
        document.removeEventListener('focusin', revealFocused);
        clearTimeout(focusTimer);
      };
    }

    return () => {
      stopGithubRefresh();
      stopT3Refresh();
      mounted = false;
      stopInsets();
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
      stopModelNoticeListener?.();
      clearTimeout(modelNoticeTimer);
      window.removeEventListener(SETTINGS_SAVE_ERROR_EVENT, onSettingsSaveError);
      window.removeEventListener('error', onWindowError);
      window.removeEventListener('unhandledrejection', onUnhandledRejection);
      media?.removeEventListener?.('change', onSystemThemeChange);
      stopOmarchyTheme?.();
      for (const stop of stopSubAppListeners) stop();
      connectivityPoll.stop();
      stopViewport();
    };
  });
</script>

{#if isBrowserDevSession()}<DeferredView view={devPanel} />{/if}

<div
  class="app"
  class:app-windows={isWindows}
  class:app-linux={isLinux && !isAndroid}
  data-android={isAndroid ? 'true' : 'false'}
  data-width-class={viewport.widthClass}
  data-compact-nav={compactNav ? 'true' : 'false'}
  data-keyboard={keyboardOpen ? 'open' : 'closed'}
>
  {#if isLinux && !isAndroid}
    <div class="native-drag-region" data-tauri-drag-region aria-hidden="true"></div>
  {/if}
  {#if appStore.setupComplete === false}
    <div inert={appStore.settingsOpen} style:visibility={appStore.settingsOpen ? 'hidden' : 'visible'}>
      <DeferredView view={setup} />
    </div>
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
          {:else}
            <DeferredView view={pages[appStore.currentPage]} />
          {/if}
        </div>
      {/key}
    </div>
  </div>
  <Settings />
  <ThemeEditorDock />
  {#if cleanupPromptEditor.open}
    <DeferredView view={cleanupModal} />
  {/if}
  {#if syncStore.status?.pairing?.kind === 'incoming' && syncStore.status.pairing.phase !== 'failed'}
    <DeferredView view={syncModal} />
  {/if}
  {#if contextsStore.subAppSheet}
    {#key contextsStore.subAppSheet}
      <SubAppSheet sheet={contextsStore.subAppSheet} onClose={() => (contextsStore.subAppSheet = null)} />
    {/key}
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
      <span class="toast-message">{errorToast}</span>
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
  {#if modelNotice}
    <div class="model-notice" role="status" style:bottom={!appStore.isOnline ? '66px' : '18px'}>
      <span>{modelNotice}</span>
      <button class="btn-ghost btn-compact" aria-label="Dismiss model notice" onclick={() => { modelNotice = ''; clearTimeout(modelNoticeTimer); }}>Dismiss</button>
    </div>
  {/if}
  {#if compactNav && !keyboardOpen}
    <MobileNav />
  {/if}
</div>

<style>
  .model-notice {
    position: fixed;
    right: 18px;
    z-index: 150;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(520px, calc(100vw - 36px));
    padding: 12px 16px;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--bg-elev);
    color: var(--ink);
    font-size: 12px;
    line-height: 1.5;
  }
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
    padding-top: var(--safe-top);
    padding-left: var(--safe-left);
    padding-right: var(--safe-right);
  }

  .app.app-windows {
    --page-pad-y: calc(var(--native-titlebar-height, 32px) + 10px);
  }

  .app.app-linux {
    --page-pad-y: clamp(20px, 2vw, 24px);
    --native-chrome-inline-clearance: max(0px, calc(var(--native-caption-right-inset, 54px) - var(--page-pad-x) + 8px));
    --native-chrome-below-clearance: max(0px, calc(var(--native-titlebar-height, 32px) - var(--page-pad-y) + 8px));
  }

  :global(.app.app-linux .native-chrome-corner--inline) {
    margin-right: var(--native-chrome-inline-clearance);
  }

  :global(.app.app-linux .native-chrome-corner--below) {
    margin-top: var(--native-chrome-below-clearance);
  }

  .native-drag-region {
    position: absolute;
    z-index: 59;
    top: 0;
    left: var(--sidebar-w);
    right: var(--native-caption-right-inset, 126px);
    height: var(--native-titlebar-height, 32px);
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
    padding: 0 0 var(--app-gutter) 0;
    gap: var(--app-gutter);
    position: relative;
  }

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

  .app[data-compact-nav='true'] {
    --sidebar-w: 0px;
    --mobile-nav-h: calc(68px + var(--safe-bottom));
  }

  /* MainActivity applies the top/side WindowInsets to the WebView content root,
     and reports only the part of the navigation bar the WebView really draws
     under (--android-safe-bottom). Do not add the WebView's CSS env() values
     again on Android, since some devices expose them inconsistently and would
     otherwise double-pad. */
  .app[data-android='true'] {
    --safe-top: 0px;
    --safe-bottom: var(--android-safe-bottom, 0px);
    --safe-left: 0px;
    --safe-right: 0px;
  }

  .app[data-compact-nav='true'] .content-fade {
    left: 0;
    right: 0;
  }

  .app[data-compact-nav='true'] .content-fade-bottom {
    bottom: 0;
  }

  .app[data-compact-nav='true'] {
    --page-pad-x: 16px;
    --page-pad-y: 16px;
  }

  .app[data-compact-nav='true'] .content {
    padding-bottom: env(safe-area-inset-bottom, 0px);
  }

  .app[data-compact-nav='true'] .page-wrapper {
    padding-right: 0;
  }

  .app[data-compact-nav='true'] .page-wrapper {
    display: flex;
    flex-direction: column;
  }

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

  .content {
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
    z-index: 80;
    width: max-content;
    max-width: min(560px, calc(100% - 36px));
    transition: bottom 0.15s ease;
  }

  .toast-message {
    min-width: 0;
    overflow-wrap: anywhere;
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
