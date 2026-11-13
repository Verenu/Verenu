<script lang="ts">
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { motionMs } from '../../motion';
  import { fmtCompact, fmtNumber } from './helpers';
  import { fmtPercent } from './analytics';
  import AnimatedNumber from './AnimatedNumber.svelte';
  import type { InsightsTotals, InsightsWords } from './types';

  let { words, totals }: { words: InsightsWords; totals: InsightsTotals } = $props();

  const max = $derived(Math.max(...words.top.map((w) => w.count), 0));
  // Share of distinct words among everything said in the range. Only meaningful with
  // enough text; a handful of words is always "100% unique".
  const variety = $derived(
    totals.words_in_range >= 200 && words.unique_words > 0
      ? Math.min(1, words.unique_words / totals.words_in_range)
      : null,
  );

  /** Full digits up to a million; past that a compact figure keeps the grid cell readable. */
  function fmtFigure(n: number): string {
    if (Math.abs(n) < 1_000_000) return fmtNumber(n);
    const { value, suffix } = fmtCompact(n);
    return `${value}${suffix}`;
  }

  const HARD_CHAR_CAP = 300;
  let wordExpanded = $state(false);
  const rawWord = $derived.by(() => {
    const w = words.longest_word ?? '';
    return w.length > HARD_CHAR_CAP ? `${w.slice(0, HARD_CHAR_CAP)}…` : w;
  });
</script>

