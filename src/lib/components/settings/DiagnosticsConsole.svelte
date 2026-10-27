<script lang="ts">
  import { onMount } from 'svelte';
  import { crossfade, fade, fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { appStore } from '../../stores';
  import { invoke, listen } from '../../tauri';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { frontendIpcActivity, type DiagnosticsSnapshot, type StructuredLogEntry } from '../../diagnostics';
  import { analyze, buildReport, clockTime, operationRows, type FindingView } from '../../diagnosticsInsights';
  import OverviewView from '../developer/OverviewView.svelte';
  import PipelineView from '../developer/PipelineView.svelte';
  import FailuresView from '../developer/FailuresView.svelte';
  import LogsView from '../developer/LogsView.svelte';
  import ResourcesView from '../developer/ResourcesView.svelte';
  import SettingsView from '../developer/SettingsView.svelte';

  const [send, receive] = crossfade({ duration: motionMs(MOTION_MS.fast), easing: expoOut });

  type View = 'overview' | 'pipeline' | 'failures' | 'logs' | 'resources' | 'settings';
  const views: { id: View; label: string }[] = [
    { id: 'overview', label: 'Overview' }, { id: 'pipeline', label: 'Pipeline' },
    { id: 'failures', label: 'Failures' }, { id: 'logs', label: 'Logs' },
    { id: 'resources', label: 'Resources' }, { id: 'settings', label: 'Developer Settings' },
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
  let recording = $state(false);
  let selectedTrace = $state<string | null>(null);
  let logQuery = $state('');
  let logLevel = $state('all');
  let logSubsystem = $state('all');
  let logTrace = $state('');
  let status = $state('');
  let statusTimer: ReturnType<typeof setTimeout> | null = null;
  let refreshRunning = false;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let refreshSoon: ReturnType<typeof setTimeout> | null = null;

  const findings = $derived(analyze(snapshot, frontendMetrics));
  const ipcPerMinute = $derived(operationRows(snapshot, frontendMetrics).reduce((sum, row) => sum + row.calls_per_minute, 0));
  const audioActive = $derived(snapshot.runtime.audio?.active === true);
  const pipelineActive = $derived(snapshot.active_pipelines.length > 0);
  const pillLabel = $derived(audioActive ? 'recording' : pipelineActive ? 'processing' : appStore.pillState || 'idle');
  const failureCount = $derived(snapshot.failure_groups.reduce((sum, group) => sum + group.count, 0));

  function say(message: string) {
    status = message;
    if (statusTimer) clearTimeout(statusTimer);
    statusTimer = setTimeout(() => (status = ''), 5000);
  }

  async function copy(text: string, what: string) {
    try { await navigator.clipboard.writeText(text); say(what); }
    catch { say('Clipboard unavailable.'); }
  }

  async function refresh() {
    if (refreshRunning || (typeof document !== 'undefined' && document.visibilityState !== 'visible')) return;
    refreshRunning = true;
    try {
      snapshot = await invoke<DiagnosticsSnapshot>('get_diagnostics_snapshot');
      frontendMetrics = frontendIpcActivity.snapshot();
    } catch (error) {
      say(`Snapshot unavailable: ${String(error).slice(0, 140)}`);
    } finally { refreshRunning = false; }
  }

  async function toggleRecording() {
    recording = !recording;
    await invoke('set_diagnostics_profiling', { enabled: recording }).catch(() => { recording = !recording; });
    void refresh();
  }

  async function download(format: 'json' | 'text') {
    try { say(`Saved ${await invoke<string>('download_diagnostics_bundle', { format })}`); }
    catch (error) { say(`Export failed: ${String(error).slice(0, 140)}`); }
  }

  async function clearDiagnostics() {
    if (typeof window !== 'undefined' && !window.confirm('Clear retained diagnostics?')) return;
    await invoke('clear_diagnostics').then(() => { say('Retained diagnostics cleared.'); void refresh(); }).catch((error) => say(`Clear failed: ${String(error).slice(0, 120)}`));
  }

  function openTrace(id: string) { selectedTrace = id; view = 'pipeline'; }
  function openLogs(id: string) { logTrace = id; logLevel = 'all'; logQuery = ''; logSubsystem = 'all'; view = 'logs'; }
  function navigate(next: FindingView) { view = next; }

  onMount(() => {
    let active = true;
    const IDLE_POLL_MS = 2_500;
    const ACTIVE_POLL_MS = 250;
    let unlisten: (() => void) | null = null;
    const onVisibility = () => { if (document.visibilityState === 'visible') void refresh(); };

    async function tick() {
      if (active) await refresh();
      if (active) pollTimer = setTimeout(tick, audioActive || pipelineActive ? ACTIVE_POLL_MS : IDLE_POLL_MS);
    }

    void invoke('subscribe_log_stream').catch(() => {});
    void invoke('set_diagnostics_monitoring', { enabled: true }).then(() => refresh()).catch(() => {});
    pollTimer = setTimeout(tick, IDLE_POLL_MS);
    document.addEventListener('visibilitychange', onVisibility);
    listen<StructuredLogEntry>('verenu:diagnostics', (event) => {
      snapshot = { ...snapshot, logs: [...snapshot.logs.slice(-999), event.payload] };
      if (!refreshSoon) refreshSoon = setTimeout(() => { refreshSoon = null; void refresh(); }, 600);
    }).then((fn) => { if (active) unlisten = fn; else fn(); }).catch(() => {});

    return () => {
      active = false;
      if (pollTimer) clearTimeout(pollTimer);
      if (refreshSoon) clearTimeout(refreshSoon);
      if (statusTimer) clearTimeout(statusTimer);
      document.removeEventListener('visibilitychange', onVisibility);
      unlisten?.();
      void invoke('unsubscribe_log_stream').catch(() => {});
      if (!recording) void invoke('set_diagnostics_monitoring', { enabled: false }).catch(() => {});
    };
  });
</script>

<section class="diagnostics-console" aria-label="Developer diagnostics">
  <div class="head">
    <div class="title">
      <h2 class="settings-h">Developer</h2>
      <span class="live" class:on={snapshot.generated_at_ms > 0}><i></i>{snapshot.generated_at_ms ? `Live · ${clockTime(snapshot.generated_at_ms)}` : 'Connecting'}</span>
    </div>
    <div class="actions">
      {#if status}<span class="export-status" in:fade={{ duration: motionMs(MOTION_MS.fast) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>{status}</span>{/if}
      <button class="btn-ghost btn-compact" class:recording title="Capture detailed per-stage timing for the next dictations" onclick={() => void toggleRecording()}>{recording ? 'Stop profile' : 'Record profile'}</button>
      <button class="btn-primary btn-compact" title="Findings, last pipeline, failures, and recent errors as plain text" onclick={() => void copy(buildReport(snapshot, frontendMetrics, findings), 'Report copied. Paste it to your agent.')}>Copy report</button>
    </div>
  </div>

  <nav class="tabs" aria-label="Diagnostics views">
    {#each views as item (item.id)}
      <button class:active={view === item.id} class="tab" role="tab" data-setting-target={`developer-${item.id}`} aria-selected={view === item.id} aria-current={view === item.id ? 'page' : undefined} onclick={() => (view = item.id)}>
        {item.label}
        {#if item.id === 'failures' && failureCount > 0}<span class="count-pill">{failureCount}</span>{/if}
        {#if view === item.id}<div class="bar" in:receive={{ key: 'diag-tab' }} out:send={{ key: 'diag-tab' }}></div>{/if}
      </button>
    {/each}
  </nav>

  {#key view}
    <div class="view" in:fly={{ y: motionPx(MOTION_PX.lift), duration: motionMs(MOTION_MS.panel), easing: expoOut }}>
      {#if view === 'overview'}
        <OverviewView {snapshot} {findings} {ipcPerMinute} onNavigate={navigate} onDownload={(format) => void download(format)} onClear={() => void clearDiagnostics()} />
      {:else if view === 'pipeline'}
        <PipelineView {snapshot} {audioActive} {pillLabel} selected={selectedTrace} onSelect={(id) => (selectedTrace = id)} onCopy={(text, what) => void copy(text, what)} onLogs={openLogs} />
      {:else if view === 'failures'}
        <FailuresView {snapshot} onCopy={(text, what) => void copy(text, what)} onTrace={openTrace} onLogs={openLogs} />
      {:else if view === 'logs'}
        <LogsView logs={snapshot.logs} bind:query={logQuery} bind:level={logLevel} bind:subsystem={logSubsystem} bind:trace={logTrace} onCopy={(text, what) => void copy(text, what)} />
      {:else if view === 'resources'}
        <ResourcesView {snapshot} frontend={frontendMetrics} />
      {:else}
        <SettingsView />
      {/if}
    </div>
  {/key}
</section>

<style>
  .diagnostics-console { position:relative; isolation:isolate; background:var(--paper); }
  .head { display:flex; justify-content:space-between; align-items:flex-start; gap:16px; flex-wrap:wrap; }
  .title { display:flex; flex-direction:column; gap:4px; }
  .title :global(.settings-h) { margin-bottom:0; }
  .live { display:inline-flex; align-items:center; gap:7px; font-size:11.5px; color:var(--ink-mute); }
  .live i { width:6px; height:6px; border-radius:50%; background:var(--ink-faint); }
  .live.on i { background:var(--success); animation:dx-pulse 2.4s ease-in-out infinite; }
  .actions { display:flex; align-items:center; gap:8px; flex-wrap:wrap; justify-content:flex-end; }
  .export-status { font-size:11.5px; color:var(--ink-mute); max-width:320px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .btn-ghost.recording { color:var(--danger); border-color:var(--danger); }

  .tabs { display:flex; gap:22px; overflow-x:auto; overflow-y:hidden; flex-wrap:nowrap; border-bottom:1px solid var(--line); margin:22px 0 24px; }
  .tab { position:relative; display:inline-flex; align-items:center; gap:6px; background:none; border:0; color:var(--ink-mute); padding:0 0 10px; white-space:nowrap; font:12.5px var(--sans); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) var(--ui-ease-out, ease); }
  .tab:hover { color:var(--ink-soft); }
  .tab.active { color:var(--ink); font-weight:500; }
  .tab:focus-visible { outline:2px solid var(--ink-strong); outline-offset:2px; border-radius:2px; }
  .bar { position:absolute; bottom:-1px; left:0; right:0; height:2px; background:var(--accent); }
  .count-pill { min-width:16px; padding:0 5px; border-radius:99px; background:var(--danger); color:#fff; font:600 10px var(--mono); line-height:16px; text-align:center; }

  .view { min-width:0; }

  /* Shared primitives for the Developer views. */
  .view :global(.dx-label) { display:flex; align-items:center; justify-content:space-between; gap:12px; margin:0 0 10px; font-size:10.5px; text-transform:uppercase; letter-spacing:.07em; color:var(--ink-mute); font-weight:500; }
  .view :global(.dx-label b) { font:400 11px var(--sans); letter-spacing:0; text-transform:none; color:var(--ink-faint); }
  .view :global(.dx-label .link) { background:none; border:0; padding:0; font:12px var(--sans); letter-spacing:0; text-transform:none; color:var(--ink-mute); cursor:pointer; }
  .view :global(.dx-label .link:hover) { color:var(--ink); }
  .view :global(.dx-empty) { margin:0; padding:14px 0; font-size:12.5px; color:var(--ink-mute); }

  @keyframes dx-pulse { 0%, 100% { opacity:1; } 50% { opacity:.35; } }
  @media (prefers-reduced-motion: reduce) { .live.on i { animation:none; } }
</style>
