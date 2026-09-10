<script lang="ts">
  import { onMount } from 'svelte';
  import { appStore } from '../stores';
  import { getVersion, invoke, listen } from '../tauri';
  import { classifyIpcError } from '../errors';
  import { isMac, isWindows } from '../platform';
  import { reducedMotionEnabled } from '../motion';
  import { localSttStore } from '../localSttStore.svelte';
  import { localLlmStore } from '../localLlmStore.svelte';
  import { syncStore } from '../syncStore.svelte';
  import { contextsStore } from '../contextsStore.svelte';
  import {
    DUMP_REGION_ATTR,
    RUIN_ACCESSIBILITY_EVENT,
    annotateAccessibilityDump,
    applyDumpWindowTitle,
    buildAgentDump,
    collectDomInventory,
    compactDumpTitle,
    getDumpEvents,
    pushDumpEvent,
    restoreAccessibilityDump,
    restoreDumpWindowTitle,
    type DumpWindowKind,
  } from '../agentAccessibilityDump';

  let {
    windowKind,
    extras = {},
  }: {
    windowKind: DumpWindowKind;
    extras?: Record<string, unknown>;
  } = $props();

  let enabled = $state(false);
  let dumpText = $state('');
  let dumpTitle = $state('');
  let settings = $state<Record<string, unknown>>({});
  let errors = $state<string[]>([]);
  let inventory = $state<string[]>([]);
  let version = $state('');
  let diagnosticsSummary = $state<Record<string, unknown>>({});

  const ERRORS_CAP = 8;

  function pushError(raw: string) {
    const classified = classifyIpcError(raw);
    const line = `${classified.kind}: ${classified.message}`;
    errors = [...errors.slice(-(ERRORS_CAP - 1)), line];
  }

  function mainExtras(): Record<string, unknown> {
    return {
      page: appStore.currentPage,
      settingsOpen: appStore.settingsOpen,
      settingsSection: appStore.settingsSection,
      appearanceMode: appStore.appearanceMode,
      accentColor: appStore.accentColor,
      cleanupEnabled: appStore.cleanupEnabled,
      legacyFeaturesEnabled: appStore.legacyFeaturesEnabled,
      syncEnabled: appStore.syncEnabled,
      pillState: appStore.pillState,
      setupComplete: appStore.setupComplete,
      snippetCount: appStore.snippets.length,
      dictionaryCount: appStore.dictionary.length,
      updateInfo: appStore.updateInfo,
      betaUpdatesEnabled: appStore.betaUpdatesEnabled,
      providerStatusAlerts: appStore.providerStatusAlerts,
      apiHealthy: appStore.apiHealthy,
      isOnline: appStore.isOnline,
      devModeEnabled: appStore.devModeEnabled,
      platform: { isWindows, isMac },
      reducedMotion: reducedMotionEnabled(),
      diagnostics: diagnosticsSummary,
      localStt: {
        current: localSttStore.state.current_model_id,
        loaded: localSttStore.state.is_loaded,
        loading: localSttStore.state.is_loading,
        downloading: localSttStore.state.is_downloading,
        downloadingModel: localSttStore.state.downloading_model_id,
        modelCount: localSttStore.models.length,
      },
      localLlm: {
        current: localLlmStore.state.current_model_id,
        loaded: localLlmStore.state.is_loaded,
        loading: localLlmStore.state.is_loading,
        downloading: localLlmStore.state.is_downloading,
        endpoint: localLlmStore.state.endpoint,
        runtimeInstalled: localLlmStore.runtime.installed,
        runtimeBackend: localLlmStore.runtime.backend,
        modelCount: localLlmStore.models.length,
      },
      sync: syncStore.status
        ? {
            listenerActive: syncStore.status.listener_active,
            device: syncStore.status.this_device.name,
            peerCount: syncStore.status.peers.length,
            discoveredCount: syncStore.status.discovered.length,
            pairingPhase: syncStore.status.pairing?.phase ?? null,
            lastErrorHint: syncStore.status.last_error_hint,
          }
        : { loaded: syncStore.loaded },
      contextCount: contextsStore.contexts.length,
      contextSelected: contextsStore.selectedId,
      ...extras,
    };
  }

  function rebuild() {
    if (!enabled) return;
    // Keep the freshly collected inventory local while building the snapshot.
    // Reading the stateful `inventory` value inside this function and then
    // assigning it below makes the `$effect` that calls `rebuild()` depend on
    // a value it also writes, which trips Svelte's effect update-depth guard.
    const nextInventory = collectDomInventory();
    const snapshot = {
      windowKind,
      version: version || appStore.appVersion,
      extras: windowKind === 'main' ? mainExtras() : extras,
      settings,
      errors,
      inventory: nextInventory,
      recent: getDumpEvents(),
    };
    const nextTitle = compactDumpTitle(snapshot);
    const nextText = buildAgentDump(snapshot);
    inventory = nextInventory;
    dumpTitle = nextTitle;
    dumpText = nextText;
    // The pill window is content-fit and uses an empty title. Rewriting
    // document.title here has no SnapShot value and can disturb native sizing.
    if (windowKind === 'main') applyDumpWindowTitle(nextTitle);
    annotateAccessibilityDump();
  }

  async function loadSettings() {
    try {
      settings = (await invoke<Record<string, unknown>>('get_all_settings')) ?? {};
    } catch (error) {
      settings = { loadError: String(error) };
    }
  }

  async function loadDiagnosticsSummary() {
    try {
      const snapshot = await invoke<Record<string, unknown>>('get_diagnostics_snapshot');
      const health = snapshot.health && typeof snapshot.health === 'object' ? snapshot.health as Record<string, unknown> : {};
      const resource = snapshot.current_resource && typeof snapshot.current_resource === 'object' ? snapshot.current_resource as Record<string, unknown> : {};
      const groups = Array.isArray(snapshot.failure_groups) ? snapshot.failure_groups : [];
      const operations = Array.isArray(snapshot.operations) ? snapshot.operations : [];
      diagnosticsSummary = {
        cpu: resource.cpu_percent ?? null, residentBytes: resource.resident_bytes ?? null,
        activeTraces: health.active_trace_count ?? 0, recentFailures: health.retained_failure_count ?? 0,
        failureFingerprints: groups.slice(-3).map((item) => { const group = item && typeof item === 'object' ? item as Record<string, unknown> : {}; return `${String(group.fingerprint ?? 'unknown')}:${String(group.count ?? 0)}`; }),
        hottestOperations: operations.slice(0, 3).map((item) => { const operation = item && typeof item === 'object' ? item as Record<string, unknown> : {}; return `${String(operation.operation ?? 'unknown')}:${String(operation.calls ?? 0)}`; }),
        profiler: snapshot.profiler_enabled === true || snapshot.profiling_recording === true,
        dropped: Number(health.dropped_logs ?? 0) + Number(health.dropped_failures ?? 0) + Number(health.dropped_traces ?? 0),
      };
    } catch { diagnosticsSummary = { unavailable: true }; }
  }

  async function setEnabled(next: boolean) {
    enabled = next;
    if (windowKind === 'main') {
      appStore.ruinAccessibility = next;
      if (next) appStore.devModeEnabled = true;
    }
    if (!next) {
      dumpText = '';
      dumpTitle = '';
      restoreAccessibilityDump();
      if (windowKind === 'main') restoreDumpWindowTitle();
      return;
    }
    await loadSettings();
    await loadDiagnosticsSummary();
    rebuild();
  }

  onMount(() => {
    let active = true;
    let unlistenFlag: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    let unlistenPillState: (() => void) | null = null;
    let unlistenPillStage: (() => void) | null = null;
    let observer: MutationObserver | null = null;
    let rebuildTimer: ReturnType<typeof setTimeout> | null = null;

    const scheduleRebuild = () => {
      if (!enabled) return;
      if (rebuildTimer) return;
      rebuildTimer = setTimeout(() => {
        rebuildTimer = null;
      if (active && enabled) rebuild();
      }, 250);
    };

    getVersion()
      .then((value) => {
        if (active) version = value;
      })
      .catch((error) => console.error('Failed to read app version for accessibility dump:', error));

    invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' })
      .then((value) => {
        if (!active) return;
        return setEnabled(value === true);
      })
      .catch((error) => console.error('Failed to load ruin_accessibility:', error));

    listen<boolean>(RUIN_ACCESSIBILITY_EVENT, (event) => {
      void setEnabled(Boolean(event.payload));
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        unlistenFlag = unlisten;
      })
      .catch((error) => console.error('Failed to listen for ruin-accessibility changes:', error));

    listen<string>('pill-state', (event) => {
      pushDumpEvent('pill-state', String(event.payload ?? ''));
      scheduleRebuild();
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        unlistenPillState = unlisten;
      })
      .catch(() => {});

    listen<string>('pill-stage', (event) => {
      pushDumpEvent('pill-stage', String(event.payload ?? ''));
      scheduleRebuild();
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        unlistenPillStage = unlisten;
      })
      .catch(() => {});

    listen<string>('verenu:error', (event) => {
      pushError(event.payload ?? '');
      pushDumpEvent('error', classifyIpcError(event.payload ?? '').kind);
      scheduleRebuild();
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        unlistenError = unlisten;
      })
      .catch((error) => console.error('Failed to listen for errors in accessibility dump:', error));

    if (typeof MutationObserver !== 'undefined') {
      observer = new MutationObserver((records) => {
        const relevant = records.some((record) => {
          const target = record.target;
          if (!(target instanceof Element)) return true;
          return !target.closest(`[${DUMP_REGION_ATTR}]`);
        });
        if (relevant) scheduleRebuild();
      });
      observer.observe(document.documentElement, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ['aria-checked', 'aria-expanded', 'aria-selected', 'aria-current', 'class', 'data-setting-target', 'data-debug-id'],
      });
    }

    const interval = setInterval(() => {
      if (!active || !enabled) return;
      void Promise.all([loadSettings(), loadDiagnosticsSummary()]).then(() => {
        if (active && enabled) rebuild();
      });
    }, 4000);

    return () => {
      active = false;
      if (rebuildTimer) clearTimeout(rebuildTimer);
      clearInterval(interval);
      observer?.disconnect();
      unlistenFlag?.();
      unlistenError?.();
      unlistenPillState?.();
      unlistenPillStage?.();
      restoreAccessibilityDump();
      if (windowKind === 'main') restoreDumpWindowTitle();
    };
  });

  $effect(() => {
    if (!enabled) return;
    void appStore.currentPage;
    void appStore.settingsOpen;
    void appStore.settingsSection;
    void appStore.pillState;
    void appStore.isOnline;
    void extras;
    rebuild();
  });
</script>

{#if enabled}
  <section
    class="agent-ax-dump"
    data-verenu-ax-dump-region="true"
    aria-label={dumpTitle}
  >
    <h1>Verenu agent accessibility dump</h1>
    <p>
      Ruin accessibility is on. This text is in the OS accessibility tree for T3 Code SnapShots.
      Turn the flag off in Settings → Developer before using a screen reader.
    </p>
    <pre>{dumpText}</pre>
  </section>
{/if}

<style>
  .agent-ax-dump {
    position: fixed;
    left: 0;
    top: 0;
    width: 1px;
    height: 1px;
    margin: 0;
    overflow: hidden;
    clip-path: inset(50%);
    contain: strict;
    white-space: nowrap;
    pointer-events: none;
  }
</style>
