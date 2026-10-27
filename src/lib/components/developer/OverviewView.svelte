<script lang="ts">
  import { fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { formatBytes, formatDuration, formatRate, type DiagnosticsSnapshot } from '../../diagnostics';
  import { completedTraces, stageLabel, stageStats, verdictFor, type Finding, type FindingView } from '../../diagnosticsInsights';
  import TrendChart from './TrendChart.svelte';

  interface Props {
    snapshot: DiagnosticsSnapshot;
    findings: Finding[];
    ipcPerMinute: number;
    onNavigate: (view: FindingView) => void;
    onDownload: (format: 'json' | 'text') => void;
    onClear: () => void;
  }

  let { snapshot, findings, ipcPerMinute, onNavigate, onDownload, onClear }: Props = $props();

  const VERDICT = {
    waiting: 'Connecting',
    healthy: 'All clear',
    attention: 'Needs attention',
    failing: 'Failing',
  } as const;

  const VISIBLE = 5;
  let showAll = $state(false);
  const shown = $derived(showAll ? findings : findings.slice(0, VISIBLE));

  const verdict = $derived(verdictFor(snapshot, findings));
  const traces = $derived(completedTraces(snapshot));
  const last = $derived(traces[traces.length - 1] ?? null);
  const stats = $derived(stageStats(traces));
  const resource = $derived(snapshot.current_resource ?? null);
  const memory = $derived(
    snapshot.resource_samples
      .filter((s) => s.snapshot.resident_bytes != null)
      .slice(-60)
      .map((s) => ({ at: s.observed_at_ms, value: s.snapshot.resident_bytes as number })),
  );

  function fadeTone(rank: number): string {
    return `color-mix(in srgb, var(--ink) ${Math.max(18, 62 - rank * 14)}%, transparent)`;
  }
</script>

<div class="overview">
  <div class="verdict" data-verdict={verdict}>
    <span class="dot"></span>
    <strong>{VERDICT[verdict]}</strong>
    {#if last}
      <span class="sep">·</span>
      <span class="sub">Last dictation {formatDuration(last.duration_ms)}</span>
      <em class:bad={last.outcome === 'failure'}>{last.outcome}</em>
    {/if}
  </div>

  <section>
    <header class="dx-label"><span>Findings</span><b>{findings.length}</b></header>
    {#if findings.length}
      <ul class="findings">
        {#each shown as finding, i (finding.id)}
          <li in:fly={{ y: motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), delay: Math.min(i, 8) * 40, easing: expoOut }}>
            <button class="finding" data-severity={finding.severity} onclick={() => onNavigate(finding.view)}>
              <span class="rail"></span>
              <span class="body">
                <span class="title">{finding.title}</span>
                <span class="detail">{finding.detail}</span>
              </span>
              <span class="metric">{finding.metric}</span>
              <svg class="chev" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6" /></svg>
            </button>
          </li>
        {/each}
      </ul>
      {#if findings.length > VISIBLE}
        <button class="link more" onclick={() => (showAll = !showAll)}>{showAll ? 'Show fewer' : `Show ${findings.length - VISIBLE} more`}</button>
      {/if}
    {:else}
      <p class="dx-empty">Nothing is slow, failing, or wasting memory.</p>
    {/if}
  </section>

  <section>
    <header class="dx-label"><span>Where time goes</span><b>{stats.length ? `avg of ${traces.length} runs` : ''}</b></header>
    {#if stats.length}
      <div class="stack" role="img" aria-label="Average time per pipeline stage">
        {#each stats as stat, i (stat.stage)}
          <span class="seg" class:hot={i === 0 && stat.share >= 0.5} style:flex-grow={stat.share} style:--tone={fadeTone(i)} style:--i={i} title="{stageLabel(stat.stage)} {formatDuration(stat.avgMs)}"></span>
        {/each}
      </div>
      <ul class="legend">
        {#each stats as stat, i (stat.stage)}
          <li>
            <i style:background={i === 0 && stat.share >= 0.5 ? 'var(--warning)' : fadeTone(i)}></i>
            <span>{stageLabel(stat.stage)}</span>
            <b>{formatDuration(stat.avgMs)}</b>
            <small>{Math.round(stat.share * 100)}%</small>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dx-empty">Dictate once to see the breakdown.</p>
    {/if}
  </section>

  <section class="vitals">
    <div class="vital grow">
      <span>Memory</span>
      <strong>{resource?.resident_bytes == null ? '—' : formatBytes(resource.resident_bytes)}</strong>
      <TrendChart points={memory} format={formatBytes} label="Resident memory over time" height={44} baseline="min" />
    </div>
    <div class="vital"><span>CPU</span><strong>{resource?.cpu_percent == null ? '—' : `${resource.cpu_percent.toFixed(1)}%`}</strong></div>
    <div class="vital"><span>Processes</span><strong>{resource?.process_count ?? '—'}</strong></div>
    <div class="vital"><span>IPC</span><strong>{formatRate(ipcPerMinute, '/min')}</strong></div>
  </section>

  <footer class="tail">
    <button class="link" onclick={() => onDownload('json')}>Download bundle</button>
    <span data-setting-target="developer-download-logs"><button class="link" onclick={() => onDownload('text')}>Download Logs</button></span>
    <button class="link danger" onclick={onClear}>Clear data</button>
    <span class="privacy">Metadata only. No dictated text.</span>
  </footer>
</div>

<style>
  .overview { display:flex; flex-direction:column; gap:28px; }

  .verdict { display:flex; align-items:baseline; gap:10px; font-size:13px; color:var(--ink-mute); }
  .verdict strong { font-size:22px; font-weight:600; letter-spacing:-0.02em; color:var(--ink); }
  .verdict .dot { width:8px; height:8px; border-radius:50%; background:var(--ink-faint); align-self:center; transition:background-color var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); }
  .verdict[data-verdict="healthy"] .dot { background:var(--success); }
  .verdict[data-verdict="attention"] .dot { background:var(--warning); }
  .verdict[data-verdict="failing"] .dot { background:var(--danger); animation:dx-pulse 1.6s ease-in-out infinite; }
  .verdict[data-verdict="failing"] strong { color:var(--danger); }
  .sep { color:var(--ink-faint); }
  em { font-style:normal; color:var(--success); font-size:12px; }
  em.bad { color:var(--danger); }

  .findings { list-style:none; margin:0; padding:0; border-top:1px solid var(--line-soft); }
  .findings li { border-bottom:1px solid var(--line-soft); }
  .finding { all:unset; box-sizing:border-box; width:100%; display:grid; grid-template-columns:3px minmax(0,1fr) auto 14px; gap:14px; align-items:center; padding:12px 8px 12px 0; cursor:pointer; border-radius:var(--r-sm); transition:background var(--ui-duration-fast, 150ms) ease; }
  .finding:hover { background:var(--control-hover); }
  .finding:focus-visible { outline:2px solid var(--ink-strong); outline-offset:-2px; }
  .rail { align-self:stretch; border-radius:2px; background:var(--ink-faint); transition:transform var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); transform-origin:center; }
  .finding:hover .rail { transform:scaleX(1.8); }
  .finding[data-severity="bad"] .rail { background:var(--danger); }
  .finding[data-severity="warn"] .rail { background:var(--warning); }
  .body { display:flex; flex-direction:column; gap:2px; min-width:0; }
  .title { font-size:13px; font-weight:500; color:var(--ink); }
  .detail { font-size:11.5px; color:var(--ink-mute); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .metric { font:500 13px var(--mono); color:var(--ink); font-variant-numeric:tabular-nums; white-space:nowrap; }
  .chev { color:var(--ink-faint); transition:transform var(--ui-duration-fast, 150ms) var(--ui-ease-out, ease), color var(--ui-duration-fast, 150ms) ease; }
  .finding:hover .chev { transform:translateX(2px); color:var(--ink-mute); }

  .stack { display:flex; gap:2px; height:10px; }
  .seg { flex-basis:0; min-width:3px; border-radius:3px; background:var(--tone); transform-origin:left; animation:dx-grow 520ms var(--ui-ease-out, ease) both; animation-delay:calc(var(--i) * 60ms); }
  .seg.hot { background:var(--warning); }
  .legend { list-style:none; margin:12px 0 0; padding:0; display:flex; flex-wrap:wrap; gap:6px 22px; }
  .legend li { display:flex; align-items:center; gap:7px; font-size:12px; color:var(--ink-soft); }
  .legend i { width:7px; height:7px; border-radius:2px; }
  .legend b { font:500 11.5px var(--mono); color:var(--ink); }
  .legend small { font:10.5px var(--mono); color:var(--ink-mute); }

  .vitals { display:grid; grid-template-columns:minmax(0,2.2fr) repeat(3,minmax(0,1fr)); gap:22px; padding-top:20px; border-top:1px solid var(--line); }
  .vital { display:flex; flex-direction:column; gap:4px; min-width:0; }
  .vital > span { font-size:10.5px; text-transform:uppercase; letter-spacing:.06em; color:var(--ink-mute); }
  .vital > strong { font:600 20px var(--sans); letter-spacing:-0.01em; color:var(--ink); font-variant-numeric:tabular-nums; }
  .vital.grow :global(.trend) { margin-top:6px; }

  .tail { display:flex; align-items:center; gap:18px; flex-wrap:wrap; }
  .link { background:none; border:0; padding:2px 0; font:12px var(--sans); color:var(--ink-mute); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) ease; }
  .link:hover { color:var(--ink); }
  .link.danger:hover { color:var(--danger); }
  .more { margin-top:8px; }
  .privacy { margin-left:auto; font-size:11px; color:var(--ink-faint); }

  @keyframes dx-grow { from { transform:scaleX(0); opacity:0; } to { transform:scaleX(1); opacity:1; } }
  @keyframes dx-pulse { 0%, 100% { opacity:1; } 50% { opacity:.35; } }

  @media (max-width: 760px) {
    .vitals { grid-template-columns:repeat(3,minmax(0,1fr)); }
    .vital.grow { grid-column:1 / -1; }
    .privacy { margin-left:0; width:100%; }
  }
  @media (prefers-reduced-motion: reduce) { .seg, .verdict .dot { animation:none !important; } }
</style>
