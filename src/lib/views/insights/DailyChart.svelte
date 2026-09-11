<script lang="ts">
  import { Spring } from 'svelte/motion';
  import { fmtDayLong, fmtNumber, niceCeiling } from './helpers';
  import { reducedMotionEnabled } from '../../motion';
  import { buildSegments, pointOnSegments, segmentsToPath } from './chartCurve';
  import RollingNumber from './RollingNumber.svelte';
  import ChartTooltip from './ChartTooltip.svelte';
  import type { InsightsDay } from './types';

  let { daily, rangeLabel }: { daily: InsightsDay[]; rangeLabel: string } = $props();

  // Unique per instance so multiple charts on the page never share a <mask> id.
  const gradientId = `daily-edge-fade-${Math.random().toString(36).slice(2)}`;

  /* Fixed viewBox + preserveAspectRatio="none" makes the chart fully fluid with
     no resize observer; vector-effect keeps strokes an even width regardless. */
  const W = 600;
  const H = 170;
  const PAD_TOP = 12;
  const PAD_BOTTOM = 22;

  /* Below this many points a line reads as noise — discrete bars are clearer. */
  const BAR_THRESHOLD = 14;

  // Reduce rather than Math.max(...spread) — an all-time range can exceed the
  // call-stack limit for spread arguments on a very long daily series.
  const max = $derived(niceCeiling(daily.reduce((m, d) => Math.max(m, d.words), 0)));
  const asBars = $derived(daily.length <= BAR_THRESHOLD);
  const plotH = H - PAD_TOP - PAD_BOTTOM;

  function x(i: number): number {
    if (daily.length <= 1) return W / 2;
    // Bars sit in the middle of their slot so the first and last aren't clipped
    // by the viewBox edge; the line spans edge to edge.
    if (asBars) return (i + 0.5) * (W / daily.length);
    return (i / (daily.length - 1)) * W;
  }

  function y(words: number): number {
    return PAD_TOP + plotH * (1 - words / max);
  }

  /* Catmull-Rom control points → cubic bezier, so the area reads as a curve
     without pulling above the data the way a naive spline does. The hover
     indicator reads these same segments, which is what keeps it on the line. */
  const segments = $derived(
    buildSegments(
      daily.map((d, i) => ({ x: x(i), y: y(d.words) })),
      PAD_TOP,
      H - PAD_BOTTOM
    )
  );

  const linePath = $derived.by(() => {
    if (daily.length === 0) return '';
    if (daily.length === 1) return `M 0 ${y(daily[0].words)} L ${W} ${y(daily[0].words)}`;
    return segmentsToPath(segments);
  });

  const areaPath = $derived(
    linePath ? `${linePath} L ${W} ${H - PAD_BOTTOM} L 0 ${H - PAD_BOTTOM} Z` : ''
  );

  const barWidth = $derived(daily.length > 0 ? Math.min(28, (W / daily.length) * 0.55) : 0);

  let hover = $state<number | null>(null);
  let plotWidth = $state(0);
  const active = $derived(hover !== null ? daily[hover] : null);
  /* Held through the fade-out so the tooltip keeps its text on the way out
     instead of blanking the instant the pointer leaves. */
  let lastActive = $state<InsightsDay | null>(null);
  $effect(() => {
    if (active) lastActive = active;
  });

  /* The pointer snaps to whole days, so without this the indicator teleports
     between them. Springing a fractional index — rather than the co-ordinates —
     is what lets the dot ride the curve instead of cutting across it. */
  const cursor = new Spring(0, { stiffness: 0.19, damping: 0.82 });

  const cursorPoint = $derived.by(() => {
    if (daily.length === 0) return { x: 0, y: H - PAD_BOTTOM };
    const at = Math.max(0, Math.min(daily.length - 1, cursor.current));
    // Bars have no curve to ride, so the indicator slides straight across.
    if (asBars || segments.length === 0) {
      const i = Math.floor(at);
      const next = daily[Math.min(daily.length - 1, i + 1)];
      const y0 = y(daily[i].words);
      return { x: x(at), y: y0 + (y(next.words) - y0) * (at - i) };
    }
    return pointOnSegments(segments, at);
  });

  // The svg's CSS height matches the viewBox height 1:1 and only its width is
  // fluid, so x scales by the rendered width and y needs no conversion.
  const cursorLeft = $derived((cursorPoint.x / W) * plotWidth);

  function onMove(event: PointerEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    if (rect.width === 0 || daily.length === 0) return;
    plotWidth = rect.width;
    const ratio = (event.clientX - rect.left) / rect.width;
    // Bars fill equal-width slots spanning [i/n, (i+1)/n), so floor maps the
    // pointer into its slot; line charts align points at i/(n-1), so round
    // to the nearest point.
    const idx = asBars
      ? Math.min(daily.length - 1, Math.max(0, Math.floor(ratio * daily.length)))
      : Math.min(daily.length - 1, Math.max(0, Math.round(ratio * (daily.length - 1))));
    // Arriving from outside, place the indicator under the pointer rather than
    // sliding it in from wherever it was last left — otherwise entering the
    // card drags a line across the whole chart to meet you.
    const entering = hover === null;
    hover = idx;
    cursor.set(idx, { instant: entering || reducedMotionEnabled() });
  }

  const total = $derived(daily.reduce((sum, d) => sum + d.words, 0));
  const best = $derived(daily.reduce<InsightsDay | null>((b, d) => (!b || d.words > b.words ? d : b), null));
  const summary = $derived(
    daily.length === 0
      ? 'No daily activity in this range.'
      : `Words dictated per day, ${rangeLabel.toLowerCase()}. ${fmtNumber(total)} words across ${daily.length} days, peaking at ${fmtNumber(best?.words ?? 0)} on ${best ? fmtDayLong(best.day) : '—'}.`
  );
