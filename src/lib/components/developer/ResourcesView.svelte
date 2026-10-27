<script lang="ts">
  import { formatBytes, formatDuration, formatRate, runtimeModelLabel, type DiagnosticsSnapshot, type FrontendIpcMetric } from '../../diagnostics';
  import { operationRows } from '../../diagnosticsInsights';
  import TrendChart from './TrendChart.svelte';

  interface Props { snapshot: DiagnosticsSnapshot; frontend: FrontendIpcMetric[] }
  let { snapshot, frontend }: Props = $props();

  let showAllOps = $state(false);

  const resource = $derived(snapshot.current_resource ?? null);
  const samples = $derived(snapshot.resource_samples.slice(-60));
  const memory = $derived(samples.filter((s) => s.snapshot.resident_bytes != null).map((s) => ({ at: s.observed_at_ms, value: s.snapshot.resident_bytes as number })));
  const cpu = $derived(samples.filter((s) => s.snapshot.cpu_percent != null).map((s) => ({ at: s.observed_at_ms, value: s.snapshot.cpu_percent as number })));
  const children = $derived([...(resource?.child_processes ?? [])].sort((a, b) => (b.resident_bytes ?? 0) - (a.resident_bytes ?? 0)));
  const maxChild = $derived(Math.max(1, ...children.map((c) => c.resident_bytes ?? 0)));
  const ops = $derived(operationRows(snapshot, frontend));
  const maxOp = $derived(Math.max(1, ...ops.map((o) => o.total_duration_ms)));
  const shownOps = $derived(showAllOps ? ops.slice(0, 60) : ops.slice(0, 10));

  const engine = $derived(
    [
      ['Local STT', runtimeModelLabel(snapshot.runtime.local_stt)],
      ['Local cleanup', runtimeModelLabel(snapshot.runtime.local_llm)],
      ['Compositor', snapshot.runtime.window_chrome?.compositor],
      ['Cleanup cache', snapshot.runtime.cleanup_cache?.entry_count != null ? `${snapshot.runtime.cleanup_cache.entry_count} entries` : null],
      ['Sync peers', snapshot.runtime.sync?.peer_count != null ? String(snapshot.runtime.sync.peer_count) : null],
    ].filter((row): row is [string, string] => Boolean(row[1])),
  );
</script>

