<script lang="ts">
  import { handleConsentError } from '../legal/consentStore.svelte';
  import { formatIpcError } from '../errors';
  import { onMount } from 'svelte';
  import { invoke, getVersion, listen, emit } from '../tauri';
  import { appStore } from '../stores';
  import { installAvailableUpdate } from '../updateActions';
  import { saveSetting } from '../settings';
  import { formatKeyLabel, defaultHotkey, isAndroid } from '../platform';
  import { getGreeting, HISTORY_PAGE_SIZE, type Entry, type Stats } from './home/helpers';
  import { applyHistoryPage, HISTORY_MAX_RETAINED } from './home/historyCache';
  import HomeHero from './home/HomeHero.svelte';
  import UpdateBanner from './home/UpdateBanner.svelte';
  import ProviderStatusBanner from './home/ProviderStatusBanner.svelte';
  import GlobalMessageBanner from './home/GlobalMessageBanner.svelte';
  import HistoryList from './home/HistoryList.svelte';
  import DictationLogDialog from './home/DictationLogDialog.svelte';
  import { parseDictationLog, type DictationLog } from './home/dictationLog';
  import StatsCard from './home/StatsCard.svelte';
  import { createLatestGate, createLatestRefresher } from './home/latestRequest';
  import { createLoadSequencer } from './home/loadSequencer';
  import { loadHotkey, hotkeyCodes } from '../hotkey.svelte';
  import { listenForSyncCompletion } from '../syncStore.svelte';
  import { desktopShortcut, type ShortcutStatus } from '../shortcutStatus.svelte';
  import ShortcutRecovery from '../components/ShortcutRecovery.svelte';
  import DictationReadiness from './home/DictationReadiness.svelte';
  import type { InstalledApp } from '../appMappings';

  let hotkey = defaultHotkey;
  let setupIncomplete = true;
  let dictationShortcut: ShortcutStatus | undefined;
  $: keyLabels = hotkey.filter(Boolean).map(formatKeyLabel);
  $: shortcutUnavailable = dictationShortcut?.active === null;

  function refreshShortcut(status = desktopShortcut('dictation')) {
    dictationShortcut = status;
    hotkey = status?.active === null ? ['Unavailable'] : hotkeyCodes();
  }

  let copiedId: number | null = null;
  let currentVersion = '';
  let failedEntry: { created_at: string } | null = null;
  let retrying = false;
  let failedTimer: ReturnType<typeof setTimeout> | null = null;
  let cancelledEntry: { created_at: string; kind: 'cancelled' | 'interrupted' } | null = null;
  let resumingCancelled = false;
  let cancelledTimer: ReturnType<typeof setTimeout> | null = null;

  const greeting = getGreeting();

  let recents: Entry[] = [];
  let stats: Stats = { total_words: 0, avg_wpm: 0, day_streak: 0 };
  let loading = true;
  let historyError = '';
  let loadingMore = false;
  let hasMoreHistory = false;
  let historyCursor: number | null = null;
  let historyOffset = 0;

  let search = '';
  let debouncedSearch = '';
  let appFilter: string | null = null;
  let apps: string[] = [];
  let installedApps: InstalledApp[] = [];
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  const statsGate = createLatestGate();
  const refreshStats = createLatestRefresher(
    () => invoke<Stats>('get_stats'),
    (next) => { stats = next; },
    statsGate,
  );
  const refreshApps = createLatestRefresher(
    () => invoke<string[] | null>('get_history_apps'),
    (list) => { apps = list ?? []; },
  );

  function handleSearchChange(value: string) {
    search = value;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      searchTimer = null;
      if (debouncedSearch !== value) {
        debouncedSearch = value;
        load(true, false);
      }
    }, 120);
  }

  function handleAppFilterChange(app: string | null) {
    if (appFilter === app) return;
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
      debouncedSearch = search;
    }
    appFilter = app;
    load(true, false);
  }

  function resetHistoryFilters(): boolean {
    const had = search !== '' || debouncedSearch !== '' || appFilter !== null;
    if (!had) return false;
    search = '';
    debouncedSearch = '';
    appFilter = null;
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
    }
    return true;
  }

  function clearHistoryFilters() {
    if (resetHistoryFilters()) load(true, false);
  }

  async function retryTranscription() {
    if (retrying) return;
    retrying = true;
    try {
      await invoke('retry_transcription');
    } catch (err) {
      console.error('Retry failed:', err);
      if (!handleConsentError(err, retryTranscription)) void emit('verenu:error', formatIpcError(err, 'Could not retry this dictation'));
    } finally {
      retrying = false;
    }
  }

  function clearCancelledEntry() {
    cancelledEntry = null;
    if (cancelledTimer) {
      clearTimeout(cancelledTimer);
      cancelledTimer = null;
    }
  }

  async function continueCancelled() {
    if (resumingCancelled) return;
    resumingCancelled = true;
    try {
      await invoke('resume_cancelled_capture');
      clearCancelledEntry();
    } catch (err) {
      console.error('Resume cancelled capture failed:', err);
      void emit('verenu:error', formatIpcError(err, 'Could not resume this recording'));
    } finally {
      resumingCancelled = false;
    }
  }

  async function dismissCancelled() {
    clearCancelledEntry();
    try {
      await invoke('dismiss_cancelled_capture');
    } catch {}
  }

  async function copyText(entry: Entry) {
    try {
      await navigator.clipboard.writeText(entry.clean_text);
      copiedId = entry.id;
      setTimeout(() => { copiedId = null; }, 1500);
    } catch (err) {
      void emit('verenu:error', formatIpcError(err, 'Could not copy this dictation to the clipboard'));
    }
  }

  let logEntry: Entry | null = null;
  let logData: DictationLog | null = null;
  let logLoading = false;
  let logError = '';
  let logSeq = 0;

  async function viewLogs(entry: Entry) {
    const seq = ++logSeq;
    logEntry = entry;
    logData = null;
    logError = '';
    logLoading = true;
    try {
      const raw = await invoke<unknown>('get_dictation_log', { id: entry.id });
      if (seq !== logSeq) return;
      logData = parseDictationLog(raw);
    } catch (err) {
      if (seq !== logSeq) return;
      logError = formatIpcError(err, 'Could not load the log for this dictation');
    } finally {
      if (seq === logSeq) logLoading = false;
    }
  }

  function closeLogs() {
    logSeq++;
    logEntry = null;
  }

  async function copyLogDetails(details: string) {
    try {
      await navigator.clipboard.writeText(details);
    } catch (err) {
      void emit('verenu:error', formatIpcError(err, 'Could not copy the log details'));
    }
  }

  async function deleteEntry(entry: Entry) {
    try {
      await invoke('delete_history_entry', { id: entry.id });
      recents = recents.filter((item) => item.id !== entry.id);
      historyOffset = Math.max(0, historyOffset - 1);
      // A get_recent issued before the delete may still be in flight and would restore the row.
      const interrupted = sequencer.interrupt();
      if (interrupted?.reset) void load(true);
      if (logEntry?.id === entry.id) closeLogs();
      void refreshStats();
      void refreshApps();
    } catch (err) {
      void emit('verenu:error', formatIpcError(err, 'Could not delete this dictation'));
    }
  }

  const sequencer = createLoadSequencer({
    fetchPage: async (reset) => {
      const nextOffset = reset ? 0 : historyOffset;
      const nextCursor = reset ? null : historyCursor;
      const page = await invoke<Entry[]>('get_recent', {
        limit: HISTORY_PAGE_SIZE,
        offset: nextOffset,
        beforeId: nextCursor ?? undefined,
        search: debouncedSearch.trim() || undefined,
        appName: appFilter ?? undefined,
      });
      return { page: page ?? [], nextOffset };
    },
    refreshStats,
    onSuccess: ({ page, nextOffset }, reset) => {
      const cached = applyHistoryPage(recents, page, nextOffset, reset);
      recents = cached.entries;
      historyOffset = cached.nextOffset;
      if (page.length > 0) historyCursor = page[page.length - 1].id;
      else if (reset) historyCursor = null;
      if (reset) historyError = '';
      hasMoreHistory = cached.hasMore;
    },
    onFailure: (err, reset) => {
      console.error('Home load failed:', err);
      if (reset) {
        historyError = formatIpcError(err, 'Could not load your dictation history');
        recents = [];
        historyOffset = 0;
        historyCursor = null;
        hasMoreHistory = false;
      }
    },
    onSettled: (reset) => {
      if (reset) loading = false;
    },
  });

  function load(reset = true, withStats = false) {
    return sequencer.load(reset, withStats);
  }

  async function loadOlder() {
    if (loadingMore || !hasMoreHistory) return;
    loadingMore = true;
    try {
      await load(false);
    } finally {
      loadingMore = false;
    }
  }

  async function handleInstall() {
    try {
      await installAvailableUpdate();
    } catch (e) {
      console.error('Install failed:', e);
      void emit('verenu:error', appStore.updateInstallError || formatIpcError(e, 'Could not install the update'));
    }
  }

  async function dismissUpdate() {
    const update = appStore.updateInfo;
    if (!update || appStore.updateInstalling || appStore.updateInstalled) return;
    try {
      await saveSetting('update_dismissed_version', update.version);
    } catch {}
    if (appStore.updateInstalling || appStore.updateInstalled || appStore.updateInfo !== update) return;
    appStore.updateInfo = null;
  }

  onMount(() => {
    invoke<InstalledApp[]>('get_installed_apps')
      .then(list => { installedApps = list ?? []; })
      .catch(() => { installedApps = []; });
    getVersion().then(v => currentVersion = v);
    void refreshApps();
    loadHotkey()
      .then(() => {
        refreshShortcut();
      })
      .catch(() => {});
    load(true, true);
    invoke<{ created_at: string; kind: string } | null>('get_cancelled_capture')
      .then((pending) => {
        if (!pending) return;
        cancelledEntry = {
          created_at: pending.created_at,
          kind: pending.kind === 'interrupted' ? 'interrupted' : 'cancelled',
        };
        if (cancelledTimer) clearTimeout(cancelledTimer);
        cancelledTimer = setTimeout(() => {
          cancelledEntry = null;
          cancelledTimer = null;
        }, 10 * 60 * 1000);
      })
      .catch(() => {});

    let mounted = true;
    const unlisteners: (() => void)[] = [];

    function trackListener(promise: Promise<() => void>) {
      promise
        .then((cleanup) => {
          if (!mounted) {
            cleanup();
            return;
          }
          unlisteners.push(cleanup);
        })
        .catch(() => {});
    }

    trackListener(listen<ShortcutStatus[]>('verenu:shortcuts-changed', (event) => {
      const dictation = event.payload.find((item) => item.id === 'dictation');
      if (dictation) {
        dictationShortcut = dictation;
        hotkey = dictation.active === null ? ['Unavailable'] : dictation.codes;
      }
    }));
    trackListener(listen<string[]>('verenu:hotkey-changed', (event) => {
      if (dictationShortcut?.active !== null) hotkey = event.payload;
    }));
    trackListener(listen('verenu:transcribed', () => {
      failedEntry = null;
      if (failedTimer) { clearTimeout(failedTimer); failedTimer = null; }
      load(true, true);
    }));

    trackListener(listen<Entry>('verenu:history-updated', (ev) => {
      failedEntry = null;
      if (failedTimer) { clearTimeout(failedTimer); failedTimer = null; }
      recents = [ev.payload, ...recents.filter((entry) => entry.id !== ev.payload.id)].slice(0, HISTORY_MAX_RETAINED);
      void refreshStats();
    }));

    trackListener(listen('verenu:history-pruned', () => load(true, true)));
    trackListener(listenForSyncCompletion(() => {
      load(true, true);
      void refreshApps();
    }));

    trackListener(listen<string>('verenu:pipeline-failed', (ev) => {
      failedEntry = { created_at: ev.payload };
      if (failedTimer) clearTimeout(failedTimer);
      failedTimer = setTimeout(() => {
        failedEntry = null;
        failedTimer = null;
      }, 10 * 60 * 1000);
    }));

    trackListener(listen<{ created_at: string; kind?: string }>('verenu:cancelled-capture', (ev) => {
      cancelledEntry = {
        created_at: ev.payload.created_at,
        kind: ev.payload.kind === 'interrupted' ? 'interrupted' : 'cancelled',
      };
      if (cancelledTimer) clearTimeout(cancelledTimer);
      cancelledTimer = setTimeout(() => {
        cancelledEntry = null;
        cancelledTimer = null;
      }, 10 * 60 * 1000);
    }));

    trackListener(listen('verenu:cancelled-capture-cleared', () => {
      clearCancelledEntry();
    }));

    return () => {
      mounted = false;
      while (unlisteners.length > 0) {
        unlisteners.pop()?.();
      }
      if (failedTimer) {
        clearTimeout(failedTimer);
        failedTimer = null;
      }
      if (cancelledTimer) {
        clearTimeout(cancelledTimer);
        cancelledTimer = null;
      }
      if (searchTimer) {
        clearTimeout(searchTimer);
        searchTimer = null;
      }
    };
  });

  $: if (appStore.settingsOpen) {
    if (resetHistoryFilters()) load(true, false);
  }
