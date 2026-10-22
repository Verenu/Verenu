<script lang="ts">
  import { untrack } from 'svelte';
  import { tweened } from 'svelte/motion';
  import { expoOut } from 'svelte/easing';
  import { motionMs } from '../../motion';
  import {
    fmtCompact,
    fmtNumber,
    bookEquivalent,
    pctDelta,
    paceScale,
    paceTickOffset,
    PACE_TICKS,
  } from './helpers';
  import AnimatedNumber from './AnimatedNumber.svelte';
  import type { InsightsPayload } from './types';

  let { data }: { data: InsightsPayload } = $props();

  const scoped = $derived(data.context_id !== null);

  const TICKS = PACE_TICKS;
  const tickIndices = Array.from({ length: TICKS }, (_, i) => i);
  let rulerW = $state(0);
  const scale = $derived(paceScale(data.totals.best_wpm));
  const scaleMax = $derived(scale.max);

  const wpmT = tweened(untrack(() => data.totals.avg_wpm), { duration: motionMs(650), easing: expoOut });
  $effect(() => { wpmT.set(data.totals.avg_wpm); });

  const totalWordsT = tweened(untrack(() => data.totals.words_in_range), { duration: motionMs(700), easing: expoOut });
  $effect(() => { totalWordsT.set(data.totals.words_in_range); });

  const wpm = $derived(Math.round($wpmT));
  const litTicks = $derived(Math.round(Math.min(1, Math.max(0, $wpmT / scaleMax)) * TICKS));
  const bestTick = $derived(scale.bestTick);

  const words = $derived(fmtCompact($totalWordsT));
  const books = $derived(bookEquivalent($totalWordsT));
  const booksLabel = $derived(books < 10 ? books.toFixed(1) : String(Math.round(books)));
  const isOneBook = $derived(Number(booksLabel) === 1);
  const delta = $derived(pctDelta(data.totals.words_in_range, data.totals.words_prev_range));
</script>

