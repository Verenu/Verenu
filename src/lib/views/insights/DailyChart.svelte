<script lang="ts">
  import { Spring } from 'svelte/motion';
  import { fmtDayLong, fmtNumber, niceCeiling } from './helpers';
  import { reducedMotionEnabled } from '../../motion';
  import { buildSegments, pointOnSegments, segmentsToPath } from './chartCurve';
  import RollingNumber from './RollingNumber.svelte';
  import ChartTooltip from './ChartTooltip.svelte';
  import type { InsightsDay } from './types';
  import { activityDescription, activityLabel, activityRangeDescription, activitySourceInfo, alignActivity, activityPath, type GithubSnapshot } from './github';

  let { daily, rangeLabel, github = null }: { daily: InsightsDay[]; rangeLabel: string; github?: GithubSnapshot | null } = $props();

  const gradientId = `daily-edge-fade-${Math.random().toString(36).slice(2)}`;

  const W = 600;
  const H = 170;
  const PAD_TOP = 12;
  const PAD_BOTTOM = 22;

  const BAR_THRESHOLD = 14;

  const max = $derived(niceCeiling(daily.reduce((m, d) => Math.max(m, d.words), 0)));
  const asBars = $derived(daily.length <= BAR_THRESHOLD);
  const plotH = H - PAD_TOP - PAD_BOTTOM;
  const activity = $derived(alignActivity(daily, github));
  const activityMax = $derived(Math.max(1, niceCeiling(activity.reduce<number>((m, count) => Math.max(m, count ?? 0), 0))));
  const activityTotal = $derived(activity.reduce<number>((sum, count) => sum + (count ?? 0), 0));
  const knownActivityTotal = $derived(activity.some(count => count !== null) ? activityTotal : null);
  const activityY = (count: number) => PAD_TOP + plotH * (1 - count / activityMax);
  const activityBounds = { top: PAD_TOP, bottom: H - PAD_BOTTOM };
  const githubPath = $derived(activityPath(activity, x, activityY, activityBounds));
  const githubArea = $derived(activityPath(activity, x, activityY, activityBounds, true));

  function describeActivity(count: number | null | undefined): string {
    if (!github) return 'GitHub activity unavailable';
    return activityDescription(count, github);
  }

  function onKey(event: KeyboardEvent) {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowUp' ? 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? -1
      : 0;
    if ((step === 0 && event.key !== 'Home' && event.key !== 'End') || !daily.length) return;
    event.preventDefault();
    hover = event.key === 'Home' ? 0 : event.key === 'End' ? daily.length - 1
      : Math.max(0, Math.min(daily.length - 1, (hover ?? 0) + step));
    plotWidth = (event.currentTarget as HTMLElement).clientWidth;
    cursor.set(hover, { instant: true });
  }

  function x(i: number): number {
    if (daily.length <= 1) return W / 2;
    if (asBars) return (i + 0.5) * (W / daily.length);
    return (i / (daily.length - 1)) * W;
  }

  function y(words: number): number {
    return PAD_TOP + plotH * (1 - words / max);
  }

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
  const selectedIndex = $derived(Math.min(hover ?? 0, Math.max(0, daily.length - 1)));
  const selectedDay = $derived(daily[selectedIndex]);
  const active = $derived(hover !== null ? daily[hover] : null);
  let lastActive = $state<InsightsDay | null>(null);
  const activeActivity = $derived(lastActive ? activity[daily.findIndex(d => d.day === lastActive?.day)] : null);
  $effect(() => {
    if (active) lastActive = active;
  });

  const cursor = new Spring(0, { stiffness: 0.19, damping: 0.82 });

  const cursorPoint = $derived.by(() => {
    if (daily.length === 0) return { x: 0, y: H - PAD_BOTTOM };
    const at = Math.max(0, Math.min(daily.length - 1, cursor.current));
    if (asBars || segments.length === 0) {
      const i = Math.floor(at);
      const next = daily[Math.min(daily.length - 1, i + 1)];
      const y0 = y(daily[i].words);
      return { x: x(at), y: y0 + (y(next.words) - y0) * (at - i) };
    }
    return pointOnSegments(segments, at);
  });

  const cursorLeft = $derived((cursorPoint.x / W) * plotWidth);

  let plotRect: DOMRect | null = null;

  function onMove(event: PointerEvent) {
    if (hover === null || !plotRect) {
      plotRect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    }
    const rect = plotRect;
    if (rect.width === 0 || daily.length === 0) return;
    plotWidth = rect.width;
    const ratio = (event.clientX - rect.left) / rect.width;
    const idx = asBars
      ? Math.min(daily.length - 1, Math.max(0, Math.floor(ratio * daily.length)))
      : Math.min(daily.length - 1, Math.max(0, Math.round(ratio * (daily.length - 1))));
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

  {#if github}
    <div class="chart-legend">
      <span class="legend-item"><span class="legend-line" aria-hidden="true"></span>Words</span>
      <span class="legend-item"><span class="legend-line commits" aria-hidden="true"></span>{activityLabel(github)}</span>
      <details class="scale-details">
        <summary class="ui-focus-ring" aria-label="Chart scale details" title="Chart scale details">ⓘ</summary>
        <p>Each line uses its own scale to compare trends: words 0–{fmtNumber(max)}, {activityLabel(github).toLowerCase()} 0–{fmtNumber(activityMax)}. {activitySourceInfo(github)}</p>
      </details>
    </div>
  {/if}

  <div
    class="plot"
    role="slider"
    aria-valuemin={0}
    aria-valuemax={Math.max(0, daily.length - 1)}
    aria-valuenow={selectedIndex}
    aria-valuetext={selectedDay ? `${fmtDayLong(selectedDay.day)}: ${fmtNumber(selectedDay.words)} words${github ? `, ${describeActivity(activity[selectedIndex])}` : ''}` : undefined}
    aria-label={summary + (github ? ` ${activityRangeDescription(knownActivityTotal, activity, github)}. Lines use independent scales to compare trends.` : '') + ' Use arrow keys to explore each day.'}
    tabindex={daily.length ? 0 : undefined}
    onkeydown={onKey}
    onblur={() => { hover = null; }}
    onpointermove={onMove}
    onpointerleave={() => { hover = null; plotRect = null; }}
  >
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none">
      <defs>
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
      {#if github}
        <path d={githubArea} fill="color-mix(in srgb, var(--success) 10%, transparent)" mask="url(#{gradientId}-mask)" />
        <path d={githubPath} fill="none" stroke="var(--success)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" vector-effect="non-scaling-stroke" class="github-line" />
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
      <ChartTooltip x={cursorLeft} y={cursorPoint.y} visible={hover !== null} boundsWidth={plotWidth}>
        <strong>{fmtNumber(lastActive.words)}</strong> words
        {#if github}<div class="commits-scale">{describeActivity(activeActivity)}</div>{/if}
        <div class="tooltip-dim">{fmtDayLong(lastActive.day)}</div>
      </ChartTooltip>
    {/if}
  </div>
  {#if github && active}
    <p class="chart-announcement" aria-live="polite">{fmtDayLong(active.day)}: {fmtNumber(active.words)} dictated words. {describeActivity(activeActivity)}.</p>
  {/if}
</section>

<style>

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
  .plot:focus-visible { outline: 1px solid var(--ink-mute); outline-offset: 4px; border-radius: var(--r-sm); }
  .chart-legend { display: flex; align-items: center; gap: 16px; font-size: 11px; color: var(--ink-soft); margin: 0 0 12px; }
  .legend-item { display: inline-flex; align-items: center; gap: 6px; }
  .legend-line { width: 14px; border-top: 2px solid var(--accent); }
  .legend-line.commits { border-color: var(--success); }
  .scale-details { position: relative; margin-left: auto; }
  .scale-details summary { display: flex; align-items: center; justify-content: center; width: 24px; height: 24px; border-radius: var(--r-sm); color: var(--ink-mute); cursor: pointer; list-style: none; font-size: 14px; }
  .scale-details summary::-webkit-details-marker { display: none; }
  .scale-details summary:hover { color: var(--ink); background: var(--control-hover); }
  .scale-details p { position: absolute; z-index: 3; right: 0; top: 100%; width: 240px; max-width: calc(100vw - 72px); margin: 4px 0 0; padding: 12px; border: 1px solid var(--line); border-radius: var(--r-sm); background: var(--bg-elev); box-shadow: var(--shadow-popover); color: var(--ink-soft); font-size: 11px; line-height: 1.5; }
  .commits-scale { color: var(--success); }
  .chart-announcement { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }

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
