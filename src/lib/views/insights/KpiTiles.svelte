<script lang="ts">
  import { untrack } from 'svelte';
  import { tweened } from 'svelte/motion';
  import { expoOut } from 'svelte/easing';
  import { motionMs, reducedMotionEnabled } from '../../motion';
  import { bookEquivalent, fmtCompact, fmtDuration, fmtNumber, PACE_TICKS, paceScale, paceTickOffset, pctDelta } from './helpers';
  import { activeDayCount, fmtHoursMinutes, speedupOverTyping, timeSavedMs, TYPING_WPM } from './analytics';
  import type { InsightsPayload } from './types';

  let { data, rangeDays }: { data: InsightsPayload; rangeDays: number } = $props();

  const TICKS = PACE_TICKS;
  const tickIndices = Array.from({ length: TICKS }, (_, i) => i);

  // Headline figures count up from zero the first time the page renders, then
  // glide between values when the range or context changes.
  const startAt = (value: number) => (reducedMotionEnabled() ? value : 0);
  const wordsT = tweened(untrack(() => startAt(data.totals.words_in_range)), { duration: motionMs(900), easing: expoOut });
  const savedT = tweened(untrack(() => startAt(timeSavedMs(data.totals.words_in_range, data.totals.total_speaking_ms))), { duration: motionMs(900), easing: expoOut });
  const wpmT = tweened(untrack(() => startAt(data.totals.avg_wpm)), { duration: motionMs(900), easing: expoOut });
  const activeT = tweened(untrack(() => startAt(activeDayCount(data.daily))), { duration: motionMs(700), easing: expoOut });

  const saved = $derived(timeSavedMs(data.totals.words_in_range, data.totals.total_speaking_ms));
  const activeDays = $derived(activeDayCount(data.daily));
  const spanDays = $derived(Math.max(1, data.daily.length));

  $effect(() => { wordsT.set(data.totals.words_in_range); });
  $effect(() => { savedT.set(saved); });
  $effect(() => { wpmT.set(Number.isFinite(data.totals.avg_wpm) ? data.totals.avg_wpm : 0); });
  $effect(() => { activeT.set(activeDays); });

  const words = $derived(fmtCompact($wordsT));
  const savedLabel = $derived(fmtHoursMinutes($savedT));
  const delta = $derived(rangeDays > 0 ? pctDelta(data.totals.words_in_range, data.totals.words_prev_range) : null);
  const books = $derived(bookEquivalent(data.totals.words_in_range));

  const bestWpm = $derived(Math.round(data.totals.best_wpm));
  const scale = $derived(paceScale(data.totals.best_wpm, TICKS));
  const wpm = $derived(Math.round($wpmT));
  const litTicks = $derived(Math.round(Math.min(1, Math.max(0, $wpmT / scale.max)) * TICKS));
  const speedup = $derived(speedupOverTyping(data.totals.avg_wpm));
  let rulerW = $state(0);

  const activeRatio = $derived(Math.min(1, $activeT / spanDays));

  function booksCaption(n: number): string {
    if (data.totals.words_in_range <= 0) return 'Your first dictation starts the count';
    if (n < 0.01) return 'Under 1% of a full-length book';
    if (n < 1) return `${Math.round(n * 100)}% of a full-length book`;
    const label = n < 10 ? n.toFixed(1).replace(/\.0$/, '') : fmtNumber(n);
    return `${label} full-length ${label === '1' ? 'book' : 'books'} of writing`;
  }

  function fmtDelta(pct: number): string {
    const abs = Math.abs(pct);
    // Past +1,000% a growth factor reads better than a percentage.
    if (pct >= 1000) return `${fmtNumber(1 + pct / 100)}×`;
    return `${abs < 10 ? abs.toFixed(1) : Math.round(abs)}%`;
  }
</script>

