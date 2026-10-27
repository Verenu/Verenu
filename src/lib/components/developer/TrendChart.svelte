<script lang="ts">
  import { buildSegments, segmentsToPath } from '../../views/insights/chartCurve';

  interface Props {
    points: { at: number; value: number }[];
    format: (value: number) => string;
    label: string;
    /** Fixed ceiling (e.g. 100 for percent). Defaults to the series peak. */
    ceiling?: number;
    /** 'min' zooms the scale to the series range so drift is visible. */
    baseline?: 'zero' | 'min';
    height?: number;
  }

  let { points, format, label, ceiling, baseline = 'zero', height = 72 }: Props = $props();

  const W = 600;
  const PAD = 6;
  const uid = `dx-trend-${Math.random().toString(36).slice(2)}`;
  let hover = $state<number | null>(null);

  const max = $derived(Math.max(1, ceiling ?? 0, ...points.map((p) => p.value)));
  const min = $derived.by(() => {
    if (baseline === 'zero' || points.length === 0) return 0;
    const low = Math.min(...points.map((p) => p.value));
    return Math.max(0, low - (max - low) * 0.15 - max * 0.02);
  });
  const plot = $derived.by(() => {
    if (points.length === 0) return { line: '', area: '', xs: [] as number[] };
    const xs = points.map((_, i) => (points.length === 1 ? W / 2 : (i / (points.length - 1)) * W));
    const ys = points.map((p) => PAD + (height - PAD * 2) * (1 - (p.value - min) / Math.max(1, max - min)));
    const line =
      points.length === 1
        ? `M 0 ${ys[0]} L ${W} ${ys[0]}`
        : segmentsToPath(buildSegments(xs.map((x, i) => ({ x, y: ys[i] })), PAD, height - PAD));
    return { line, area: `${line} L ${W} ${height} L 0 ${height} Z`, xs };
  });

  function move(event: PointerEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    if (rect.width === 0 || points.length === 0) return;
    hover = Math.min(points.length - 1, Math.max(0, Math.round(((event.clientX - rect.left) / rect.width) * (points.length - 1))));
  }

  const active = $derived(hover !== null ? points[hover] : null);
</script>

<div class="trend" style:height="{height}px" role="img" aria-label={label} onpointermove={move} onpointerleave={() => (hover = null)}>
  {#if plot.line}
    <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none">
      <defs>
        <linearGradient id="{uid}-fill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.22" />
          <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
        </linearGradient>
      </defs>
      <path d={plot.area} fill="url(#{uid}-fill)" />
      <path d={plot.line} fill="none" stroke="var(--accent)" stroke-width="1.5" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
      {#if hover !== null}
        <line x1={plot.xs[hover]} x2={plot.xs[hover]} y1="0" y2={height} stroke="var(--ink-faint)" stroke-width="1" stroke-dasharray="3 3" vector-effect="non-scaling-stroke" />
      {/if}
    </svg>
    {#if active}
      <div class="tip"><strong>{format(active.value)}</strong><span>{new Date(active.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}</span></div>
    {/if}
  {:else}
    <span class="empty">Collecting…</span>
  {/if}
</div>

<style>
  .trend { position:relative; animation:dx-reveal 700ms var(--ui-ease-out, ease) both; }
  svg { display:block; width:100%; height:100%; overflow:visible; }
  .tip { position:absolute; top:0; right:0; display:flex; gap:8px; align-items:baseline; padding:3px 8px; border-radius:var(--r-sm); background:var(--bg-elev); border:1px solid var(--line); font-size:11px; color:var(--ink-mute); pointer-events:none; }
  .tip strong { color:var(--ink); font-weight:500; font-variant-numeric:tabular-nums; }
  .empty { display:block; padding-top:calc(50% - 10px); font-size:11px; color:var(--ink-faint); text-align:center; }
  @keyframes dx-reveal { from { clip-path:inset(0 100% 0 0); } to { clip-path:inset(0 0 0 0); } }
  @media (prefers-reduced-motion: reduce) { .trend { animation:none; } }
</style>
