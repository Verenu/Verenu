<script lang="ts">
  import { formatIpcError } from '../../errors';
  import { invoke, isTauriRuntime } from '../../tauri';
  import { isAndroid } from '../../platform';
  import { fly, fade } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import Toggle from '../Toggle.svelte';
  import Dropdown from '../Dropdown.svelte';
  import { loadSettingsSnapshot, saveSetting, type HistoryRetention } from '../../settings';
  import { setServiceChecksEnabled } from '../../serviceStatus';
  import { modalFocusTrap } from '../../modalFocus';
  import { animateWidth, modalBackdrop, modalCard, MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';

  const historyOptions = ['7 days', '30 days', '90 days', 'Forever'];
  const HISTORY_MENU_ID = 'history-retention-menu';
  let agentAccessMessage = $state('');
  async function revokeAgentAccess() {
    try {
      await invoke('agent_revoke_all');
      agentAccessMessage = 'Agent access revoked.';
    } catch {
      agentAccessMessage = 'Could not revoke agent access.';
    }
  }
  let confirmRetention = $state<{ value: string; count: number } | null>(null);
  type CleanupCacheStatus = {
    entry_count: number;
    payload_bytes: number;
    session: { hits: number; misses: number; provider_calls: number; provider_ms: number };
  } | null;

  let historyRetention = $state('30 days');
  let historyDropdownOpen = $state(false);
  let historyMenuOpensUp = $state(false);
  let serviceChecksEnabled = $state(true);
  let analyticsEnabled = $state(true);
  let cleanupCacheEntries = $state(0);
  let cleanupCacheStatus = $state<CleanupCacheStatus>(null);
  let cleanupCacheEnabled = $state(true);
  let savingCleanupCache = $state(false);
  let cleanupCacheError = $state('');
  let cleanupCacheMessage = $state('');
  let clearingCleanupCache = $state(false);
  async function loadSettings() {
    try {
      const settings = await loadSettingsSnapshot();
      if (settings.history_retention) historyRetention = settings.history_retention;
      serviceChecksEnabled = settings.verenu_service_checks_enabled ?? true;
      analyticsEnabled = settings.analytics_enabled ?? true;
      setServiceChecksEnabled(serviceChecksEnabled);
      cleanupCacheEnabled = settings.cleanup_cache_enabled ?? true;
    } catch (err) {
      console.error('PrivacySection load failed:', err);
    }
  }

  async function saveHistoryRetention(value: string) {
    historyRetention = value;
    historyDropdownOpen = false;
    try {
      await saveSetting('history_retention', value as HistoryRetention);
    } catch (err) {
      console.error('saveHistoryRetention failed:', err);
    }
  }

  async function requestHistoryRetention(value: string) {
    historyDropdownOpen = false;
    if (value === historyRetention) return;
    try {
      const count = await invoke<number>('count_old_transcriptions', { retention: value });
      if (count > 0) {
        confirmRetention = { value, count };
      } else {
        await saveHistoryRetention(value);
      }
    } catch (err) {
      console.error('count_old_transcriptions failed:', err);
    }
  }

  async function confirmHistoryRetention() {
    if (!confirmRetention) return;
    const { value } = confirmRetention;
    confirmRetention = null;
    await saveHistoryRetention(value);
  }

  function handleRetentionModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmRetention) confirmRetention = null;
  }

  let serviceChecksError = $state(false);

  async function handleServiceChecks(value: boolean) {
    const previous = serviceChecksEnabled;
    serviceChecksEnabled = value;
    if (!value) setServiceChecksEnabled(false);
    try {
      await saveSetting('verenu_service_checks_enabled', value);
      setServiceChecksEnabled(value);
    } catch (err) {
      serviceChecksEnabled = previous;
      serviceChecksError = true;
      setServiceChecksEnabled(previous);
      console.error('save verenu_service_checks_enabled failed:', err);
    }
  }

  let analyticsError = $state(false);

  async function handleAnalytics(value: boolean) {
    const previous = analyticsEnabled;
    analyticsEnabled = value;
    try {
      await saveSetting('analytics_enabled', value);
    } catch (err) {
      analyticsEnabled = previous;
      analyticsError = true;
      console.error('save analytics_enabled failed:', err);
    }
  }

  async function clearCleanupCache() {
    if (clearingCleanupCache || savingCleanupCache) return;
    clearingCleanupCache = true;
    cleanupCacheError = '';
    cleanupCacheMessage = '';
    try {
      await invoke<number>('clear_cleanup_cache');
      await refreshCleanupCache();
      cleanupCacheMessage = 'Cache cleared.';
    } catch (err) {
      cleanupCacheError = `Could not clear the cache. ${formatIpcError(err)}`;
    } finally {
      clearingCleanupCache = false;
    }
  }

  async function refreshCleanupCache() {
    cleanupCacheStatus = await invoke<CleanupCacheStatus>('get_cleanup_cache_status');
    cleanupCacheEntries = cleanupCacheStatus?.entry_count ?? 0;
  }

  async function handleCleanupCache(value: boolean) {
    if (savingCleanupCache || clearingCleanupCache) return;
    savingCleanupCache = true;
    cleanupCacheError = '';
    cleanupCacheMessage = '';
    try {
      await saveSetting('cleanup_cache_enabled', value);
      cleanupCacheEnabled = value;
      await refreshCleanupCache();
      cleanupCacheMessage = value ? 'Caching enabled.' : 'Caching disabled and stored results cleared.';
    } catch (err) {
      cleanupCacheEnabled = await invoke<boolean | null>('get_setting', { key: 'cleanup_cache_enabled' })
        .then((enabled) => enabled ?? true).catch(() => cleanupCacheEnabled);
      cleanupCacheError = `Could not update the cache. ${formatIpcError(err)}`;
    } finally {
      savingCleanupCache = false;
    }
  }

  function cacheSize(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
  }

  loadSettings();
  refreshCleanupCache().catch((err) => {
    cleanupCacheError = `Cache status unavailable. ${formatIpcError(err)}`;
  });

  type ImportSummary = {
    settings_applied: number;
    settings_skipped: number;
    dictionary_inserted: number;
    dictionary_skipped: number;
    dictionary_already_existed: number;
    contexts_inserted?: number;
    contexts_already_existed?: number;
    context_conflicts?: number;
    dictionary_corrections_inserted?: number;
    dictionary_corrections_skipped?: number;
    snippets_inserted: number;
    snippets_skipped: number;
    snippets_already_existed: number;
    app_targets_kept?: number;
    app_targets_matched?: number;
    app_targets_dropped?: number;
    sub_apps_imported?: number;
    sub_apps_dropped?: number;
    history_inserted?: number;
    history_existing?: number;
    history_conflicts?: number;
    history_expired?: number;
    history_context_unmatched?: number;
    activity_conflicts?: number;
    website_targets_skipped?: number;
  };

  let exporting = $state(false);
  let exportMsg = $state('');
  let exportMsgKind = $state<'ok' | 'err' | ''>('');
  let importing = $state(false);
  let importMsg = $state('');
  let importMsgKind = $state<'ok' | 'err' | ''>('');
  let fileInput: HTMLInputElement | null = $state(null);
  let historyRetentionButton: HTMLButtonElement | null = $state(null);
  let historyRetentionMenu: HTMLDivElement | null = $state(null);

  $effect(() => {
    if (!historyDropdownOpen) {
      historyMenuOpensUp = false;
      return;
    }

    const frame = requestAnimationFrame(() => {
      const trigger = historyRetentionButton;
      const menu = historyRetentionMenu;
      if (!trigger || !menu) return;

      const triggerRect = trigger.getBoundingClientRect();
      const menuHeight = menu.getBoundingClientRect().height;
      let clipTop = 0;
      let clipBottom = window.innerHeight;
      for (let parent = trigger.parentElement; parent; parent = parent.parentElement) {
        const overflowY = getComputedStyle(parent).overflowY;
        if (overflowY !== 'auto' && overflowY !== 'scroll' && overflowY !== 'hidden' && overflowY !== 'clip') continue;
        const bounds = parent.getBoundingClientRect();
        clipTop = Math.max(clipTop, bounds.top + parent.clientTop);
        clipBottom = Math.min(clipBottom, bounds.top + parent.clientTop + parent.clientHeight);
      }

      const spaceAbove = triggerRect.top - clipTop - 4;
      const spaceBelow = clipBottom - triggerRect.bottom - 4;
      historyMenuOpensUp = menuHeight > spaceBelow && spaceAbove > spaceBelow;
    });
    return () => cancelAnimationFrame(frame);
  });
  let retentionCancelButton: HTMLButtonElement | null = $state(null);

  async function handleExport() {
    exporting = true;
    exportMsg = '';
    exportMsgKind = '';
    try {
      const path = await invoke<string>('export_data');
      exportMsg = `Saved to ${path}`;
      exportMsgKind = 'ok';
    } catch (err) {
      exportMsg = formatIpcError(err, 'Could not export your backup');
      exportMsgKind = 'err';
    } finally {
      exporting = false;
    }
  }

  async function handleFileSelected(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    importing = true;
    importMsg = '';
    importMsgKind = '';
    try {
      const json = await file.text();
      const s = await invoke<ImportSummary>('import_data', { json });
      const dictParts = [
        s.dictionary_inserted > 0 ? `${s.dictionary_inserted} added` : '',
        s.dictionary_already_existed > 0 ? `${s.dictionary_already_existed} already on device` : '',
        s.dictionary_skipped > 0 ? `${s.dictionary_skipped} skipped` : '',
      ].filter(Boolean).join(', ') || 'none';
      const snipParts = [
        s.snippets_inserted > 0 ? `${s.snippets_inserted} added` : '',
        s.snippets_already_existed > 0 ? `${s.snippets_already_existed} already on device` : '',
        s.snippets_skipped > 0 ? `${s.snippets_skipped} skipped` : '',
      ].filter(Boolean).join(', ') || 'none';
      const contextTotal = (s.contexts_inserted ?? 0) + (s.contexts_already_existed ?? 0);
      const correctionParts = [
        s.dictionary_corrections_inserted && s.dictionary_corrections_inserted > 0
          ? `${s.dictionary_corrections_inserted} added`
          : '',
        s.dictionary_corrections_skipped && s.dictionary_corrections_skipped > 0
          ? `${s.dictionary_corrections_skipped} skipped`
          : '',
      ].filter(Boolean).join(', ');
      const appParts = [
        (s.app_targets_kept ?? 0) + (s.app_targets_matched ?? 0) > 0
          ? `${(s.app_targets_kept ?? 0) + (s.app_targets_matched ?? 0)} apps linked`
          : '',
        s.app_targets_matched ? `${s.app_targets_matched} matched to apps on this device` : '',
        s.app_targets_dropped ? `${s.app_targets_dropped} unmatched or ambiguous, left out` : '',
        s.sub_apps_imported ? `${s.sub_apps_imported} sub-apps` : '',
        s.sub_apps_dropped ? `${s.sub_apps_dropped} sub-apps left out` : '',
        s.website_targets_skipped ? `${s.website_targets_skipped} websites already assigned elsewhere` : '',
        s.context_conflicts ? `${s.context_conflicts} existing Context settings kept` : '',
      ].filter(Boolean).join(', ');
      const contextParts = contextTotal > 0 || appParts
        ? ` Contexts: ${contextTotal}${appParts ? ` (${appParts})` : ''}.` : '';
      const correctionSummary = correctionParts ? ` Corrections: ${correctionParts}.` : '';
      const historySummary = s.history_inserted || s.history_existing || s.history_conflicts || s.history_expired
        ? ` History: ${s.history_inserted ?? 0} added, ${s.history_existing ?? 0} already on device${s.history_expired ? `, ${s.history_expired} outside your retention period` : ''}${s.history_conflicts ? `, ${s.history_conflicts} with unresolved conflicts` : ''}${s.history_context_unmatched ? `, ${s.history_context_unmatched} without a matching Context` : ''}.`
        : '';
      const activitySummary = s.activity_conflicts
        ? ` ${s.activity_conflicts} activity summaries overlapped or could not be matched; some older totals may be incomplete.`
        : '';
      importMsg = `Applied ${s.settings_applied} settings.${contextParts} Dictionary: ${dictParts}.${correctionSummary} Snippets: ${snipParts}.${historySummary}${activitySummary}`;
      importMsgKind = 'ok';
    } catch (err: unknown) {
      importMsg = formatIpcError(err, 'Could not import this backup');
      importMsgKind = 'err';
    } finally {
      importing = false;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<svelte:window onkeydown={handleRetentionModalKeydown} />

<h2 class="settings-h">Privacy</h2>

{#if !isAndroid && isTauriRuntime()}
  <h3 class="settings-subhead first">Agent access</h3>
  <div class="setting-row" data-setting-target="privacy-agent-access">
    <div>
      <div class="label">Connected agents</div>
      <div class="desc">Approved agents can read and change Verenu data for up to one hour. Revoke all access at any time. {agentAccessMessage}</div>
    </div>
    <button class="btn-ghost" onclick={revokeAgentAccess}>Revoke access</button>
  </div>
{/if}

<h3 class="settings-subhead">Network</h3>
<div class="setting-row" data-setting-target="privacy-service-checks">
  <div>
    <div class="label">Allow Verenu service checks</div>
    <div class="desc">Checks provider status, service health, and global messages in the background. Turn this off to stop background requests to api.verenu.com. Dictation requests to your selected AI provider are unaffected.</div>
  </div>
  <Toggle checked={serviceChecksEnabled} onchange={handleServiceChecks} label="Allow Verenu service checks" bind:error={serviceChecksError} />
</div>

<div class="setting-row" data-setting-target="privacy-analytics">
  <div>
        <div class="label">Share pseudonymous product analytics</div>
    <div class="desc">Helps improve Verenu by measuring feature usage, settings adoption, pipeline reliability, retries, and fallbacks. Dictated content, prompts, clipboard contents, app names, and raw errors are never included.</div>
  </div>
        <Toggle checked={analyticsEnabled} onchange={handleAnalytics} label="Share pseudonymous product analytics" bind:error={analyticsError} />
</div>

<h3 class="settings-subhead">Stored on this device</h3>
<div class="setting-row" data-setting-target="privacy-history">
  <div><div class="label">Transcription history</div><div class="desc">How long to keep past dictations</div></div>
  <Dropdown bind:open={historyDropdownOpen} closeSelector=".history-dropdown">
    <div class="ui-dropdown history-dropdown">
      <button
        bind:this={historyRetentionButton}
        class="ui-dropdown-trigger ui-dropdown-trigger--compact mic-btn"
        type="button"
        use:animateWidth={{ text: historyRetention }}
        onclick={() => (historyDropdownOpen = !historyDropdownOpen)}
        aria-haspopup="listbox"
        aria-expanded={historyDropdownOpen}
        aria-controls={HISTORY_MENU_ID}
        aria-label="Transcription history retention"
      >
        <span>{historyRetention}</span>
        <svg class:open={historyDropdownOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="m6 9 6 6 6-6"/>
        </svg>
      </button>
      {#if historyDropdownOpen}
        <div
          bind:this={historyRetentionMenu}
          id={HISTORY_MENU_ID}
          class="ui-dropdown-menu ui-dropdown-menu--padded mic-menu scroll-styled scroll-thumb-elev"
          class:open-up={historyMenuOpensUp}
          role="listbox"
          tabindex="-1"
          aria-label="History retention options"
          in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
          out:fade={{ duration: motionMs(MOTION_MS.fast) }}
        >
          {#each historyOptions as opt}
          <button
            class="ui-dropdown-option mic-item"
            class:active={historyRetention === opt}
            onclick={() => requestHistoryRetention(opt)}
            role="option"
            type="button"
            aria-selected={historyRetention === opt}
          >
            {opt}
          </button>
          {/each}
        </div>
      {/if}
    </div>
  </Dropdown>
</div>
<div class="setting-row" data-setting-target="privacy-cache">
  <div>
    <div class="label">Cleanup cache</div>
    <div class="desc">
      Save cleaned text on this device to speed up repeated dictation.
      Results expire after 2 idle days or 7 days total. Turning this off clears stored results.
    </div>
    <div class="desc cache-status" data-setting-target="privacy-cache-storage">
      {#if cleanupCacheStatus}
        {cleanupCacheEntries} cached result{cleanupCacheEntries === 1 ? '' : 's'} · {cacheSize(cleanupCacheStatus.payload_bytes)} of text.
        {#if cleanupCacheStatus.session.hits + cleanupCacheStatus.session.misses > 0}
          Reused {cleanupCacheStatus.session.hits} of {cleanupCacheStatus.session.hits + cleanupCacheStatus.session.misses} eligible requests this session.
        {/if}
      {:else}
        Cache status unavailable.
      {/if}
      {#if cleanupCacheError}<span class="cache-note" role="alert">{cleanupCacheError}</span>{/if}
      {#if cleanupCacheMessage}<span class="cache-note" role="status">{cleanupCacheMessage}</span>{/if}
    </div>
  </div>
  <div class="row-actions">
    <button
      class="btn-ghost btn-compact"
      onclick={clearCleanupCache}
      disabled={clearingCleanupCache || savingCleanupCache}
    >
      {clearingCleanupCache ? 'Clearing…' : 'Clear Cache'}
    </button>
    <Toggle checked={cleanupCacheEnabled} onchange={handleCleanupCache} label="Cleanup cache" disabled={savingCleanupCache || clearingCleanupCache} />
  </div>
</div>

<h3 class="settings-subhead">Backup</h3>
<div class="setting-row" data-setting-target="privacy-backup">
  <div>
    <div class="label">Back up and restore</div>
    <div class="desc">Export saves settings, Contexts, Insights and transcription history to a file. The file contains private text; API keys are excluded. Import merges a backup into this device, and History retention can remove old imported entries.</div>
    {#each [{ msg: exportMsg, kind: exportMsgKind }, { msg: importMsg, kind: importMsgKind }] as status}
      {#if status.msg}
        <div
          class="desc data-status"
          class:data-ok={status.kind === 'ok'}
          class:data-err={status.kind === 'err'}
        >{status.msg}</div>
      {/if}
    {/each}
  </div>
  <div class="row-actions">
    <button class="btn-ghost" onclick={() => fileInput?.click()} disabled={importing || exporting}>
      {importing ? 'Importing…' : 'Import'}
    </button>
    <button class="btn-ghost" onclick={handleExport} disabled={exporting || importing}>
      {exporting ? 'Exporting…' : 'Export'}
    </button>
  </div>
</div>
<input
  bind:this={fileInput}
  type="file"
  accept=".json"
  style="display:none"
  onchange={handleFileSelected}
/>

{#if confirmRetention}
  <button class="modal-backdrop" aria-label="Close dialog" onclick={() => (confirmRetention = null)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="modal-card"
    use:modalFocusTrap={{
      active: !!confirmRetention,
      initialFocus: () => retentionCancelButton,
      restoreFocus: () => historyRetentionButton,
    }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="retention-confirm-title"
    tabindex="-1"
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="modal-header">
      <h2 id="retention-confirm-title" class="modal-title">Delete older history?</h2>
    </div>
    <div class="modal-body">
      <p class="confirm-copy">
        Looks like you have {confirmRetention.count} transcription{confirmRetention.count === 1 ? '' : 's'} before this point. Changing this will delete that data. Are you sure?
      </p>
    </div>
    <div class="modal-footer">
      <div class="footer-actions">
        <button bind:this={retentionCancelButton} class="btn-ghost" onclick={() => (confirmRetention = null)}>Cancel</button>
        <button class="btn-danger btn-compact" onclick={confirmHistoryRetention}>Delete history</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .data-ok { color: var(--success); }
  .data-err { color: var(--accent); }
  .data-status {
    animation: data-drop 260ms cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  @keyframes data-drop {
    from { opacity: 0; transform: translateY(-6px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .history-dropdown { position: relative; flex-shrink: 0; }
  .mic-btn { max-width: 180px; }
  .mic-btn span { overflow: hidden; white-space: nowrap; }
  .mic-menu { min-width: 200px; max-width: 280px; max-height: 200px; z-index: 10; }
  .mic-menu.open-up { top: auto; bottom: calc(100% + 4px); }
  @media (max-width: 600px) {
    .history-dropdown .mic-menu { left: 0; right: auto; }
  }
  .row-actions { display: flex; align-items: center; gap: 12px; flex-shrink: 0; }
  .cache-status { font-variant-numeric: tabular-nums; }
  .cache-note { display: block; margin-top: 2px; }
  .modal-backdrop {
    position: fixed;
    inset: 0;
    border: 0;
    padding: 0;
    appearance: none;
    background: var(--overlay);
    z-index: 50;
    outline: none;
  }
  .modal-card {
    position: fixed;
    top: 50%;
    left: 50%;
    translate: -50% -50%;
    z-index: 51;
    isolation: isolate;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    width: min(420px, calc(100vw - 40px));
    box-shadow: var(--shadow-elev);
    overflow: hidden;
  }
  .modal-header {
    padding: 20px 20px 0;
  }
  .modal-title {
    font-family: var(--sans);
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--ink);
    margin: 0;
  }
  .modal-body { padding: 10px 20px 18px; }
  .confirm-copy {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--ink-soft);
  }
  .modal-footer {
    padding: 0 20px 20px;
  }
  .footer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