</script>

<div class="content-inner" class:desktop-home={!isAndroid}>
  <div class="home-grid">
    <div>
      <h1 class="page-h">Welcome back</h1>
      <p class="page-sub">{greeting}</p>

      <HomeHero {keyLabels} {setupIncomplete} {shortcutUnavailable} android={isAndroid} />

      <ShortcutRecovery />
      <DictationReadiness onchange={(incomplete) => { setupIncomplete = incomplete; }} />

      {#if appStore.globalMessage}
        <GlobalMessageBanner message={appStore.globalMessage.message} />
      {/if}

      {#if appStore.recoveryStorageWarning}
        <GlobalMessageBanner message="Your drive is full. Some features won't work properly." />
      {/if}

      {#if appStore.providerStatusAlerts.length > 0}
        <ProviderStatusBanner alerts={appStore.providerStatusAlerts} />
      {:else if appStore.updateInfo}
        <UpdateBanner
          {currentVersion}
          updateInfo={appStore.updateInfo}
          installing={appStore.updateInstalling}
          onInstall={handleInstall}
          onDismiss={dismissUpdate}
        />
      {/if}

      <HistoryList
        {recents}
        {failedEntry}
        {cancelledEntry}
        {loading}
        {historyError}
        onRetryHistory={() => load()}
        {hasMoreHistory}
        {loadingMore}
        {retrying}
        {resumingCancelled}
        {copiedId}
        {keyLabels}
        {setupIncomplete}
        {shortcutUnavailable}
        android={isAndroid}
        {search}
        {apps}
        {installedApps}
        {appFilter}
        onSearchChange={handleSearchChange}
        onAppFilterChange={handleAppFilterChange}
        onClearFilters={clearHistoryFilters}
        onRetry={retryTranscription}
        onContinueCancelled={continueCancelled}
        onDismissCancelled={dismissCancelled}
        onLoadOlder={loadOlder}
        onCopy={copyText}
        onViewLogs={viewLogs}
        onDelete={deleteEntry}
      />
    </div>

    {#if !isAndroid}
      <div class="stat-stack native-chrome-corner--below">
        <StatsCard {stats} />
      </div>
    {/if}
  </div>
</div>

{#if logEntry}
  <DictationLogDialog
    entry={logEntry}
    log={logData}
    loading={logLoading}
    error={logError}
    devMode={appStore.devModeEnabled}
    onclose={closeLogs}
    oncopy={copyLogDetails}
  />
{/if}

<style>
  .content-inner {
    width: min(100%, var(--page-max));
    margin-inline: auto;
    padding: var(--page-pad-y) var(--page-pad-x) 36px;
    min-width: 0;
  }

  .home-grid {
    display: grid;
    grid-template-columns: minmax(540px, 1fr) minmax(220px, 280px);
    gap: clamp(18px, 3vw, 32px);
    align-items: start;
    min-width: 0;
  }

  .home-grid > div {
    min-width: 0;
  }

  .page-h {
    font-family: var(--sans);
    font-size: 23px;
    font-weight: 600;
    letter-spacing: -0.025em;
    margin: 0 0 4px;
    line-height: 1.1;
    color: var(--ink);
  }

  .page-sub { color: var(--ink-mute); font-size: 12.5px; margin: 0 0 22px; }

  .stat-stack {
    display: flex;
    flex-direction: column;
    gap: 22px;
    position: sticky;
    top: calc(var(--page-pad-y) + var(--native-chrome-below-clearance, 0px));
  }

  @media (max-width: 1060px) {
    .home-grid {
      grid-template-columns: 1fr;
      min-width: 640px;
    }

    .desktop-home .stat-stack { display: none; }
  }

  :global(.app[data-android='true']) .home-grid { min-width: 0; }

  @media (max-width: 720px) {
    .content-inner {
      flex: 1;
      display: flex;
      flex-direction: column;
      padding-bottom: 20px;
    }

    .home-grid {
      flex: 1;
      align-items: stretch;
      grid-template-rows: 1fr auto;
      gap: 16px;
      min-width: 0;
    }

    .home-grid > div:first-child {
      display: flex;
      flex-direction: column;
      min-height: 0;
    }

    .home-grid :global(.empty-state) {
      flex: 1;
      justify-content: center;
      padding-block: 24px;
    }

    .page-sub { margin-bottom: 16px; }
  }
</style>
