<script lang="ts">
  import { onMount } from 'svelte';
  import { crossfade, fade, fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import Toggle from '../Toggle.svelte';
  import Dropdown from '../Dropdown.svelte';
  import { appStore } from '../../stores';
  import { checkStatus } from '../../serviceStatus';
  import { ensureNotificationPermission } from '../../notifications';
  import { invoke, listen } from '../../tauri';
  import { saveSetting, type ProviderId } from '../../settings';
  import { MOTION_MS, MOTION_PX, modalBackdrop, modalCard, motionMs, motionPx } from '../../motion';
  import { modalFocusTrap } from '../../modalFocus';
  import { frontendIpcActivity, filterLogs, formatBytes, formatDuration, formatRate, spanWidth, unknown, type DiagnosticsSnapshot, type StructuredLogEntry, type PipelineTrace } from '../../diagnostics';

  const [send, receive] = crossfade({ duration: motionMs(MOTION_MS.fast), easing: expoOut });

  type View = 'overview' | 'pipeline' | 'failures' | 'logs' | 'runtime' | 'activity' | 'storage' | 'faults' | 'settings';
  const views: { id: View; label: string }[] = [
    { id: 'overview', label: 'Overview' }, { id: 'pipeline', label: 'Pipeline' },
    { id: 'failures', label: 'Failures' }, { id: 'logs', label: 'Logs' },
    { id: 'runtime', label: 'Runtime' }, { id: 'activity', label: 'Activity' },
    { id: 'storage', label: 'Storage / Internals' }, { id: 'faults', label: 'Fault Injection' },
    { id: 'settings', label: 'Developer Settings' },
  ];

  const LOG_LEVELS = [
    { value: 'all', label: 'All levels' }, { value: 'error', label: 'Error' },
    { value: 'warn', label: 'Warn' }, { value: 'info', label: 'Info' }, { value: 'debug', label: 'Debug' },
  ];
  const PROVIDER_OPTIONS: { value: ProviderId; label: string }[] = [
    { value: 'groq', label: 'Groq' }, { value: 'openai', label: 'OpenAI' },
    { value: 'google', label: 'Gemini' }, { value: 'assemblyai', label: 'AssemblyAI' },
  ];

  const emptySnapshot: DiagnosticsSnapshot = {
    generated_at_ms: 0, profiler_enabled: false, profiling_recording: false,
    current_resource: null, resource_samples: [], latest_failures: [], failure_groups: [],
    active_pipelines: [], recent_pipelines: [], logs: [], operations: [], runtime: {},
    health: { initialized: false, profiler_enabled: false, retained_log_count: 0, retained_failure_count: 0,
      retained_trace_count: 0, retained_operation_count: 0, retained_resource_sample_count: 0,
      active_trace_count: 0, active_span_count: 0, total_logs_recorded: 0, total_failures_recorded: 0,
      total_traces_started: 0, total_traces_completed: 0, total_operations_recorded: 0,
      dropped_logs: 0, dropped_failures: 0, dropped_traces: 0, dropped_spans: 0,
      dropped_resource_samples: 0, collector_samples: 0, collector_duration_us_total: 0 },
  };

  let view = $state<View>('overview');
  let snapshot = $state<DiagnosticsSnapshot>(emptySnapshot);
  let frontendMetrics = $state(frontendIpcActivity.snapshot());
  let monitoring = $state(false);
  let recording = $state(false);
  let paused = $state(false);
  let autoScroll = $state(true);
  let logQuery = $state('');
  let logLevel = $state('all');
  let logSubsystem = $state('all');
  let logTrace = $state('');
  let logLevelOpen = $state(false);
  let logSubsystemOpen = $state(false);
  let faultProviderOpen = $state(false);
  let privacyModalOpen = $state(false);
  let privacyModalCloseBtn: HTMLButtonElement | null = $state(null);
  let selectedTrace = $state<string | null>(null);
  let exportMessage = $state('');
  let faultMessage = $state('');
  let providerStatusRaw = $state('');
  let verboseEnabled = $state(false);
  let ruinAccessibility = $state(false);
  let devModeOnStartup = $state(false);
  let forceSetupOnLaunch = $state(false);
  let storageFullSimulation = $state(false);
  let syncEnabled = $state(false);
  let provider = $state<ProviderId>('groq');
  let refreshRunning = false;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let unlistenDiagnostics: (() => void) | null = null;
  let onVisibility: (() => void) | null = null;

  function latestTrace(): PipelineTrace | null {
    return snapshot.active_pipelines[0] ?? snapshot.recent_pipelines[snapshot.recent_pipelines.length - 1] ?? null;
  }

  function shortTime(ms: number | null | undefined): string {
    return ms ? new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—';
  }

  function currentResource() { return snapshot.current_resource ?? null; }
  function operationRows() { return [...snapshot.operations, ...frontendMetrics.map((m) => ({
    operation: `frontend.invoke:${m.command}`, calls: m.calls, success_count: m.calls - m.failures,
    failure_count: m.failures, cancelled_count: 0, skipped_count: 0, total_duration_ms: m.total_duration_ms,
    average_duration_ms: m.average_duration_ms, recent_duration_ms: m.samples[m.samples.length - 1] ?? null,
    p95_duration_ms: m.p95_duration_ms, max_duration_ms: m.max_duration_ms, calls_per_minute: 0,
    currently_running: m.currently_running, last_called_at_ms: m.last_seen,
  }))].sort((a, b) => b.total_duration_ms - a.total_duration_ms); }

  function filteredLogs(): StructuredLogEntry[] {
    return filterLogs(snapshot.logs, logQuery, logLevel, logSubsystem, logTrace);
  }

  function subsystems(): string[] { return [...new Set(snapshot.logs.map((entry) => entry.subsystem))].sort(); }
  function subsystemLabel(value: string): string {
    if (value === 'all') return 'All subsystems';
    return value.replace(/^verenu::/, '').split('::').map((part) => part.replace(/_/g, ' ')).join(' / ');
  }
  function subsystemOptions() { return [{ value: 'all', label: 'All subsystems' }, ...subsystems().map((s) => ({ value: s, label: subsystemLabel(s) }))]; }

  // Runtime stages mirror the real backend span order emitted by the pipeline
  // (see start_span calls in src-tauri/src/pipeline/mod.rs: capture ->
  // context -> vad -> transcription -> cleanup -> injection). There is no
  // "reconciliation", "persistence", or "auto-learn" span — a fixed fake list
  // just froze in the wrong order and lit every downstream node at once the
  // moment recording stopped. Microphone is the one node with no span; it
  // tracks the live recording atomics instead (see stage() below).
  // audio.active and active_pipelines come from the polled backend snapshot,
  // not the frontend pill's local `appStore.pillState` — the dictation pill
  // runs in its own window with its own local state that never syncs back to
  // this one, so pillState reads 'idle' here even mid-dictation. This view
  // must never create its own audio processing or retained buffers alongside
  // the real recording session.
  const RUNTIME_STAGES: { id: string | null; label: string }[] = [
    { id: null, label: 'Microphone' },
    { id: 'capture', label: 'Capture' },
    { id: 'context', label: 'Context' },
    { id: 'vad', label: 'VAD Gate' },
    { id: 'transcription', label: 'Transcription' },
    { id: 'cleanup', label: 'Cleanup' },
    { id: 'injection', label: 'Injection' },
  ];
  function audioActive(): boolean { return snapshot.runtime.audio?.active === true; }
  function pipelineActive(): boolean { return snapshot.active_pipelines.length > 0; }
  function currentTrace(): PipelineTrace | null { return snapshot.active_pipelines[0] ?? null; }
  function spanForStage(stageId: string) {
    const trace = currentTrace();
    if (!trace) return undefined;
    // Dual transcription starts two parallel spans under the same stage —
    // the most recently started one is the representative one to show.
    return [...trace.spans].reverse().find((s) => s.stage === stageId);
  }
  function pillLabel(): string {
    if (audioActive()) return 'recording';
    if (pipelineActive()) return 'processing';
    return appStore.pillState || 'idle';
  }
  function stageState(stageId: string | null): 'idle' | 'active' | 'done' | 'error' {
    if (stageId === null) {
      if (audioActive()) return 'active';
      return pipelineActive() ? 'done' : 'idle';
    }
    const span = spanForStage(stageId);
    if (!span) return 'idle';
    if (span.outcome === 'running') return 'active';
    if (span.outcome === 'failure') return 'error';
    return 'done';
  }
  function stageDetail(stageId: string | null): string {
    if (stageId === null) return audioActive() ? 'recording' : (pipelineActive() ? 'done' : 'idle');
    const span = spanForStage(stageId);
    if (!span) return 'pending';
    if (span.outcome === 'running') return 'running…';
    if (span.outcome === 'failure') return 'failed';
    return formatDuration(span.duration_ms);
  }

  // Frontend invokes and backend pipeline operations come from different
  // sources (frontendIpcActivity vs the Rust operation tracker) and are
  // interesting to scan separately rather than interleaved by duration.
  function groupedOperations() {
    const rows = operationRows();
    const frontend = rows.filter((r) => r.operation.startsWith('frontend.invoke:'));
    const backend = rows.filter((r) => !r.operation.startsWith('frontend.invoke:'));
    return [
      { label: 'Backend operations', rows: backend },
      { label: 'Frontend invokes', rows: frontend },
    ].filter((group) => group.rows.length > 0);
  }

  async function refresh() {
    if (refreshRunning || (typeof document !== 'undefined' && document.visibilityState !== 'visible')) return;
    refreshRunning = true;
    try {
      snapshot = await invoke<DiagnosticsSnapshot>('get_diagnostics_snapshot');
      frontendMetrics = frontendIpcActivity.snapshot();
    } catch (error) {
      faultMessage = `Snapshot unavailable: ${String(error).slice(0, 140)}`;
    } finally { refreshRunning = false; }
  }

  async function toggleRecording() {
    recording = !recording;
    await invoke('set_diagnostics_profiling', { enabled: recording }).catch(() => { recording = !recording; });
    void refresh();
  }

  async function download(format: 'json' | 'text' = 'json') {
    try {
      const path = await invoke<string>('download_diagnostics_bundle', { format });
      exportMessage = `Saved ${path}`;
    } catch (error) { exportMessage = `Export failed: ${String(error).slice(0, 140)}`; }
  }

  async function clearDiagnostics() {
    if (typeof window !== 'undefined' && !window.confirm('Clear retained diagnostics?')) return;
    await invoke('clear_diagnostics').then(() => { exportMessage = 'Retained diagnostics cleared.'; void refresh(); }).catch((error) => { exportMessage = `Clear failed: ${String(error).slice(0, 120)}`; });
  }

  async function copyLogs() {
    try {
      await navigator.clipboard.writeText(filteredLogs().map((entry) => JSON.stringify(entry)).join('\n'));
      exportMessage = 'Visible structured logs copied.';
    } catch { exportMessage = 'Clipboard unavailable.'; }
  }

  async function copyAllLogs() {
    try {
      await navigator.clipboard.writeText(snapshot.logs.map((entry) => JSON.stringify(entry)).join('\n'));
      exportMessage = `All ${snapshot.logs.length} retained logs copied.`;
    } catch { exportMessage = 'Clipboard unavailable.'; }
  }

  async function copyTrace(trace: PipelineTrace | undefined) {
    if (!trace) return;
    try { await navigator.clipboard.writeText(JSON.stringify(trace, null, 2)); exportMessage = 'Trace copied.'; }
    catch { exportMessage = 'Clipboard unavailable.'; }
  }

  async function setVerbose(value: boolean) {
    verboseEnabled = value;
    await invoke('set_dev_logging_enabled', { enabled: value }).catch(() => { verboseEnabled = !value; });
  }

  async function setRuin(value: boolean) {
    ruinAccessibility = value; appStore.ruinAccessibility = value;
    if (value) appStore.devModeEnabled = true;
    await saveSetting('ruin_accessibility', value).catch(() => { ruinAccessibility = !value; appStore.ruinAccessibility = !value; });
  }

  async function setDevModeOnStartup(value: boolean) {
    const previous = devModeOnStartup;
    devModeOnStartup = value;
    appStore.devModeOnStartup = value;
    await saveSetting('dev_mode_on_startup', value).catch(() => {
      devModeOnStartup = previous;
      appStore.devModeOnStartup = previous;
    });
  }

  async function setForceSetup(value: boolean) {
    forceSetupOnLaunch = value;
    await saveSetting('force_setup_on_launch', value).catch(() => { forceSetupOnLaunch = !value; });
  }

  async function setSync(value: boolean) {
    if (value && typeof window !== 'undefined' && !window.confirm('Enable experimental encrypted LAN sync?')) return;
    syncEnabled = value;
    await saveSetting('sync_enabled', value).then(() => { appStore.syncEnabled = value; }).catch(() => { syncEnabled = !value; });
  }

  function simulateProviderDown() {
    appStore.providerStatusSimulation = true;
    appStore.providerStatusAlerts = [{ providerId: provider, providerName: provider === 'google' ? 'Gemini' : provider, status: 'degraded', severity: 'high', message: 'Diagnostic provider degradation preview.', detailsUrl: '' }];
    faultMessage = `${provider} degraded preview enabled.`;
  }
  function simulateWifiOffline() { appStore.isOnline = false; faultMessage = 'Offline state enabled.'; }
  function simulateDriveFull() { appStore.recoveryStorageWarning = true; faultMessage = 'Drive-full notice enabled.'; }
  async function simulateGlobalMessage() { appStore.globalMessageSimulation = true; appStore.globalMessage = { message: 'Diagnostic global message preview.', showToUsers: true }; faultMessage = 'Global message enabled.'; }
  async function toggleStorageFault(enabled: boolean) {
    storageFullSimulation = enabled;
    await invoke('set_storage_full_simulation', { enabled }).catch(() => { storageFullSimulation = !enabled; });
    faultMessage = enabled ? 'Storage write failures enabled.' : 'Storage write failures disabled.';
  }
  async function clearFaults() {
    appStore.providerStatusAlerts = []; appStore.providerStatusSimulation = false; appStore.globalMessage = null; appStore.globalMessageSimulation = false; appStore.recoveryStorageWarning = false; appStore.isOnline = true;
    await invoke('set_storage_full_simulation', { enabled: false }).catch(() => {}); storageFullSimulation = false; faultMessage = 'All reversible fault injections cleared.'; void checkStatus().catch(() => {});
  }
  async function testNotifications() {
    if (!(await ensureNotificationPermission())) { faultMessage = 'Notification permission was not granted.'; return; }
    await invoke('test_notifications', { notificationType: 'service' }).then(() => { faultMessage = 'Notification sent.'; }).catch((error) => { faultMessage = `Notification failed: ${String(error).slice(0, 100)}`; });
  }
  async function testInstaller() { await invoke<string>('reinstall_latest_update').then((version) => { faultMessage = `Installer test started for v${version}.`; }).catch((error) => { faultMessage = `Installer test failed: ${String(error).slice(0, 100)}`; }); }
  async function checkProviderStatus() {
    providerStatusRaw = '';
    await invoke('check_provider_status_raw').then((value) => { providerStatusRaw = JSON.stringify(value, null, 2); }).catch((error) => { providerStatusRaw = `Check failed: ${String(error).slice(0, 160)}`; });
  }

  onMount(() => {
    let active = true;
    // A whole dictation runs end to end in well under a fixed 2.5s poll gap, so at
    // the idle rate every Runtime stage node appeared to jump straight to "done" in
    // one snapshot. Ramp the interval down while a dictation is actually in flight
    // so each stage transition lands in its own poll instead of getting skipped.
    const IDLE_POLL_MS = 2_500;
    const ACTIVE_POLL_MS = 150;
    function scheduleNext() {
      if (!active) return;
      const delay = audioActive() || pipelineActive() ? ACTIVE_POLL_MS : IDLE_POLL_MS;
      pollTimer = setTimeout(tick, delay);
    }
    async function tick() {
      if (active && monitoring && typeof document !== 'undefined' && document.visibilityState === 'visible') {
        await refresh();
      }
      scheduleNext();
    }
    monitoring = true;
    void invoke('subscribe_log_stream').catch(() => {});
    void invoke('set_diagnostics_monitoring', { enabled: true }).then(() => refresh());
    scheduleNext();
    onVisibility = () => { if (document.visibilityState === 'visible') void refresh(); };
    document.addEventListener('visibilitychange', onVisibility);
    listen<StructuredLogEntry>('verenu:diagnostics', (event) => {
      if (paused) return;
      snapshot = { ...snapshot, logs: [...snapshot.logs.slice(-999), event.payload] };
      // Each structured log line is a real backend lifecycle transition — refresh
      // on receipt too, so the ramp from idle to fast polling doesn't miss the
      // first stage change while waiting for the next scheduled tick.
      void refresh();
    }).then((unlisten) => { if (active) unlistenDiagnostics = unlisten; else unlisten(); }).catch(() => {});
    Promise.all([
      invoke<boolean>('get_dev_logging_enabled'), invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' }),
      invoke<boolean | null>('get_setting', { key: 'dev_mode_on_startup' }),
      invoke<boolean | null>('get_setting', { key: 'force_setup_on_launch' }), invoke<boolean | null>('get_setting', { key: 'sync_enabled' }),
      invoke<boolean>('get_storage_full_simulation'),
    ]).then(([verbose, ruin, devStartup, force, sync, storage]) => { verboseEnabled = verbose; ruinAccessibility = ruin ?? false; devModeOnStartup = devStartup ?? false; forceSetupOnLaunch = force ?? false; syncEnabled = sync ?? false; storageFullSimulation = storage; }).catch(() => {});
    return () => { active = false; if (pollTimer) clearTimeout(pollTimer); document.removeEventListener('visibilitychange', onVisibility!); if (unlistenDiagnostics) unlistenDiagnostics(); void invoke('unsubscribe_log_stream').catch(() => {}); if (!recording) void invoke('set_diagnostics_monitoring', { enabled: false }).catch(() => {}); };
  });
</script>

<svelte:window onkeydown={(event) => { if (event.key === 'Escape') selectedTrace = null; }} />

<section class="diagnostics-console" aria-label="Developer diagnostics">
  <div class="diag-head">
    <div><h2 class="settings-h">Developer</h2><p class="panel-note">Bounded, metadata-first observability for Verenu. Payloads, prompts, clipboard contents, and dictated text are excluded.</p></div>
    <div class="diag-head-actions">
      <span class="diag-updated">Last updated {shortTime(snapshot.generated_at_ms)}</span>
      <button class="btn-ghost btn-compact" onclick={() => void refresh()}>Update</button>
      <button class="btn-ghost btn-compact" title="Captures detailed per-stage timing for the next dictations, for diagnosing slow or stuck pipelines" onclick={() => void toggleRecording()}>{recording ? 'Stop recording' : 'Record profile'}</button>
    </div>
  </div>
  <nav class="diag-tabs" aria-label="Diagnostics views">
    {#each views as item}
      <button class:active={view === item.id} class="diag-tab" role="tab" data-setting-target={`developer-${item.id}`} aria-selected={view === item.id} aria-current={view === item.id ? 'page' : undefined} onclick={() => view = item.id}>
        {item.label}
        {#if view === item.id}<div class="diag-tab-bar" in:receive={{key: 'diag-tab'}} out:send={{key: 'diag-tab'}}></div>{/if}
      </button>
    {/each}
  </nav>

  {#key view}
  <div class="diag-view" in:fade={{ duration: motionMs(MOTION_MS.base) }}>
  {#if view === 'overview'}
    {@const resource = currentResource()}
    <div class="metric-grid">
      <div class="metric"><span>CPU</span><strong>{resource?.cpu_percent == null ? 'Unavailable' : `${resource.cpu_percent.toFixed(1)}%`}</strong><small>process tree</small></div>
      <div class="metric"><span>Resident</span><strong>{formatBytes(resource?.resident_bytes)}</strong><small>peak {formatBytes(resource?.peak_resident_bytes)}</small></div>
      <div class="metric"><span>Process count</span><strong>{unknown(resource?.process_count)}</strong><small>children {resource?.child_processes?.length ?? 0}</small></div>
      <div class="metric"><span>I/O read</span><strong>{formatRate(resource?.read_bytes_per_sec, '/s')}</strong><small>{formatBytes(resource?.read_bytes_total)} total</small></div>
      <div class="metric"><span>I/O write</span><strong>{formatRate(resource?.write_bytes_per_sec, '/s')}</strong><small>{formatBytes(resource?.write_bytes_total)} total</small></div>
      <div class="metric"><span>Failures</span><strong>{snapshot.health.total_failures_recorded}</strong><small>{snapshot.failure_groups.length} fingerprints</small></div>
      <div class="metric"><span>IPC rate</span><strong>{formatRate(operationRows().reduce((sum, row) => sum + row.calls_per_minute, 0), '/min')}</strong><small>{operationRows().length} tracked operations</small></div>
      <div class="metric"><span>Profiler overhead</span><strong>{snapshot.health.collector_duration_us_average == null ? 'Unavailable' : `${(snapshot.health.collector_duration_us_average / 1000).toFixed(2)} ms`}</strong><small>{snapshot.health.collector_samples} samples</small></div>
    </div>
    <div class="two-col"><section class="diag-panel"><div class="panel-title"><h3>Current runtime</h3><span class="mono">{shortTime(snapshot.generated_at_ms)}</span></div><div class="key-lines"><div><span>Dictation</span><b>{appStore.pillState || 'idle'}</b></div><div><span>Local STT</span><b>{unknown(snapshot.runtime.local_stt?.current_model_id)}</b></div><div><span>Local cleanup</span><b>{unknown(snapshot.runtime.local_llm?.current_model_id)}</b></div><div><span>Active traces</span><b>{snapshot.health.active_trace_count}</b></div></div></section><section class="diag-panel"><div class="panel-title"><h3>Last pipeline</h3><button class="link-btn" onclick={() => view = 'pipeline'}>Inspect</button></div>{#if latestTrace()}<div class="trace-summary"><span class="mono">{latestTrace()!.trace_id}</span><strong>{formatDuration(latestTrace()!.duration_ms)}</strong><em class:bad={latestTrace()!.outcome === 'failure'}>{latestTrace()!.outcome}</em></div>{:else}<p class="muted">No completed pipeline retained.</p>{/if}</section></div>
    <div class="toolbar"><button class="btn-ghost btn-compact" onclick={() => void clearDiagnostics()}>Clear retained data</button><button class="btn-ghost btn-compact" onclick={() => void download('json')}>Download diagnostics bundle</button><span data-setting-target="developer-download-logs"><button class="btn-ghost btn-compact" onclick={() => void download('text')}>Download Logs</button></span>{#if exportMessage}<span class="muted">{exportMessage}</span>{/if}</div>
  {:else if view === 'pipeline'}
    <section class="diag-panel" data-setting-target="developer-pipeline"><div class="panel-title"><h3>Pipeline traces</h3><span class="muted">{snapshot.active_pipelines.length} active · {snapshot.recent_pipelines.length} completed</span></div>{#each [...snapshot.active_pipelines, ...snapshot.recent_pipelines].slice(-8).reverse() as trace}<button class="trace-row" class:selected={selectedTrace === trace.trace_id} onclick={() => selectedTrace = trace.trace_id}><span class="mono">{trace.trace_id}</span><span>{trace.root_operation}</span><span>{trace.spans.length} stages</span><strong>{formatDuration(trace.duration_ms)}</strong><em class:bad={trace.outcome === 'failure'}>{trace.outcome}</em></button>{/each}{#if !snapshot.active_pipelines.length && !snapshot.recent_pipelines.length}<p class="muted">Start a dictation to capture a bounded timeline.</p>{/if}</section>
    {#if selectedTrace}<section class="diag-panel trace-detail"><div class="panel-title"><h3>Waterfall <span class="mono">{selectedTrace}</span></h3><div><button class="link-btn" onclick={() => void copyTrace([...snapshot.active_pipelines, ...snapshot.recent_pipelines].find((item) => item.trace_id === selectedTrace))}>Copy trace</button><button class="link-btn" onclick={() => selectedTrace = null}>Close</button></div></div>{#each [...snapshot.active_pipelines, ...snapshot.recent_pipelines].filter((item) => item.trace_id === selectedTrace) as trace}{#each trace.spans as span}<div class="span-row"><span class="span-label">{span.stage ?? span.operation}</span><div class="waterfall"><i style={`width:${spanWidth(span.duration_ms, trace.duration_ms)}%;`} class:failed={span.outcome === 'failure'}></i></div><span class="mono">{formatDuration(span.duration_ms)}</span><span>{span.provider ?? span.model ?? ''}</span></div>{/each}{/each}</section>{/if}
  {:else if view === 'failures'}
    <div class="two-col"><section class="diag-panel" data-setting-target="developer-latest-failures"><div class="panel-title"><h3>Latest failures</h3><span>{snapshot.latest_failures.length}</span></div>{#each snapshot.latest_failures.slice(-20).reverse() as failure}<details class="failure-row"><summary><span class="severity-dot"></span><span>{shortTime(failure.timestamp_ms)}</span><b>{subsystemLabel(failure.subsystem)}</b><span>{failure.operation ?? failure.stage ?? 'unknown'}</span><strong>{failure.cause}</strong></summary><div class="detail-grid"><span>fingerprint <code>{failure.fingerprint}</code></span><span>trace <code>{failure.trace_id ?? '—'}</code></span><span>duration {formatDuration(failure.duration_ms)}</span><span>{failure.provider ?? ''} {failure.model ?? ''}</span></div></details>{/each}{#if !snapshot.latest_failures.length}<p class="muted">No failures retained.</p>{/if}</section><section class="diag-panel"><div class="panel-title"><h3>Most common</h3><span>normalized</span></div>{#each [...snapshot.failure_groups].sort((a, b) => b.count - a.count).slice(0, 20) as group}<div class="group-row"><span class="mono">{group.fingerprint}</span><b>{group.count}×</b><span>{subsystemLabel(group.subsystem)} / {group.operation ?? group.stage ?? 'unknown'}</span><small>{group.representative_cause}</small></div>{/each}{#if !snapshot.failure_groups.length}<p class="muted">No grouped failures yet.</p>{/if}</section></div>
  {:else if view === 'logs'}
    <section class="diag-panel" data-setting-target="developer-logs"><div class="panel-title"><h3>Structured logs</h3><span>{filteredLogs().length} / {snapshot.logs.length}</span></div><div class="log-toolbar"><div class="log-filters">
      <input aria-label="Search logs" placeholder="Search message, operation, trace…" bind:value={logQuery} />
      <Dropdown bind:open={logLevelOpen} closeSelector=".log-level-dropdown">
        <div class="ui-dropdown log-level-dropdown">
          <button class="btn-ghost ui-dropdown-trigger" aria-haspopup="true" aria-expanded={logLevelOpen} aria-label="Log level" onclick={() => logLevelOpen = !logLevelOpen}>
            <span>{LOG_LEVELS.find((l) => l.value === logLevel)?.label}</span>
            <svg class:open={logLevelOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
          </button>
          {#if logLevelOpen}
            <div class="ui-dropdown-menu ui-dropdown-menu--padded" role="listbox" aria-label="Log level options" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
              {#each LOG_LEVELS as opt}<button class="ui-dropdown-option" class:active={logLevel === opt.value} onclick={() => { logLevel = opt.value; logLevelOpen = false; }}>{opt.label}</button>{/each}
            </div>
          {/if}
        </div>
      </Dropdown>
      <Dropdown bind:open={logSubsystemOpen} closeSelector=".log-subsystem-dropdown">
        <div class="ui-dropdown log-subsystem-dropdown">
          <button class="btn-ghost ui-dropdown-trigger" aria-haspopup="true" aria-expanded={logSubsystemOpen} aria-label="Log subsystem" onclick={() => logSubsystemOpen = !logSubsystemOpen}>
            <span class="subsystem-trigger-label">{subsystemLabel(logSubsystem)}</span>
            <svg class:open={logSubsystemOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
          </button>
          {#if logSubsystemOpen}
            <div class="ui-dropdown-menu ui-dropdown-menu--padded subsystem-menu scroll-styled scroll-thumb-elev" role="listbox" aria-label="Log subsystem options" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
              {#each subsystemOptions() as opt}<button class="ui-dropdown-option" class:active={logSubsystem === opt.value} onclick={() => { logSubsystem = opt.value; logSubsystemOpen = false; }}>{opt.label}</button>{/each}
            </div>
          {/if}
        </div>
      </Dropdown>
      <input class="trace-filter" aria-label="Trace filter" placeholder="Trace ID" bind:value={logTrace} />
      </div>
      <div class="log-actions" aria-label="Log actions">
        <button class="link-btn" onclick={() => paused = !paused}>{paused ? 'Resume' : 'Pause'}</button>
        <button class="link-btn" onclick={() => autoScroll = !autoScroll}>{autoScroll ? 'Auto-scroll' : 'Manual scroll'}</button>
        <button class="link-btn" onclick={() => void copyLogs()}>Copy visible</button>
        <button class="link-btn" onclick={() => void copyAllLogs()}>Copy all</button>
      </div>
    </div><div class="log-table" role="table">{#each filteredLogs().slice(-500).reverse() as entry}<div class="log-row" role="row"><time>{shortTime(entry.timestamp_ms)}</time><b class={`level-${entry.level}`}>{entry.level}</b><span class="subsystem">{subsystemLabel(entry.subsystem)}</span><span>{entry.operation ?? entry.stage ?? ''}</span><span class="message">{entry.message}</span><code>{entry.trace_id ?? ''}</code></div>{/each}</div></section>
  {:else if view === 'runtime'}
    <section class="diag-panel runtime-panel" data-setting-target="developer-runtime"><div class="panel-title"><h3>Live runtime</h3><span class="stage-live" data-state={stageState(null)}><i class="node-dot"></i>{pillLabel()}</span></div><div class="runtime-flow">{#each RUNTIME_STAGES as node, index}<div class="runtime-node" data-state={stageState(node.id)}><span class="node-head"><i class="node-dot"></i>{node.label}</span><small>{stageDetail(node.id)}</small></div>{#if index < RUNTIME_STAGES.length - 1}<span class="flow-arrow">→</span>{/if}{/each}</div><div class="audio-grid"><div><span>Recording state</span><b>{audioActive() ? 'active' : (appStore.pillState || 'idle')}</b></div><div><span>Raw RMS / processed</span><b>{snapshot.runtime.audio ? `${snapshot.runtime.audio.raw_rms?.toFixed(4) ?? '—'} / ${snapshot.runtime.audio.processed_level?.toFixed(4) ?? '—'}` : 'Unavailable'}</b></div><div><span>Gate threshold / verdict</span><b>{snapshot.runtime.audio ? `${snapshot.runtime.audio.gate_rms?.toFixed(4) ?? '—'} / ${snapshot.runtime.audio.would_pass_gate == null ? 'unknown' : snapshot.runtime.audio.would_pass_gate ? 'pass' : 'below gate'}` : 'Unavailable when idle'}</b></div><div><span>VAD / sensitivity / gain</span><b>{snapshot.runtime.audio ? `${snapshot.runtime.audio.speech_detected ? 'speech' : 'quiet'} / L${snapshot.runtime.audio.adaptive_sensitivity ?? '—'} / ×${snapshot.runtime.audio.microphone_gain?.toFixed(2) ?? '—'}` : 'Unavailable when idle'}</b></div></div>{#if snapshot.runtime.audio?.stream_error}<p class="panel-note bad-note">The capture stream reported an error; the session may be ending.</p>{:else}<p class="panel-note">Audio diagnostics read the existing recording session atomics and the pipeline’s actual gate threshold. No duplicate audio processing or retained audio buffers are created by this view.</p>{/if}</section>
  {:else if view === 'activity'}
    <section class="diag-panel" data-setting-target="developer-activity"><div class="panel-title"><h3>Activity / operations</h3><span class="muted">backend + frontend invoke metrics</span></div>{#each groupedOperations() as group}<h4 class="operation-group-h">{group.label}</h4><div class="operation-table"><div class="operation-head"><span>Operation</span><span>Calls / min</span><span>Total</span><span>Avg</span><span>p95</span><span>Failures</span><span>Active</span></div>{#each group.rows.slice(0, 40) as operation}<div class="operation-row"><code>{operation.operation}</code><span>{operation.calls_per_minute}</span><span>{formatDuration(operation.total_duration_ms)}</span><span>{formatDuration(operation.average_duration_ms)}</span><span>{formatDuration(operation.p95_duration_ms)}</span><span class:bad={operation.failure_count > 0}>{operation.failure_count}</span><span>{operation.currently_running}</span></div>{/each}</div>{/each}{#if frontendMetrics.some((metric) => metric.failures > 0)}<div class="frontend-errors"><h4 class="operation-group-h">Latest frontend IPC errors</h4>{#each frontendMetrics.filter((metric) => metric.failures > 0 && metric.last_error) as metric}<div class="frontend-error-row"><code>{metric.command}</code><span>{metric.last_error}</span></div>{/each}</div>{/if}</section>
  {:else if view === 'storage'}
    <div class="two-col"><section class="diag-panel"><div class="panel-title"><h3>Resource timeline</h3><span class="muted">{snapshot.resource_samples.length} points</span></div><div class="resource-chart">{#each snapshot.resource_samples.slice(-60) as sample}<i title={`${shortTime(sample.observed_at_ms)} ${formatBytes(sample.snapshot.resident_bytes)}`} style={`height:${Math.min(100, Math.max(4, ((sample.snapshot.resident_bytes ?? 0) / Math.max(1, currentResource()?.peak_resident_bytes ?? sample.snapshot.resident_bytes ?? 1)) * 100))}%;`}></i>{/each}</div><div class="key-lines"><div><span>Peak resident</span><b>{formatBytes(currentResource()?.peak_resident_bytes)}</b></div><div><span>GPU signal</span><b>{formatBytes(currentResource()?.gpu_memory_bytes)}</b></div><div><span>Collector samples</span><b>{snapshot.health.collector_samples}</b></div><div><span>Cleanup cache</span><b>{unknown(snapshot.runtime.cleanup_cache?.entry_count)}</b></div><div><span>Sync log / peers</span><b>{unknown(snapshot.runtime.sync?.log_entries)} / {unknown(snapshot.runtime.sync?.peer_count)}</b></div></div></section><section class="diag-panel"><div class="panel-title"><h3>Bounded retention</h3><span class="muted">health</span></div><div class="key-lines"><div><span>Logs</span><b>{snapshot.health.retained_log_count} / dropped {snapshot.health.dropped_logs}</b></div><div><span>Failures</span><b>{snapshot.health.retained_failure_count} / dropped {snapshot.health.dropped_failures}</b></div><div><span>Traces</span><b>{snapshot.health.retained_trace_count} / dropped {snapshot.health.dropped_traces}</b></div><div><span>Resource samples</span><b>{snapshot.health.retained_resource_sample_count} / dropped {snapshot.health.dropped_resource_samples}</b></div><div><span>Auto-learn promotions</span><b>{unknown(snapshot.runtime.auto_learn?.promotions)}</b></div></div></section></div>
  {:else if view === 'faults'}
    <section class="diag-panel" data-setting-target="developer-simulations"><div class="panel-title"><h3>Fault injection</h3><span class="muted">reversible previews and controlled failures</span></div><div class="fault-grid"><button class="btn-ghost btn-compact" onclick={simulateProviderDown}>Provider Down</button><button class="btn-ghost btn-compact" onclick={simulateWifiOffline}>Wi-Fi Offline</button><button class="btn-ghost btn-compact" onclick={simulateGlobalMessage}>Global Message</button><button class="btn-ghost btn-compact" onclick={simulateDriveFull}>Drive Full</button><button class="btn-ghost btn-compact" data-setting-target="developer-notifications" onclick={() => void testNotifications()}>Send Notification</button><button class="btn-ghost btn-compact" data-setting-target="developer-installer" onclick={() => void testInstaller()}>Reinstall Latest Stable</button><button class="btn-ghost btn-compact" data-setting-target="developer-status" onclick={() => void checkProviderStatus()}>Run Check</button></div><div class="fault-row"><div class="fault-provider">
      <span class="fault-provider-label">Provider</span>
      <Dropdown bind:open={faultProviderOpen} closeSelector=".fault-provider-dropdown">
        <div class="ui-dropdown fault-provider-dropdown">
          <button class="btn-ghost ui-dropdown-trigger" aria-haspopup="true" aria-expanded={faultProviderOpen} aria-label="Fault injection provider" onclick={() => faultProviderOpen = !faultProviderOpen}>
            <span>{PROVIDER_OPTIONS.find((p) => p.value === provider)?.label}</span>
            <svg class:open={faultProviderOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
          </button>
          {#if faultProviderOpen}
            <div class="ui-dropdown-menu ui-dropdown-menu--padded" role="listbox" aria-label="Fault injection provider options" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
              {#each PROVIDER_OPTIONS as opt}<button class="ui-dropdown-option" class:active={provider === opt.value} onclick={() => { provider = opt.value; faultProviderOpen = false; }}>{opt.label}</button>{/each}
            </div>
          {/if}
        </div>
      </Dropdown>
    </div><span data-setting-target="developer-storage-simulation"><Toggle checked={storageFullSimulation} onchange={toggleStorageFault} label="Full Storage Failure" /></span></div>{#if providerStatusRaw}<pre class="status-output">{providerStatusRaw}</pre>{/if}<div class="toolbar"><button class="btn-danger btn-compact" onclick={() => void clearFaults()}>Clear</button>{#if faultMessage}<span class="muted">{faultMessage}</span>{/if}</div></section>
  {:else}
    <section class="diag-panel" data-setting-target="developer-settings"><div class="panel-title"><h3>Developer settings</h3><span class="muted">explicit opt-in controls</span></div><div class="setting-row" data-setting-target="developer-dev-mode-startup"><div><div class="label">Enable dev mode on startup</div><div class="desc">Keep Developer visible automatically whenever Verenu starts.</div></div><Toggle checked={devModeOnStartup} onchange={setDevModeOnStartup} label="Enable dev mode on startup" /></div><div class="setting-row" data-setting-target="developer-ruin-accessibility"><div><div class="label">Ruin accessibility</div><div class="desc">Expose a compact, privacy-filtered state dump for T3 Code SnapShots. It never includes logs, transcripts, prompts, or keys.</div></div><Toggle checked={ruinAccessibility} onchange={setRuin} label="Ruin accessibility" /></div><div class="setting-row" data-setting-target="developer-logs"><div><div class="label">Verbose logging</div><div class="desc">{verboseEnabled ? 'Debug logging enabled.' : 'Metadata diagnostics remain useful with verbose logging off.'} <button class="link-btn privacy-link" onclick={() => privacyModalOpen = true}>Privacy details</button></div></div><Toggle checked={verboseEnabled} onchange={setVerbose} label="Verbose logging" /></div><div class="setting-row" data-setting-target="developer-setup"><div><div class="label">Force setup on launch</div><div class="desc">Show onboarding without erasing saved settings.</div></div><Toggle checked={forceSetupOnLaunch} onchange={setForceSetup} label="Force setup" /></div><div class="setting-row" data-setting-target="developer-sync"><div><div class="label">LAN Device Sync</div><div class="desc">Experimental encrypted device-to-device sync. Off by default.</div></div><Toggle checked={syncEnabled} onchange={setSync} label="Enable LAN sync" /></div></section>
  {/if}
  </div>
  {/key}
</section>

{#if privacyModalOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <button class="modal-backdrop" aria-label="Close dialog" onclick={() => (privacyModalOpen = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="modal-card"
    use:modalFocusTrap={{ active: privacyModalOpen, initialFocus: () => privacyModalCloseBtn }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="privacy-warning-title"
    tabindex="-1"
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="modal-header"><h2 id="privacy-warning-title" class="modal-title">Verbose logging privacy</h2></div>
    <div class="modal-body"><p class="confirm-copy">Verbose logging can contain dictated text and provider prompts. Keep it off unless you are deliberately investigating a local issue.</p></div>
    <div class="modal-footer"><div class="footer-actions"><button bind:this={privacyModalCloseBtn} class="btn-primary" onclick={() => (privacyModalOpen = false)}>Got it</button></div></div>
  </div>
{/if}

<style>
  .diag-head, .panel-title, .toolbar, .log-toolbar, .fault-row { display:flex; align-items:center; gap:10px; }
  .diag-head { justify-content:space-between; align-items:flex-start; }
  .diag-head-actions { display:flex; gap:8px; align-items:center; }
  .diag-updated { font-size:11.5px; color:var(--ink-mute); white-space:nowrap; }

  .diag-tabs { display:flex; gap:20px; overflow-x:auto; overflow-y:hidden; flex-wrap:nowrap; border-bottom:1px solid var(--line); margin:20px 0 18px; }
  .diag-tab { position:relative; background:none; border:0; color:var(--ink-mute); padding:0 0 10px; white-space:nowrap; font-size:12.5px; font-family:var(--sans); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) var(--ui-ease-out, ease); }
  .diag-tab:hover { color:var(--ink-soft); }
  .diag-tab.active { color:var(--ink); font-weight:500; }
  .diag-tab-bar { position:absolute; bottom:-1px; left:0; right:0; height:2px; background:var(--accent); }

  .diag-view { min-width:0; }
  .frontend-errors { margin-top:18px; }
  .frontend-error-row { display:grid; grid-template-columns:minmax(150px, .35fr) minmax(0, 1fr); gap:12px; padding:8px 0; border-top:1px solid var(--line-soft); font-size:11.5px; }
  .frontend-error-row code { color:var(--ink-soft); }
  .frontend-error-row span { color:var(--danger, #ef4444); overflow-wrap:anywhere; }
  /* The tab bar's own bottom border already separates it from the view below —
     a first section's top hairline would just double that line 18px later. */
  .diag-view > .diag-panel:first-child,
  .diag-view > .two-col:first-child { border-top:0; }

  .stage-live { display:inline-flex; align-items:center; gap:6px; font-size:11px; color:var(--ink-mute); font-weight:500; text-transform:capitalize; }
  .stage-live .node-dot { background:var(--ink-faint); }
  .stage-live[data-state="active"] { color:var(--success); }
  .stage-live[data-state="active"] .node-dot { background:var(--success); animation:diag-pulse 1.6s ease-in-out infinite; }
  .stage-live[data-state="done"] { color:var(--accent-ink); }
  .stage-live[data-state="done"] .node-dot { background:var(--accent); }
  @keyframes diag-pulse { 0%, 100% { opacity:1; } 50% { opacity:.4; } }

  /* One bounded stat table, not eight separate cards — internal hairlines
     carry the grouping instead of a box per metric. */
  .metric-grid { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); border:1px solid var(--line); border-radius:var(--r-md); overflow:hidden; }
  .metric { padding:14px; display:flex; flex-direction:column; gap:4px; border-right:1px solid var(--line); border-bottom:1px solid var(--line); }
  .metric:nth-child(4n) { border-right:0; }
  .metric:nth-child(n+5) { border-bottom:0; }
  .metric span, .metric small, .muted { color:var(--ink-mute); }
  .metric span, .metric small { font-size:11px; }
  .metric strong { font:600 18px var(--sans); color:var(--ink); font-variant-numeric:tabular-nums; }
  .metric small { font:10px var(--mono); }

  /* Flat, titled sections divided by a single hairline — no card boxes. */
  .diag-panel { padding-top:16px; margin-top:22px; min-width:0; border-top:1px solid var(--line); }
  .diag-panel:first-child { margin-top:0; }
  .diag-panel h3 { font-size:13px; margin:0; color:var(--ink); font-weight:600; }
  .panel-title { justify-content:space-between; margin-bottom:12px; }
  .panel-title > span { font-size:11px; color:var(--ink-mute); }
  .key-lines { display:grid; gap:2px; }
  .key-lines > div { display:flex; justify-content:space-between; gap:12px; font-size:12px; padding:5px 0; border-bottom:1px solid var(--line-soft); color:var(--ink-soft); }
  .key-lines b { font-family:var(--mono); font-size:11px; text-align:right; color:var(--ink); font-weight:500; }

  .two-col { display:grid; grid-template-columns:1fr 1fr; gap:24px; margin-top:22px; padding-top:16px; border-top:1px solid var(--line); }
  .two-col .diag-panel { border-top:0; margin-top:0; padding-top:0; }
  .two-col .diag-panel + .diag-panel { border-left:1px solid var(--line); padding-left:24px; }

  .mono, code { font-family:var(--mono); font-size:10px; }
  .trace-summary, .trace-row, .group-row { display:grid; grid-template-columns:minmax(100px,1fr) auto auto; gap:10px; align-items:center; }
  .trace-summary { padding:5px 0; }
  em { color:var(--success); font-style:normal; font-size:11px; }
  em.bad, .bad { color:var(--danger); }
  .link-btn { background:none; border:0; color:var(--accent-ink); cursor:pointer; padding:2px; font-size:12px; white-space:nowrap; transition:opacity var(--ui-duration-fast, 150ms) ease; }
  .link-btn:hover { opacity:0.75; }

  .trace-row { width:100%; text-align:left; border:0; border-bottom:1px solid var(--line); background:none; color:inherit; padding:9px 4px; cursor:pointer; grid-template-columns:minmax(100px,1.2fr) 1fr auto auto auto; border-radius:var(--r-sm); transition:background var(--ui-duration-fast, 150ms) ease; }
  .trace-row:hover { background:var(--control-hover); }
  .trace-row.selected { background:var(--accent-soft); }
  .trace-detail { overflow:auto; }
  .span-row { display:grid; grid-template-columns:140px 1fr 65px 120px; gap:8px; align-items:center; padding:6px 0; font-size:11px; }
  .waterfall { height:7px; background:var(--line); border-radius:4px; overflow:hidden; }
  .waterfall i { display:block; height:100%; min-width:2px; background:var(--accent); transition:width var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); }
  .waterfall i.failed { background:var(--danger); }

  .failure-row { border-bottom:1px solid var(--line); padding:8px 0; }
  .failure-row summary { cursor:pointer; display:grid; grid-template-columns:7px 62px minmax(0,110px) minmax(0,1fr) minmax(0,1.3fr); gap:10px; align-items:center; list-style:none; font-size:11px; color:var(--ink-soft); }
  .failure-row summary::-webkit-details-marker { display:none; }
  .failure-row summary > * { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .severity-dot { width:6px; height:6px; border-radius:50%; background:var(--danger); }
  .failure-row strong { font-weight:500; color:var(--ink); }
  .detail-grid { display:flex; flex-wrap:wrap; gap:8px 16px; padding:8px 0 2px 79px; color:var(--ink-mute); font-size:10px; }

  .group-row { grid-template-columns:100px 34px 1fr; padding:8px 0; border-bottom:1px solid var(--line); font-size:11px; color:var(--ink-soft); }
  .group-row small { grid-column:2 / -1; color:var(--ink-mute); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

  .log-toolbar { justify-content:space-between; flex-wrap:wrap; margin-bottom:12px; gap:8px 12px; }
  .log-filters, .log-actions { display:flex; align-items:center; gap:8px; min-width:0; }
  .log-filters { flex:1 1 520px; }
  .log-actions { flex:0 0 auto; padding-left:4px; }
  input { background:var(--bg-elev); border:1px solid var(--line); color:var(--ink); border-radius:var(--r-sm); padding:6px 8px; font-size:11px; min-width:0; font-family:var(--sans); transition:border-color var(--ui-duration-fast, 150ms) ease; }
  input:focus-visible { outline:none; border-color:var(--ink-strong); }
  .log-filters input:first-child { flex:1 1 180px; }
  .log-filters .trace-filter { flex:0 1 110px; width:110px; }
  .log-level-dropdown .ui-dropdown-trigger { min-width:100px; }
  .log-subsystem-dropdown { min-width:190px; }
  .log-subsystem-dropdown .ui-dropdown-trigger { width:100%; max-width:220px; }
  .subsystem-trigger-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .subsystem-menu { max-height:260px; min-width:250px; }
  .log-table { border-top:1px solid var(--line); }
  .log-row { display:grid; grid-template-columns:70px 45px 105px 110px minmax(180px,1fr) 110px; gap:7px; align-items:center; padding:6px 4px; border-bottom:1px solid var(--line-soft); font:11px var(--mono); }
  .log-row time, .log-row code, .subsystem { color:var(--ink-mute); font-size:10px; }
  .log-row .message { color:var(--ink-soft); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .level-error { color:var(--danger); }
  .level-warn { color:var(--warning); }
  .level-info { color:var(--ink-mute); }

  /* Wraps instead of scrolling sideways — every stage stays visible at once. */
  .runtime-flow { display:flex; flex-wrap:wrap; align-items:center; gap:6px; padding:12px 0; }
  .runtime-node { flex:1 1 110px; padding:9px 10px; border:1px solid var(--line); border-radius:var(--r-sm); background:var(--bg-elev); display:flex; flex-direction:column; gap:4px; transition:border-color var(--ui-duration-base, 220ms) var(--ui-ease-out, ease), opacity var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); }
  .flow-arrow { color:var(--ink-faint); font-size:12px; flex:0 0 auto; }
  .node-head { display:flex; align-items:center; gap:6px; font-size:11px; color:var(--ink-soft); }
  .node-dot { display:inline-block; width:6px; height:6px; border-radius:50%; background:var(--ink-faint); flex-shrink:0; transition:background-color var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); }
  .runtime-node small { color:var(--ink-mute); font:9px var(--mono); padding-left:12px; }
  /* active = this span is running right now; done = it already finished —
     kept visually quiet so the eye lands on whichever node is actually active. */
  .runtime-node[data-state="active"] { border-color:var(--success); }
  .runtime-node[data-state="active"] .node-dot { background:var(--success); animation:diag-pulse 1.2s ease-in-out infinite; }
  .runtime-node[data-state="done"] { border-color:var(--line); opacity:.6; }
  .runtime-node[data-state="done"] .node-dot { background:var(--accent); }
  .runtime-node[data-state="error"] { border-color:var(--danger); }
  .runtime-node[data-state="error"] .node-dot { background:var(--danger); }
  .audio-grid { display:grid; grid-template-columns:repeat(4,1fr); gap:8px; margin-top:16px; }
  .audio-grid > div { display:flex; flex-direction:column; gap:3px; font-size:11px; color:var(--ink-soft); }
  .audio-grid b { font:10px var(--mono); color:var(--ink); }

  .operation-group-h { font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.04em; color:var(--ink-mute); margin:18px 0 6px; }
  .operation-group-h:first-child { margin-top:0; }
  .operation-table { overflow:auto; }
  .operation-head, .operation-row { display:grid; grid-template-columns:minmax(190px,1.6fr) repeat(6, minmax(55px,.6fr)); gap:8px; align-items:center; padding:8px 4px; border-bottom:1px solid var(--line-soft); font-size:11px; }
  .operation-head { color:var(--ink-mute); font-size:10px; text-transform:uppercase; letter-spacing:.04em; border-bottom-color:var(--line); }
  .operation-row code { color:var(--ink-soft); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

  .resource-chart { height:110px; display:flex; align-items:flex-end; gap:2px; border-bottom:1px solid var(--line); padding:8px 0 0; }
  .resource-chart i { flex:1; min-width:2px; background:var(--accent); opacity:.75; border-radius:1px 1px 0 0; transition:opacity var(--ui-duration-fast, 150ms) ease; }
  .resource-chart i:hover { opacity:1; }

  .fault-grid { display:grid; grid-template-columns:repeat(3,1fr); gap:14px; }
  .fault-row { justify-content:space-between; margin-top:22px; }
  .fault-provider { display:flex; align-items:center; gap:10px; }
  .fault-provider-label { font-size:11px; color:var(--ink-soft); }
  .status-output { max-height:140px; overflow:auto; color:var(--ink-mute); font:10px var(--mono); white-space:pre-wrap; background:var(--bg-elev); border:1px solid var(--line); border-radius:var(--r-sm); padding:10px; margin-top:14px; }
  .toolbar { margin-top:18px; }

  .privacy-link { margin-left:4px; }

  .modal-backdrop { position:fixed; inset:0; border:0; padding:0; appearance:none; background:var(--overlay); z-index:50; outline:none; }
  .modal-card { position:fixed; top:50%; left:50%; translate:-50% -50%; z-index:51; isolation:isolate; background:var(--bg-elev); border:1px solid var(--line); border-radius:var(--r-lg); width:min(420px, calc(100vw - 40px)); box-shadow:var(--shadow-elev); overflow:hidden; }
  .modal-header { padding:20px 20px 0; }
  .modal-title { font-family:var(--sans); font-size:17px; font-weight:600; letter-spacing:-0.01em; color:var(--ink); margin:0; }
  .modal-body { padding:10px 20px 18px; }
  .confirm-copy { margin:0; font-size:13px; line-height:1.5; color:var(--ink-soft); }
  .modal-footer { padding:0 20px 20px; }
  .footer-actions { display:flex; justify-content:flex-end; gap:8px; }

  @media (max-width: 760px) {
    .metric-grid { grid-template-columns:repeat(2,1fr); }
    .metric { border-right:1px solid var(--line); border-bottom:1px solid var(--line); }
    .metric:nth-child(4n) { border-right:1px solid var(--line); }
    .metric:nth-child(2n) { border-right:0; }
    .metric:nth-child(n+5) { border-bottom:1px solid var(--line); }
    .metric:nth-child(n+7) { border-bottom:0; }
    .two-col { grid-template-columns:1fr; }
    .two-col .diag-panel + .diag-panel { border-left:0; padding-left:0; border-top:1px solid var(--line); padding-top:16px; margin-top:16px; }
    .audio-grid { grid-template-columns:repeat(2,1fr); }
    .fault-grid { grid-template-columns:repeat(2,1fr); }
    .log-filters { flex-basis:100%; flex-wrap:wrap; }
    .log-filters input:first-child { flex-basis:100%; }
    .log-actions { width:100%; justify-content:flex-start; padding-left:0; }
    .log-row { grid-template-columns:60px 40px 85px 1fr; }
    .log-row > :nth-child(4), .log-row > code { display:none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .runtime-node, .resource-chart i, .waterfall i { transition:none; }
    .stage-live .node-dot, .runtime-node .node-dot { animation:none; }
  }
</style>