<div class="resources" data-setting-target="developer-resources">
  <section class="charts">
    <div>
      <header class="dx-label"><span>Memory</span><b>{resource?.peak_resident_bytes == null ? '' : `peak ${formatBytes(resource.peak_resident_bytes)}`}</b></header>
      <p class="big">{resource?.resident_bytes == null ? '—' : formatBytes(resource.resident_bytes)}</p>
      <TrendChart points={memory} format={formatBytes} label="Resident memory over time" height={84} baseline="min" />
    </div>
    <div>
      <header class="dx-label"><span>CPU</span><b>{resource?.thread_count ?? '—'} threads</b></header>
      <p class="big">{resource?.cpu_percent == null ? '—' : `${resource.cpu_percent.toFixed(1)}%`}</p>
      <TrendChart points={cpu} format={(v) => `${v.toFixed(1)}%`} label="CPU over time" height={84} />
    </div>
  </section>

  <section>
    <header class="dx-label"><span>Processes</span><b>read {formatRate(resource?.read_bytes_per_sec, '/s')} · write {formatRate(resource?.write_bytes_per_sec, '/s')}</b></header>
    {#each children as child, i (`${child.label}-${child.pid ?? i}`)}
      <div class="proc">
        <span class="name">{child.label}</span>
        <span class="track"><i style:transform="scaleX({(child.resident_bytes ?? 0) / maxChild})" style:--i={i}></i></span>
        <b>{formatBytes(child.resident_bytes)}</b>
        <small>{child.cpu_percent == null ? '' : `${child.cpu_percent.toFixed(1)}%`}</small>
      </div>
    {:else}
      <p class="dx-empty">Single process. No children running.</p>
    {/each}
  </section>

  <section>
    <header class="dx-label"><span>Heaviest operations</span><b>total time</b></header>
    {#if ops.length}
      <div class="ops-head"><span></span><span></span><span>calls</span><span>avg</span><span>p95</span><span>fail</span></div>
      {#each shownOps as op, i (op.operation)}
        <div class="op">
          <code>{op.operation}</code>
          <span class="track"><i style:transform="scaleX({op.total_duration_ms / maxOp})" style:--i={i}></i></span>
          <span>{op.calls}</span>
          <span>{formatDuration(op.average_duration_ms)}</span>
          <span>{formatDuration(op.p95_duration_ms)}</span>
          <span class:bad={op.failure_count > 0}>{op.failure_count || ''}</span>
        </div>
      {/each}
      {#if ops.length > 10}<button class="link" onclick={() => (showAllOps = !showAllOps)}>{showAllOps ? 'Show fewer' : `Show all ${ops.length}`}</button>{/if}
    {:else}
      <p class="dx-empty">No operations recorded yet.</p>
    {/if}
  </section>

  {#if engine.length}
    <section class="engine">
      {#each engine as [label, value] (label)}
        <div><span>{label}</span><b>{value}</b></div>
      {/each}
    </section>
  {/if}
</div>

<style>
  .resources { display:flex; flex-direction:column; gap:30px; }
  .charts { display:grid; grid-template-columns:1fr 1fr; gap:28px; }
  .big { margin:0 0 8px; font:600 24px var(--sans); letter-spacing:-0.02em; color:var(--ink); font-variant-numeric:tabular-nums; }

  .proc { display:grid; grid-template-columns:minmax(90px,170px) minmax(0,1fr) 78px 52px; gap:14px; align-items:center; padding:8px 0; border-bottom:1px solid var(--line-soft); }
  .name, code { font-size:12.5px; color:var(--ink-soft); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  code { font:11.5px var(--mono); }
  .track { height:6px; border-radius:3px; background:var(--line-soft); overflow:hidden; }
  .track i { display:block; height:100%; border-radius:3px; background:var(--ink-mute); transform-origin:left; animation:dx-grow 560ms var(--ui-ease-out, ease) both; animation-delay:calc(var(--i) * 40ms); }
  .proc b { font:500 11.5px var(--mono); color:var(--ink); text-align:right; }
  .proc small { font:10.5px var(--mono); color:var(--ink-mute); text-align:right; }

  .ops-head, .op { display:grid; grid-template-columns:minmax(150px,1.5fr) minmax(60px,1fr) 52px 66px 66px 40px; gap:12px; align-items:center; }
  .ops-head { padding:0 0 6px; font-size:10px; text-transform:uppercase; letter-spacing:.06em; color:var(--ink-faint); }
  .ops-head span, .op > span:not(.track) { text-align:right; }
  .op { padding:7px 0; border-bottom:1px solid var(--line-soft); font:11.5px var(--mono); color:var(--ink-soft); font-variant-numeric:tabular-nums; }
  .bad { color:var(--danger); }
  .link { margin-top:10px; background:none; border:0; padding:2px 0; font:12px var(--sans); color:var(--ink-mute); cursor:pointer; }
  .link:hover { color:var(--ink); }

  .engine { display:flex; flex-wrap:wrap; gap:8px 30px; padding-top:18px; border-top:1px solid var(--line); }
  .engine div { display:flex; flex-direction:column; gap:2px; }
  .engine span { font-size:10.5px; text-transform:uppercase; letter-spacing:.06em; color:var(--ink-mute); }
  .engine b { font:500 12px var(--mono); color:var(--ink); }

  @keyframes dx-grow { from { transform:scaleX(0); } }
  @media (max-width: 760px) {
    .charts { grid-template-columns:1fr; }
    .ops-head, .op { grid-template-columns:minmax(0,1fr) 44px 60px 60px; }
    .ops-head span:nth-child(2), .op .track, .ops-head span:nth-child(6), .op > span:last-child { display:none; }
  }
  @media (prefers-reduced-motion: reduce) { .track i { animation:none; } }
</style>