</script>

<section class="card">
  <header class="card-head">
    <div>
      <h2 class="card-h">Words per day</h2>
      <p class="card-sub">{rangeLabel}</p>
    </div>
    <div class="readout" aria-live="polite">
      <span class="readout-num"><RollingNumber value={active ? active.words : total} format={fmtNumber} /></span>
      <span class="readout-day">{active ? fmtDayLong(active.day) : 'total'}</span>
    </div>
  </header>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="plot"
    role="img"
    aria-label={summary}
    onpointermove={onMove}
    onpointerleave={() => (hover = null)}
  >
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none">
      <defs>
        <!-- Fades the area fill's hard vertical edges instead of cutting it off flush. -->
        <linearGradient id={gradientId} x1="0" x2="1" y1="0" y2="0">
          <stop offset="0%" stop-color="white" stop-opacity="0" />
          <stop offset="4%" stop-color="white" stop-opacity="1" />
          <stop offset="96%" stop-color="white" stop-opacity="1" />
          <stop offset="100%" stop-color="white" stop-opacity="0" />
        </linearGradient>
        <mask id="{gradientId}-mask" maskUnits="userSpaceOnUse" x="0" y="0" width={W} height={H}>
          <rect x="0" y="0" width={W} height={H} fill="url(#{gradientId})" />
        </mask>
      </defs>
      <line
        x1="0" y1={H - PAD_BOTTOM} x2={W} y2={H - PAD_BOTTOM}
        stroke="var(--line)" stroke-width="1" vector-effect="non-scaling-stroke"
      />
      {#if asBars}
        {#each daily as d, i}
          <rect
            x={x(i) - barWidth / 2}
            y={d.words > 0 ? y(d.words) : H - PAD_BOTTOM - 1}
            width={barWidth}
            height={d.words > 0 ? Math.max(1, H - PAD_BOTTOM - y(d.words)) : 1}
            fill={hover === i ? 'var(--accent)' : 'color-mix(in srgb, var(--accent) 55%, transparent)'}
          ><title>{fmtDayLong(d.day)} — {fmtNumber(d.words)} words</title></rect>
        {/each}
      {:else}
        <path d={areaPath} fill="color-mix(in srgb, var(--accent) 18%, transparent)" mask="url(#{gradientId}-mask)" class="area-path" />
        <path
          d={linePath}
          fill="none"
          stroke="var(--accent)"
          stroke-width="1.5"
          stroke-linejoin="round"
          vector-effect="non-scaling-stroke"
          class="line-path"
        />
      {/if}
      {#if daily.length > 0}
        <line
          class="cursor-line"
          class:on={hover !== null}
          x1={cursorPoint.x} y1={PAD_TOP - 6} x2={cursorPoint.x} y2={H - PAD_BOTTOM}
          stroke="var(--accent)" stroke-width="1" stroke-dasharray="3 3"
          vector-effect="non-scaling-stroke"
        />
      {/if}
    </svg>
    {#if !asBars && daily.length > 0}
      <!-- Positioned as a percentage so it tracks a resize without remeasuring;
           only the tooltip needs the plot's pixel width. -->
      <span
        class="hover-dot"
        class:on={hover !== null}
        style:left="{(cursorPoint.x / W) * 100}%"
        style:top="{cursorPoint.y}px"
        aria-hidden="true"
      ></span>
    {/if}
    <div class="axis">
      <span>{daily.length ? fmtDayLong(daily[0].day) : ''}</span>
      <span>{daily.length > 1 ? fmtDayLong(daily[daily.length - 1].day) : ''}</span>
    </div>
    {#if lastActive}
      <ChartTooltip x={cursorLeft} y={cursorPoint.y} visible={hover !== null}>
        <strong>{fmtNumber(lastActive.words)}</strong> words
        <div class="tooltip-dim">{fmtDayLong(lastActive.day)}</div>
      </ChartTooltip>
    {/if}
  </div>
</section>

<style>
  /* .card / .card-head / .card-h / .card-sub are owned by Insights.svelte. */

  .readout {
    text-align: right;
    white-space: nowrap;
    min-width: 0;
    flex-shrink: 1;
  }
  .readout-num {
    display: block;
    font-family: var(--sans);
    font-size: 20px;
    font-weight: 500;
    color: var(--ink);
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .readout-day {
    font-size: 11px;
    color: var(--ink-mute);
  }

  .plot { flex: 1; position: relative; }

  svg {
    display: block;
    width: 100%;
    height: 170px;
    overflow: visible;
  }

  .axis {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 10.5px;
    color: var(--ink-mute);
    margin-top: 2px;
    min-width: 0;
  }

  .axis span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .axis span:last-child { text-align: right; }

  rect { transition: fill var(--ui-duration-fast) var(--ui-ease-out), y var(--ui-duration-base) var(--ui-ease-out), height var(--ui-duration-base) var(--ui-ease-out); }

  /* Progressive enhancement: browsers that support animating `d` glide to a
     new shape when the range or a fresh dictation changes the data. */
  .area-path,
  .line-path {
    transition: d var(--ui-duration-base) var(--ui-ease-out);
  }

  .line-path {
    animation: daily-line-in 260ms var(--ui-ease-out) both;
  }

  @keyframes daily-line-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (prefers-reduced-motion: reduce) {
    .line-path { animation: none; }
  }

  /* Only opacity and scale are transitioned. Position is driven by the spring
     every frame, so a transition on left/top would fight it and lag behind. */
  .cursor-line {
    opacity: 0;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out);
  }

  .cursor-line.on {
    opacity: 0.55;
  }

  .hover-dot {
    position: absolute;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--accent);
    border: 2.5px solid var(--bg-elev);
    box-sizing: border-box;
    transform: translate(-50%, -50%) scale(0.5);
    opacity: 0;
    pointer-events: none;
    filter: drop-shadow(0 1px 3px color-mix(in srgb, var(--accent) 55%, transparent));
    transition:
      opacity var(--ui-duration-fast) var(--ui-ease-out),
      transform var(--ui-duration-base) var(--ui-ease-out);
  }

  .hover-dot.on {
    opacity: 1;
    transform: translate(-50%, -50%) scale(1);
  }
</style>