<section class="card vocab-card" aria-labelledby="vocab-h">
  <header class="card-head">
    <div>
      <h2 class="card-h" id="vocab-h">Your vocabulary</h2>
      <p class="card-sub">Most-used words, everyday filler excluded</p>
    </div>
  </header>

  <div class="split">
    {#if words.top.length > 0}
      <ol class="word-list" style:--rows={Math.ceil(words.top.length / 2)}>
        {#each words.top as entry, i (entry.word)}
          <li style:--i={i}>
            <span class="rank" aria-hidden="true">{i + 1}</span>
            <span class="word-track">
              <span class="word-fill" style:transform="scaleX({max > 0 ? Math.max(0.04, entry.count / max) : 0})"></span>
              <span class="word" title={entry.word}>{entry.word}</span>
            </span>
            <span class="word-count">{fmtNumber(entry.count)}</span>
          </li>
        {/each}
      </ol>
    {:else}
      <p class="foot">Not enough dictation yet to find your favourite words.</p>
    {/if}

    <dl class="figures">
      <div>
        <dt>Unique words</dt>
        <dd title={fmtNumber(words.unique_words)}><AnimatedNumber value={words.unique_words} from={0} format={fmtFigure} /></dd>
      </div>
      <div>
        <dt>Word variety</dt>
        <dd title="Distinct words as a share of all words in this range">
          {#if variety === null}—{:else}{fmtPercent(variety)}{/if}
        </dd>
      </div>
      <div>
        <dt>Words per dictation</dt>
        <dd><AnimatedNumber value={totals.avg_words_per_transcription} from={0} /></dd>
      </div>
      <div>
        <dt>Dictations</dt>
        <dd title={fmtNumber(totals.total_transcriptions)}><AnimatedNumber value={totals.total_transcriptions} from={0} format={fmtFigure} /></dd>
      </div>
      <div>
        <dt>Average word length</dt>
        <dd>
          {#if words.avg_word_length > 0}
            <AnimatedNumber value={words.avg_word_length} format={(n) => `${n.toFixed(1)} chars`} />
          {:else}
            —
          {/if}
        </dd>
      </div>
      <div class="longest-word-field">
        <dt>Longest word</dt>
        {#if !rawWord}
          <dd class="dd-word">—</dd>
        {:else}
          <dd class="dd-word">
            <button
              type="button"
              class="word-toggle"
              class:open={wordExpanded}
              aria-expanded={wordExpanded}
              aria-controls={wordExpanded ? 'insights-longest-word-full' : undefined}
              onclick={() => (wordExpanded = !wordExpanded)}
              onkeydown={(event) => {
                if (event.key === 'Escape' && wordExpanded) {
                  event.preventDefault();
                  wordExpanded = false;
                }
              }}
              title={wordExpanded ? 'Click to collapse' : 'Click to see the full word'}
            >
              <span class="word-toggle-text">{rawWord}</span>
              <svg class="word-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="m6 9 6 6 6-6"/>
              </svg>
            </button>
          </dd>
        {/if}
      </div>
    </dl>
  </div>
  {#if wordExpanded && rawWord}
    <div id="insights-longest-word-full" class="word-full scroll-styled" transition:slide={{ duration: motionMs(220), easing: cubicOut }}>
      {rawWord}
    </div>
  {/if}
</section>

<style>
  .split {
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
  }

  .word-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    grid-template-rows: repeat(var(--rows, 6), auto);
    grid-auto-flow: column;
    gap: 5px 28px;
    min-width: 0;
  }

  .word-list li {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    min-width: 0;
    animation: word-in 420ms var(--ui-ease-out) both;
    animation-delay: calc(var(--i) * 28ms + 100ms);
  }

  @keyframes word-in {
    from { opacity: 0; transform: translateX(-6px); }
    to { opacity: 1; transform: none; }
  }

  .rank {
    font-size: 10.5px;
    color: var(--ink-faint);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .word-track {
    position: relative;
    display: flex;
    align-items: center;
    height: 24px;
    min-width: 0;
    border-radius: 6px;
  }

  .word-fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: color-mix(in srgb, var(--accent) 12%, var(--paper-2));
    transform-origin: left;
    transition: transform 560ms var(--ui-ease-out);
  }
  .word-list li:first-child .word-fill { background: color-mix(in srgb, var(--accent) 20%, var(--paper-2)); }

  .word {
    position: relative;
    padding: 0 9px;
    font-size: 11.5px;
    color: var(--ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .word-count {
    font-size: 11.5px;
    color: var(--ink-mute);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .figures {
    order: -1;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 16px 18px;
    padding-bottom: 18px;
    border-bottom: 1px solid var(--line);
  }
  .figures div { min-width: 0; }
  .figures dt {
    font-size: 11px;
    color: var(--ink-mute);
    margin-bottom: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .figures dd {
    margin: 0;
    font-family: var(--sans);
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--ink);
    line-height: 1.15;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }
  .dd-word { font-size: 15px; }

  .longest-word-field { overflow: hidden; }

  .word-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    background: none;
    border: 0;
    padding: 0;
    margin: 0;
    font: inherit;
    cursor: pointer;
    color: var(--ink);
    text-align: left;
  }
  .word-toggle-text {
    min-width: 0;
    flex: 1;
    font-family: var(--sans);
    font-size: 15px;
    font-weight: 500;
    color: inherit;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-bottom: 1px dashed var(--line-strong);
    transition: color var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .word-toggle:hover .word-toggle-text {
    color: var(--accent-ink);
    border-color: var(--accent-ink);
  }
  .word-toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 3px; }
  .word-chevron {
    flex: 0 0 auto;
    color: var(--ink-mute);
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .word-toggle.open .word-chevron { transform: rotate(180deg); }

  .word-full {
    margin-top: 14px;
    padding: 10px 12px;
    max-height: 130px;
    overflow-y: auto;
    border-radius: var(--r-sm);
    background: var(--paper-2);
    font-family: var(--sans);
    font-size: 13px;
    font-weight: 500;
    color: var(--ink);
    line-height: 1.45;
    word-break: break-word;
  }

  .foot {
    margin: 0;
    font-size: 11.5px;
    color: var(--ink-mute);
  }

  @container insights (max-width: 760px) {
    .figures { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  }

  @container insights (max-width: 560px) {
    .word-list { grid-template-columns: minmax(0, 1fr); grid-auto-flow: row; grid-template-rows: none; }
  }

  @container insights (max-width: 400px) {
    .figures { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }

  @media (prefers-reduced-motion: reduce) {
    .word-list li { animation: none; }
    .word-fill { transition-duration: 1ms; }
  }
</style>