<div class="kpis">
  <section class="kpi" aria-label="Words dictated">
    <header class="kpi-top">
      <span class="kpi-label">Words dictated</span>
      {#if delta !== null && Number.isFinite(delta)}
        <span
          class="delta"
          class:up={delta > 0.05}
          class:down={delta < -0.05}
          title="Compared with the previous {rangeDays} days"
        >
          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {#if delta >= 0}<path d="M12 19V5M5 12l7-7 7 7"/>{:else}<path d="M12 5v14M5 12l7 7 7-7"/>{/if}
          </svg>
          <span class="sr-only">{delta >= 0 ? 'Up' : 'Down'}</span>{fmtDelta(delta)}
        </span>
      {/if}
    </header>
    <p class="kpi-value">{words.value.replace(/\.0$/, "")}{#if words.suffix}<span class="kpi-suffix">{words.suffix}</span>{/if}</p>
    <p class="kpi-caption">{booksCaption(books)}</p>
    <p class="kpi-foot">{fmtNumber(data.totals.total_words)} all-time</p>
  </section>

  <section class="kpi" aria-label="Time saved compared with typing">
    <header class="kpi-top">
      <span class="kpi-label">Time saved</span>
    </header>
    <p class="kpi-value">{savedLabel.value}<span class="kpi-unit">{savedLabel.unit}</span></p>
    <p class="kpi-caption">versus typing at {TYPING_WPM} wpm</p>
    <p class="kpi-foot">{data.totals.total_speaking_ms >= 100 * 3_600_000 ? `${fmtNumber(data.totals.total_speaking_ms / 3_600_000)} hrs` : fmtDuration(data.totals.total_speaking_ms)} spent speaking</p>
  </section>

  <section
    class="kpi"
    aria-label={wpm > 0 ? `Average speaking pace ${wpm} words per minute${bestWpm > 0 ? `, best ${bestWpm}` : ''}` : 'Average speaking pace, not measured yet'}
  >
    <header class="kpi-top">
      <span class="kpi-label">Speaking pace</span>
      {#if speedup !== null && speedup >= 1.05}
        <span class="chip" title="Compared with typing at {TYPING_WPM} wpm">{speedup < 10 ? speedup.toFixed(1) : Math.round(speedup)}× typing</span>
      {/if}
    </header>
    <p class="kpi-value">{wpm > 0 ? wpm : '—'}<span class="kpi-unit">wpm</span></p>
    <div class="ruler" bind:clientWidth={rulerW} aria-hidden="true">
      {#each tickIndices as i}
        <span
          class="tick"
          class:on={i < litTicks}
          class:best={i === scale.bestTick}
          style:left={paceTickOffset(i, rulerW, undefined, TICKS)}
          style:transition-delay="{i * 12}ms"
        ></span>
      {/each}
    </div>
    <p class="kpi-foot">
      {#if bestWpm > 0}
        <span class="best-key" aria-hidden="true"></span>Best {fmtNumber(bestWpm)} wpm
      {:else}
        Speak a little longer to measure this
      {/if}
    </p>
  </section>

  <section class="kpi" aria-label="Active days: {activeDays} of {spanDays}">
    <header class="kpi-top">
      <span class="kpi-label">Active days</span>
      {#if data.streak.current_days > 1}
        <span class="chip chip-streak" title="Consecutive days with a dictation">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 2c1 3.5 5 6 5 11a5 5 0 0 1-10 0c0-2 1-3.5 2-4.5 0 2 1 3 2 3 0-3 1-6.5 1-9.5Z"/></svg>
          {fmtNumber(data.streak.current_days)}
        </span>
      {/if}
    </header>
    <p class="kpi-value">{Math.round($activeT)}<span class="kpi-unit">/ {fmtNumber(spanDays)}</span></p>
    <div class="meter" aria-hidden="true"><span class="meter-fill" style:transform="scaleX({activeRatio})"></span></div>
    <p class="kpi-foot">
      {#if data.streak.current_days > 0}
        {fmtNumber(data.streak.current_days)}-day streak · best {fmtNumber(data.streak.longest_days)}
      {:else if data.streak.longest_days > 0}
        Longest streak {fmtNumber(data.streak.longest_days)} {data.streak.longest_days === 1 ? 'day' : 'days'}
      {:else}
        No streak yet
      {/if}
    </p>
  </section>
</div>

<style>
  .kpis {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--insights-gap);
  }

  .kpi {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 14px 16px 13px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .kpi:hover { border-color: var(--line-strong); }

  .kpi-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    min-height: 20px;
    min-width: 0;
  }

  .kpi-label {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--ink-mute);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .kpi-value {
    margin: 10px 0 0;
    font-family: var(--sans);
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.03em;
    line-height: 1;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .kpi-suffix { font-weight: 500; color: var(--ink-mute); margin-left: 1px; }

  .kpi-unit {
    margin-left: 5px;
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0;
    color: var(--ink-mute);
  }

  .kpi-caption {
    margin: 8px 0 0;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--ink-soft);
  }

  .kpi-foot {
    margin: auto 0 0;
    padding-top: 12px;
    font-size: 11px;
    line-height: 1.35;
    color: var(--ink-mute);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  .delta,
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    height: 20px;
    padding: 0 7px;
    border-radius: 999px;
    font-size: 10.5px;
    font-weight: 600;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    background: var(--paper-3);
    color: var(--ink-soft);
  }
  .delta.up { background: var(--success-bg); color: var(--success); }
  .delta.down { background: var(--paper-3); color: var(--ink-mute); }
  .chip-streak { color: var(--warning); background: var(--warning-bg); }

  .ruler {
    position: relative;
    height: 16px;
    margin-top: 12px;
  }

  .tick {
    position: absolute;
    bottom: 0;
    width: 2px;
    height: 9px;
    border-radius: 1px;
    background: var(--line-strong);
    transform-origin: bottom;
    transition: background-color 260ms var(--ui-ease-out);
  }
  .tick.on { background: var(--accent); }
  .tick.best { height: 16px; background: var(--ink-soft); }

  .best-key {
    display: inline-block;
    width: 2px;
    height: 9px;
    border-radius: 1px;
    background: var(--ink-soft);
    margin-right: 6px;
    vertical-align: -1px;
  }

  .meter {
    position: relative;
    height: 6px;
    margin-top: 17px;
    border-radius: 999px;
    background: var(--paper-3);
    overflow: hidden;
  }
  .meter-fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: var(--accent);
    transform-origin: left;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @container insights (max-width: 760px) {
    .kpis { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }

  @container insights (max-width: 340px) {
    .kpis { grid-template-columns: minmax(0, 1fr); }
  }

  @media (prefers-reduced-motion: reduce) {
    .tick { transition-duration: 1ms; transition-delay: 0ms !important; }
  }
</style>
