<script lang="ts">
  import { onMount } from 'svelte';
  import { fmtDay, fmtDayLong, fmtNumber, parseLocalDay } from './helpers';
  import AnimatedNumber from './AnimatedNumber.svelte';
  import ChartTooltip from './ChartTooltip.svelte';
  import type { InsightsDay, InsightsStreak } from './types';

  let {
    daily,
    streak,
    historyStartedOn,
  }: { daily: InsightsDay[]; streak: InsightsStreak; historyStartedOn: string | null } = $props();

  const CELL = 12;
  const GAP = 4;
  const STEP = CELL + GAP;
  const MIN_WEEKS = 18;
  const RIGHT_GUTTER = 10;
  const WEEKDAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const WEEKDAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

  interface GridCell { day: string; words: number; noData: boolean; streakDays: number }

  function streakColor(days: number, longest: number): string {
    if (days <= 0) return 'var(--control-hover)';
    const pct = longest <= 1 ? 100 : 8 + (days / longest) * 92;
    return `color-mix(in srgb, var(--accent) ${pct.toFixed(2)}%, var(--paper-2))`;
  }

  const allCells = $derived.by((): GridCell[] => {
    const pad = (n: number) => String(n).padStart(2, '0');
    const toKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

    const today = new Date();
    today.setHours(12, 0, 0, 0);
    const lastDate = daily.length > 0 ? parseLocalDay(daily[daily.length - 1].day) : today;

    const endDate = new Date(lastDate);
    endDate.setHours(12, 0, 0, 0);
    endDate.setDate(endDate.getDate() + (6 - endDate.getDay()));

    const firstRealDate = daily.length > 0 ? parseLocalDay(daily[0].day) : lastDate;
    const daysSpanningReal = Math.round((endDate.getTime() - firstRealDate.getTime()) / 86_400_000) + 1;
    const totalWeeks = Math.max(MIN_WEEKS, Math.ceil(daysSpanningReal / 7));

    const startDate = new Date(endDate);
    startDate.setDate(startDate.getDate() - (totalWeeks * 7 - 1));

    const byDay = new Map(daily.map((d) => [d.day, d]));
    const list: GridCell[] = [];
    const cursor = new Date(startDate);
    cursor.setHours(12, 0, 0, 0);
    let run = 0;
    for (let i = 0; i < totalWeeks * 7; i++) {
      const key = toKey(cursor);
      const real = byDay.get(key);
      const outsideHistory = !historyStartedOn || key < historyStartedOn || cursor > today;
      if (real && !outsideHistory) {
        run = real.words > 0 ? run + 1 : 0;
        list.push({ day: key, words: real.words, noData: false, streakDays: run });
      } else {
        run = 0;
        list.push({ day: key, words: 0, noData: true, streakDays: 0 });
      }
      cursor.setDate(cursor.getDate() + 1);
    }
    return list;
  });

  let gridHost = $state<HTMLDivElement | null>(null);
  let availableWidth = $state(640);
  const visibleWeekCount = $derived(
    Math.max(MIN_WEEKS, Math.min(53, Math.floor((availableWidth - RIGHT_GUTTER + GAP) / STEP)))
  );
  const cells = $derived(allCells.slice(-visibleWeekCount * 7));
  const longestVisibleScale = $derived(Math.max(1, cells.reduce((max, cell) => Math.max(max, cell.streakDays), 0)));
  const legendDays = $derived(Array.from(new Set([
    1,
    Math.max(1, Math.ceil(longestVisibleScale / 3)),
    Math.max(1, Math.ceil(longestVisibleScale * 2 / 3)),
    longestVisibleScale,
  ])));
  const columnCount = $derived(Math.ceil(cells.length / 7));
  const gridWidth = $derived(columnCount * STEP - GAP);

  onMount(() => {
    if (!gridHost) return;
    let raf = 0;
    const update = () => {
      raf = 0;
      if (gridHost) availableWidth = gridHost.clientWidth;
    };
    const schedule = () => {
      if (raf) return;
      raf = requestAnimationFrame(update);
    };
    update();
    const observer = new ResizeObserver(schedule);
    observer.observe(gridHost);
    return () => {
      observer.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
  });

  const monthLabels = $derived.by(() => {
    const labels: Array<{ col: number; label: string }> = [];
    let lastMonth = '';
    let lastCol = -Infinity;
    cells.forEach((cell, i) => {
      const month = cell.day.slice(0, 7);
      if (month !== lastMonth) {
        lastMonth = month;
        const col = Math.floor(i / 7);
        if (col >= lastCol + 2) {
          labels.push({ col, label: parseLocalDay(cell.day).toLocaleDateString([], { month: 'short' }) });
          lastCol = col;
        }
      }
    });
    return labels;
  });

  const bestWeekday = $derived.by(() => {
    const sums = new Array(7).fill(0);
    const counts = new Array(7).fill(0);
    for (const d of daily) {
      if (d.words <= 0) continue;
      const dow = parseLocalDay(d.day).getDay();
      sums[dow] += d.words;
      counts[dow] += 1;
    }
    const averages = sums.map((s, i) => (counts[i] > 0 ? s / counts[i] : 0));
    const peak = Math.max(...averages);
    if (peak <= 0) return null;
    const activeDays = counts.filter((c) => c > 0).length;
    if (activeDays < 3) return null;
    return { name: WEEKDAY_NAMES[averages.indexOf(peak)], avg: peak };
  });

  const rangeActivity = $derived.by(() => {
    let days = 0;
    let words = 0;
    for (const day of cells) {
      if (day.noData || day.words <= 0) continue;
      days++;
      words += day.words;
    }
    return { days, average: days > 0 ? words / days : 0 };
  });
  const visibleActiveDays = $derived(rangeActivity.days);
  const visibleRangeLabel = $derived.by(() => {
    if (cells.length === 0) return 'recent activity';
    return `${fmtDay(cells[0].day)} to ${fmtDay(cells[cells.length - 1].day)}`;
  });

  let hover = $state<GridCell | null>(null);
  let hoverPos = $state<{ x: number; y: number } | null>(null);

  function onEnter(cell: GridCell, event: PointerEvent) {
    hover = cell;
    const card = (event.currentTarget as HTMLElement).closest('.card');
    const cardRect = card?.getBoundingClientRect();
    const cellRect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    hoverPos = cardRect
      ? { x: cellRect.left - cardRect.left + CELL / 2, y: cellRect.top - cardRect.top }
      : null;
  }

  function clearHover() {
    hover = null;
    hoverPos = null;
  }

  const longestRange = $derived(
    streak.longest_started_on && streak.longest_ended_on
      ? `${fmtDay(streak.longest_started_on)} – ${fmtDay(streak.longest_ended_on)}`
      : null
  );
</script>

<section class="card">
  <header class="card-head">
    <div>
      <h2 class="card-h"><AnimatedNumber value={streak.current_days} /> day streak</h2>
      <p class="card-sub">{fmtNumber(visibleActiveDays)} active days shown · {visibleRangeLabel}</p>
    </div>
    <div class="best">
      <span class="best-num"><AnimatedNumber value={streak.longest_days} /></span>
      <span class="best-label">longest</span>
    </div>
  </header>

  <div class="heat-layout">
   <div class="heat-main">
    <div class="grid-row">
    <div class="weekday-col" aria-hidden="true">
      {#each WEEKDAY_LABELS as label}
        <span>{label}</span>
      {/each}
    </div>

    <div class="grid-scroll" bind:this={gridHost}>
     <div class="calendar-content" style:width="{gridWidth}px">
      <div class="month-row" aria-hidden="true">
        {#each monthLabels as m}
          <span class:edge={m.col >= columnCount - 2} style:left="{m.col * STEP}px">{m.label}</span>
        {/each}
      </div>
      <div
        class="grid"
        style:width="{gridWidth}px"
        role="img"
        aria-label={`Daily activity calendar. Current streak ${streak.current_days} days, longest ${streak.longest_days} days.`}
      >
        {#each cells as cell}
          {#if cell.noData}
            <span
              class="cell cell-nodata"
              role="presentation"
              onpointerenter={(e) => onEnter(cell, e)}
              onpointerleave={clearHover}
            ></span>
          {:else}
            <span
              class="cell"
              role="presentation"
              style:background={streakColor(cell.streakDays, longestVisibleScale)}
              onpointerenter={(e) => onEnter(cell, e)}
              onpointerleave={clearHover}
            ></span>
          {/if}
        {/each}
      </div>
     </div>
    </div>
    </div>

    <div class="legend">
      <span class="legend-label">Day 1</span>
      {#each legendDays as day}
        <span class="cell cell-key" style:background={streakColor(day, longestVisibleScale)} aria-hidden="true"></span>
      {/each}
      <span class="legend-label legend-label-more">Day {longestVisibleScale}</span>
      <span class="legend-gap"></span>
      <span class="cell cell-nodata" aria-hidden="true"></span>
      <span class="legend-label">Not tracked</span>
    </div>
   </div>

   <div class="heat-side">
    {#if daily.length === 0}
      <p class="foot">No activity to chart yet.</p>
    {:else if streak.longest_days === 0}
      <p class="foot">Dictate on two days in a row to start a streak.</p>
    {:else}
      <div class="streak-stats">
        <div class="stat">
          <span class="stat-label">Longest streak</span>
          <span class="stat-value">{streak.longest_days} {streak.longest_days === 1 ? 'day' : 'days'}</span>
          <span class="stat-sub">{fmtNumber(streak.longest_words)} words{#if longestRange} · {longestRange}{/if}</span>
        </div>
        {#if bestWeekday}
          <div class="stat">
            <span class="stat-label">Best day</span>
            <span class="stat-value">{bestWeekday.name}</span>
            <span class="stat-sub">{fmtNumber(bestWeekday.avg)} words on average</span>
          </div>
        {/if}
        <div class="stat">
          <span class="stat-label">Average active day</span>
          <span class="stat-sub">{fmtNumber(rangeActivity.average)} words per active day</span>
        </div>
      </div>
    {/if}
   </div>
  </div>

  {#if hoverPos && hover}
    <ChartTooltip x={hoverPos.x} y={hoverPos.y} visible={true}>
      {#if hover.noData}
        <strong>Not tracked</strong>
      {:else}
        {#if hover.streakDays > 0}
          <strong>Day {hover.streakDays} of streak</strong>
          <div class="tooltip-dim">{fmtNumber(hover.words)} words</div>
        {:else}
          <strong>No dictation</strong>
        {/if}
      {/if}
      <div class="tooltip-dim">{fmtDayLong(hover.day)}</div>
    </ChartTooltip>
  {/if}

  <div class="sr-only">
  <table>
    <caption>Words dictated per day</caption>
    <thead><tr><th scope="col">Day</th><th scope="col">Words</th></tr></thead>
    <tbody>
      {#each daily as d}
        <tr><th scope="row">{fmtDayLong(d.day)}</th><td>{fmtNumber(d.words)}</td></tr>
      {/each}
    </tbody>
  </table>
  </div>
</section>

<style>
  .card { position: relative; }

  .best { text-align: right; }
  .best-num {
    display: block;
    font-family: var(--sans);
    font-size: 19px;
    font-weight: 600;
    color: var(--ink);
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .best-label {
    font-size: 10.5px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--ink-mute);
  }

  .heat-layout {
    display: flex;
    gap: clamp(18px, 3vw, 32px);
    align-items: flex-start;
  }

  .heat-main { flex: 1 1 auto; min-width: 0; }

  .heat-side {
    flex: 0 0 clamp(200px, 24%, 300px);
    border-left: 1px solid var(--line);
    padding-left: clamp(18px, 3vw, 32px);
  }

  .grid-row {
    display: flex;
    gap: 6px;
  }

  .weekday-col {
    display: grid;
    grid-template-rows: repeat(7, 12px);
    gap: 4px;
    flex: 0 0 25px;
    padding-top: 16px;
  }
  .weekday-col span {
    font-size: 9px;
    line-height: 12px;
    color: var(--ink-mute);
    white-space: nowrap;
  }

  .grid-scroll {
    overflow: hidden;
    min-width: 0;
    flex: 1;
  }

  .calendar-content {
    margin-left: auto;
    margin-right: 10px;
  }

  .month-row {
    position: relative;
    height: 15px;
    margin-bottom: 4px;
  }
  .month-row span {
    position: absolute;
    top: 0;
    font-size: 9.5px;
    color: var(--ink-mute);
    white-space: nowrap;
  }
  .month-row span.edge {
    left: auto !important;
    right: 0;
  }

  .grid {
    display: grid;
    grid-template-rows: repeat(7, 12px);
    grid-auto-flow: column;
    grid-auto-columns: 12px;
    gap: 4px;
  }

  .cell {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    display: block;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ink) 12%, transparent);
    transition: background-color var(--ui-duration-base) var(--ui-ease-out);
  }
  .cell-nodata {
    background: color-mix(in srgb, var(--ink) 11%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ink) 20%, transparent);
  }

  .legend {
    display: flex;
    align-items: center;
    gap: 3px;
    margin-top: 14px;
  }
  .legend-label {
    font-size: 10.5px;
    color: var(--ink-mute);
  }
  .legend-label:first-child { margin-right: 3px; }
  .legend-label-more { margin-left: 3px; }
  .legend-gap { width: 10px; }

  @container insights (max-width: 540px) {
    .heat-layout { flex-direction: column; }
    .heat-main { align-self: stretch; width: 100%; }
    .heat-side {
      flex: 1 1 auto;
      align-self: stretch;
      border-left: 0;
      padding-left: 0;
      border-top: 1px solid var(--line);
      padding-top: 14px;
    }
  }

  .foot {
    margin: 0;
    font-size: 11.5px;
    color: var(--ink-soft);
    line-height: 1.45;
  }

  .streak-stats {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 14px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .stat-label {
    font-size: 10px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--ink-mute);
  }
  .stat-value {
    font-family: var(--sans);
    font-size: 15px;
    font-weight: 500;
    color: var(--ink);
    line-height: 1.2;
  }
  .stat-sub {
    font-size: 11px;
    color: var(--ink-soft);
    line-height: 1.35;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