<div class="hero">
  <section
    class="tile tile-pace"
    aria-label={wpm > 0
      ? `Average speaking pace ${wpm} words per minute, best ${Math.round(data.totals.best_wpm)}`
      : 'Average speaking pace, not measured yet'}
  >
    <div class="tile-head">
      <span class="big">{wpm > 0 ? wpm : '—'}<small class="unit">wpm</small></span>
    </div>
    <p class="tile-label">average speaking pace</p>

    <div class="meter" aria-hidden="true">
      <div class="ruler" bind:clientWidth={rulerW}>
        {#each tickIndices as i}
          <span
            class="tick"
            class:major={i % 11 === 0}
            class:on={i < litTicks}
            class:best={i === bestTick}
            style="left: {paceTickOffset(i, rulerW)}"
          ></span>
        {/each}
      </div>
      <div class="scale">
        <span>0</span>
        <span>{scaleMax}</span>
      </div>
    </div>

    <p class="tile-note tile-note-dim">
      {#if data.totals.best_wpm > 0}
        <span class="best-key" aria-hidden="true"></span>Best <strong>{Math.round(data.totals.best_wpm)}</strong> wpm
      {:else}
        Speak a little longer to measure this
      {/if}
    </p>
  </section>

  <section class="tile tile-relative" aria-label="Total words dictated">
    {#if delta !== null}
      <span class="delta" class:down={delta < 0}>
        <svg class="delta-arrow" class:flip={delta < 0} width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 19V5M5 12l7-7 7 7"/>
        </svg>
        {Math.abs(delta).toFixed(1)}%
      </span>
    {/if}
    <div class="tile-head">
      <span class="big">{words.value}{#if words.suffix}<small>{words.suffix}</small>{/if}</span>
    </div>
    <p class="tile-label">total words dictated</p>
    <p class="tile-note">
      {#if books >= 1}
        {#if isOneBook}
          That's 1 full-length book of writing.
        {:else}
          That's {booksLabel} full-length books of writing.
        {/if}
      {:else if data.totals.total_words > 0}
        {Math.round(books * 100)}% of a full-length book so far.
      {:else}
        Your first dictation starts the count.
      {/if}
    </p>
    <p class="tile-note tile-note-dim">
      {fmtNumber(data.totals.total_words)} words all-time
    </p>
  </section>

  <section class="tile" aria-label="Words changed by Verenu">
    <div class="tile-head">
      <span class="big"><AnimatedNumber value={data.cleanup.edits_applied} /></span>
    </div>
    <p class="tile-label">words changed by Verenu</p>
    {#if scoped}
      <p class="tile-note tile-note-dim">dictionary and learned totals across all contexts</p>
    {/if}
    <div class="sub-rows">
      <div class="stat-line">
        <span class="stat-num"><AnimatedNumber value={data.cleanup.dictionary_fixes} /></span>
        <span class="stat-label">dictionary fixes</span>
      </div>
      <div class="stat-line">
        <span class="stat-num"><AnimatedNumber value={data.cleanup.auto_learned_terms} /></span>
        <span class="stat-label">terms auto-learned</span>
      </div>
    </div>
  </section>
</div>

<style>
  .hero {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: clamp(18px, 3vw, 32px);
    padding-bottom: 20px;
    margin-bottom: 26px;
    border-bottom: 1px solid var(--line);
  }

  .tile {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .tile + .tile {
    border-left: 1px solid var(--line);
    padding-left: clamp(18px, 3vw, 32px);
  }

  .tile-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
    min-width: 0;
  }

  .tile-relative .tile-head {
    padding-right: 92px;
  }

  .big {
    font-family: var(--sans);
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.03em;
    line-height: 1;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }
  .big small {
    font-size: 18px;
    color: var(--ink-mute);
    font-weight: 400;
  }

  .tile-relative { position: relative; }

  .delta {
    position: absolute;
    top: 0;
    right: 0;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    font-weight: 500;
    padding: 3px 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-ink);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .delta.down {
    background: var(--danger-bg);
    color: var(--danger);
  }

  .delta-arrow.flip { transform: rotate(180deg); }

  .tile-label {
    margin: 9px 0 0;
    font-size: 10.5px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--ink-mute);
  }

  .tile-note {
    margin: 8px 0 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--ink-soft);
  }
  .tile-note-dim {
    margin-top: auto;
    padding-top: 8px;
    color: var(--ink-mute);
    font-size: 11.5px;
  }

  .big .unit {
    font-size: 12px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-mute);
    margin-left: 5px;
  }

  .meter { margin-top: 14px; }

  .ruler {
    position: relative;
    height: 18px;
  }

  .tick {
    position: absolute;
    bottom: 0;
    width: 2px;
    height: 10px;
    border-radius: 1px;
    background: var(--line-strong);
    transition:
      background-color var(--ui-duration-base, 200ms) ease,
      height var(--ui-duration-base, 200ms) ease;
  }
  .tick.major { height: 14px; }
  .tick.on { background: var(--accent); }
  .tick.best {
    height: 18px;
    background: var(--ink-soft);
  }

  .scale {
    display: flex;
    justify-content: space-between;
    margin-top: 7px;
    font-family: var(--sans);
    font-size: 10px;
    font-weight: 500;
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
  }

  .best-key {
    display: inline-block;
    width: 2px;
    height: 10px;
    border-radius: 1px;
    background: var(--ink-soft);
    margin-right: 7px;
    vertical-align: -1px;
  }

  .tile-pace .tile-note-dim strong {
    color: var(--ink-soft);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .sub-rows {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .stat-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    border-top: 1px solid var(--line);
    padding-top: 8px;
  }

  .stat-num {
    font-family: var(--sans);
    font-size: 16px;
    font-weight: 600;
    line-height: 1;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 11.5px;
    color: var(--ink-mute);
    margin-left: auto;
  }

  @container insights (max-width: 680px) {
    .hero { gap: 14px; }
    .tile + .tile { padding-left: 14px; }
    .tile-relative .tile-head { padding-right: 84px; }
    .delta { font-size: 10px; padding: 2px 7px; }
    .big { font-size: 24px; }
    .big small { font-size: 15px; }
    .big .unit { font-size: 11px; margin-left: 4px; }
    .ruler { height: 16px; }
    .tick { height: 9px; }
    .tick.best { height: 16px; }
    .tile-note { font-size: 11.5px; }
    .stat-num { font-size: 14px; }
    .stat-label { font-size: 10.5px; }
  }

  @container insights (max-width: 500px) {
    .hero { grid-template-columns: 1fr; gap: 0; }
    .tile + .tile {
      border-left: 0;
      padding-left: 0;
      border-top: 1px solid var(--line);
      padding-top: 18px;
      margin-top: 18px;
    }
  }

  @container insights (max-width: 380px) {
    .tile-relative .tile-head { padding-right: 0; }
    .delta { position: static; align-self: flex-start; margin-bottom: 8px; }
  }
</style>
