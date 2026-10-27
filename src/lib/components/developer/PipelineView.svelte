<script lang="ts">
  import { slide } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../../motion';
  import { formatDuration, spanWidth, type DiagnosticsSnapshot, type PipelineTrace } from '../../diagnostics';
  import { clockTime, completedTraces, formatTrace, stageDurations, stageLabel, stageStats } from '../../diagnosticsInsights';

  interface Props {
    snapshot: DiagnosticsSnapshot;
    audioActive: boolean;
    pillLabel: string;
    selected: string | null;
    onSelect: (id: string | null) => void;
    onCopy: (text: string, what: string) => void;
    onLogs: (traceId: string) => void;
  }

  let { snapshot, audioActive, pillLabel, selected, onSelect, onCopy, onLogs }: Props = $props();

  const STAGES: { id: string | null; label: string }[] = [
    { id: null, label: 'Mic' },
    { id: 'capture', label: 'Capture' },
    { id: 'context', label: 'Context' },
    { id: 'vad', label: 'VAD' },
    { id: 'transcription', label: 'Transcribe' },
    { id: 'cleanup', label: 'Cleanup' },
    { id: 'injection', label: 'Insert' },
  ];

  const stats = $derived(stageStats(completedTraces(snapshot)));
  const maxAvg = $derived(Math.max(1, ...stats.map((s) => s.avgMs)));
  const runs = $derived.by(() => {
    const recent = snapshot.recent_pipelines.slice(-12);
    const pinned = selected ? snapshot.recent_pipelines.find((t) => t.trace_id === selected) : undefined;
    const withPinned = pinned && !recent.includes(pinned) ? [pinned, ...recent] : recent;
    return [...snapshot.active_pipelines, ...[...withPinned].reverse()];
  });
  const live = $derived(snapshot.active_pipelines[0] ?? null);
  const audio = $derived(snapshot.runtime.audio ?? null);

  function liveState(stage: string | null): 'idle' | 'active' | 'done' | 'error' {
    if (stage === null) return audioActive ? 'active' : live ? 'done' : 'idle';
    const span = live ? [...live.spans].reverse().find((s) => s.stage === stage) : undefined;
    if (!span) return 'idle';
    if (span.outcome === 'running') return 'active';
    return span.outcome === 'failure' ? 'error' : 'done';
  }

  function bottleneck(trace: PipelineTrace): string | null {
    const top = [...stageDurations(trace)].sort((a, b) => b[1] - a[1])[0];
    return top && trace.duration_ms && top[1] / trace.duration_ms >= 0.4 ? top[0] : null;
  }

  function offset(trace: PipelineTrace, startedAt: number): number {
    const total = trace.duration_ms ?? trace.spans.reduce((end, s) => Math.max(end, (s.ended_at_ms ?? s.started_at_ms) - trace.started_at_ms), 0);
    return total > 0 ? Math.min(99, Math.max(0, ((startedAt - trace.started_at_ms) / total) * 100)) : 0;
  }
</script>

<div class="pipeline" data-setting-target="developer-pipeline">
  <section class="live" aria-label="Live pipeline">
    <div class="flow">
      {#each STAGES as node (node.label)}
        <span class="node" data-state={liveState(node.id)}><i></i>{node.label}</span>
      {/each}
    </div>
    <span class="pill" data-state={audioActive ? 'active' : live ? 'active' : 'idle'}>{pillLabel}</span>
  </section>
  {#if audio && audioActive}
    <p class="audio">
      <span>Level <b>{audio.processed_level?.toFixed(3) ?? '—'}</b></span>
      <span>Gate <b>{audio.gate_rms?.toFixed(3) ?? '—'}</b></span>
      <span class:bad={audio.would_pass_gate === false}>{audio.would_pass_gate === false ? 'Below gate' : audio.speech_detected ? 'Speech' : 'Quiet'}</span>
      <span>Gain <b>×{audio.microphone_gain?.toFixed(2) ?? '—'}</b></span>
      {#if audio.stream_error}<span class="bad">Stream error</span>{/if}
    </p>
  {/if}

  <section>
    <header class="dx-label"><span>Stage timing</span><b>avg · p95 · max</b></header>
    {#if stats.length}
      <ul class="stages">
        {#each stats as stat, i (stat.stage)}
          <li>
            <span class="name">{stageLabel(stat.stage)}</span>
            <span class="track"><i class:hot={i === 0 && stat.share >= 0.5} style:transform="scaleX({stat.avgMs / maxAvg})" style:--i={i}></i></span>
            <b>{formatDuration(stat.avgMs)}</b>
            <small>{formatDuration(stat.p95Ms)}</small>
            <small>{formatDuration(stat.maxMs)}</small>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dx-empty">Dictate once to capture timing.</p>
    {/if}
  </section>

  <section>
    <header class="dx-label"><span>Recent runs</span><b>{runs.length}</b></header>
    {#each runs as trace (trace.trace_id)}
      {@const open = selected === trace.trace_id}
      {@const hot = bottleneck(trace)}
      <div class="run" class:open>
        <button class="run-head" aria-expanded={open} onclick={() => onSelect(open ? null : trace.trace_id)}>
          <time>{clockTime(trace.started_at_ms)}</time>
          <span class="mini" aria-hidden="true">
            {#each [...stageDurations(trace)] as [stage, ms] (stage)}
              <i class:hot={stage === hot} style:flex-grow={ms}></i>
            {/each}
          </span>
          <span class="hotname">{hot ? stageLabel(hot) : ''}</span>
          <b>{formatDuration(trace.duration_ms)}</b>
          <em class:bad={trace.outcome === 'failure'} class:running={trace.ended_at_ms == null}>{trace.ended_at_ms == null ? 'running' : trace.outcome}</em>
        </button>
        {#if open}
          <div class="detail" transition:slide={{ duration: motionMs(MOTION_MS.panel), easing: expoOut }}>
            {#each trace.spans as span (span.span_id)}
              <div class="span">
                <span class="name">{stageLabel(span.stage ?? span.operation)}</span>
                <span class="wf">
                  <i class:failed={span.outcome === 'failure'} class:hot={span.stage === hot} style:left="{offset(trace, span.started_at_ms)}%" style:width="{Math.max(1, spanWidth(span.duration_ms, trace.duration_ms))}%"></i>
                </span>
                <b>{formatDuration(span.duration_ms)}</b>
                <small>
                  {[span.provider, span.model, span.cache && `cache ${span.cache}`, (span.retry_number ?? 0) > 0 && `retry ${span.retry_number}`, (span.fallback_number ?? 0) > 0 && `fallback ${span.fallback_number}`, span.error_category].filter(Boolean).join(' · ')}
                </small>
              </div>
            {/each}
            <div class="acts">
              <button class="link" onclick={() => onCopy(formatTrace(trace), 'Trace copied')}>Copy trace</button>
              <button class="link" onclick={() => onLogs(trace.trace_id)}>Logs for this run</button>
              <code>{trace.trace_id}</code>
            </div>
          </div>
        {/if}
      </div>
    {:else}
      <p class="dx-empty">No runs yet.</p>
    {/each}
  </section>
</div>

<style>
  .pipeline { display:flex; flex-direction:column; gap:26px; }

  .live { display:flex; align-items:center; justify-content:space-between; gap:12px; }
  .flow { display:flex; flex-wrap:wrap; gap:4px 16px; }
  .node { display:inline-flex; align-items:center; gap:7px; font-size:12px; color:var(--ink-faint); transition:color var(--ui-duration-base, 220ms) var(--ui-ease-out, ease); }
  .node i { width:6px; height:6px; border-radius:50%; background:currentColor; transition:background-color var(--ui-duration-base, 220ms) ease; }
  .node[data-state="done"] { color:var(--ink-mute); }
  .node[data-state="active"] { color:var(--success); }
  .node[data-state="active"] i { animation:dx-pulse 1.1s ease-in-out infinite; }
  .node[data-state="error"] { color:var(--danger); }
  .pill { font:500 11px var(--mono); color:var(--ink-mute); text-transform:capitalize; }
  .pill[data-state="active"] { color:var(--success); }
  .audio { display:flex; flex-wrap:wrap; gap:4px 18px; margin:-14px 0 0; font-size:11.5px; color:var(--ink-mute); }
  .audio b { font-family:var(--mono); font-weight:500; color:var(--ink); }
  .bad { color:var(--danger); }

  .stages { list-style:none; margin:0; padding:0; }
  .stages li { display:grid; grid-template-columns:100px minmax(0,1fr) 70px 70px 70px; gap:14px; align-items:center; padding:8px 0; border-bottom:1px solid var(--line-soft); }
  .name { font-size:12.5px; color:var(--ink-soft); }
  .track { height:6px; border-radius:3px; background:var(--line-soft); overflow:hidden; }
  .track i { display:block; height:100%; border-radius:3px; background:var(--ink-mute); transform-origin:left; animation:dx-grow 560ms var(--ui-ease-out, ease) both; animation-delay:calc(var(--i) * 60ms); }
  .track i.hot { background:var(--warning); }
  .stages b, .stages small, .span b, .run-head b { font:500 11.5px var(--mono); color:var(--ink); text-align:right; font-variant-numeric:tabular-nums; }
  .stages small { color:var(--ink-mute); font-weight:400; }

  .run { border-bottom:1px solid var(--line-soft); }
  .run-head { all:unset; box-sizing:border-box; width:100%; display:grid; grid-template-columns:78px minmax(0,1fr) 92px 74px 64px; gap:14px; align-items:center; padding:10px 6px; cursor:pointer; border-radius:var(--r-sm); transition:background var(--ui-duration-fast, 150ms) ease; }
  .run-head:hover, .run.open .run-head { background:var(--control-hover); }
  .run-head:focus-visible { outline:2px solid var(--ink-strong); outline-offset:-2px; }
  time { font:11px var(--mono); color:var(--ink-mute); }
  .mini { display:flex; gap:2px; height:6px; }
  .mini i { flex-basis:0; min-width:2px; border-radius:2px; background:var(--ink-faint); }
  .mini i.hot { background:var(--warning); }
  .hotname { font-size:11px; color:var(--ink-mute); text-align:right; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  em { font-style:normal; font-size:11px; color:var(--success); text-align:right; }
  em.bad { color:var(--danger); }
  em.running { color:var(--ink-mute); }

  .detail { padding:4px 6px 14px; }
  .span { display:grid; grid-template-columns:100px minmax(0,1fr) 74px minmax(0,1.1fr); gap:14px; align-items:center; padding:5px 0; }
  .wf { position:relative; height:6px; border-radius:3px; background:var(--line-soft); }
  .wf i { position:absolute; top:0; bottom:0; border-radius:3px; background:var(--ink-mute); min-width:2px; transform-origin:left; animation:dx-grow 480ms var(--ui-ease-out, ease) both; }
  .wf i.hot { background:var(--warning); }
  .wf i.failed { background:var(--danger); }
  .span small { font-size:10.5px; color:var(--ink-mute); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .acts { display:flex; align-items:center; gap:18px; margin-top:8px; }
  .acts code { margin-left:auto; font:10px var(--mono); color:var(--ink-faint); }
  .link { background:none; border:0; padding:2px 0; font:12px var(--sans); color:var(--ink-mute); cursor:pointer; transition:color var(--ui-duration-fast, 150ms) ease; }
  .link:hover { color:var(--ink); }

  @keyframes dx-grow { from { transform:scaleX(0); } }
  @keyframes dx-pulse { 0%, 100% { opacity:1; } 50% { opacity:.35; } }

  @media (max-width: 760px) {
    .stages li { grid-template-columns:80px minmax(0,1fr) 62px; }
    .stages small { display:none; }
    .run-head { grid-template-columns:70px minmax(0,1fr) 64px 56px; }
    .hotname { display:none; }
    .span { grid-template-columns:80px minmax(0,1fr) 64px; }
    .span small { display:none; }
  }
  @media (prefers-reduced-motion: reduce) { .track i, .wf i, .node i { animation:none !important; } }
</style>
