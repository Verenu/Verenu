<script lang="ts">
  import AnimatedNumber from './AnimatedNumber.svelte';
  import { fmtNumber } from './helpers';
  import type { InsightsCleanup } from './types';

  let { cleanup, totalDictations = 0 }: { cleanup: InsightsCleanup; totalDictations?: number } = $props();

  // Word counts here come from dictation text still kept in history, while the dictation total
  // comes from daily statistics that outlive history retention. Say so whenever any are missing.
  // An older backend without `text_dictations` gives no coverage signal, so no qualifier.
  const saved = $derived(cleanup.text_dictations);
  const textMissing = $derived(saved !== undefined && saved <= 0 && totalDictations > 0);
  const textPartial = $derived(saved !== undefined && saved > 0 && saved < totalDictations);

  const rawMax = $derived(Math.max(cleanup.raw_words, cleanup.clean_words, 1));
  const changePct = $derived(
    cleanup.raw_words > 0 ? ((cleanup.clean_words - cleanup.raw_words) / cleanup.raw_words) * 100 : null,
  );
  const untracked = $derived(cleanup.activity_incomplete && !cleanup.activity_tracked_dictations);
  const editRate = $derived(cleanup.raw_words > 0 ? cleanup.edits_applied / cleanup.raw_words : null);

  function fmtChange(pct: number): string {
    const abs = Math.abs(pct);
    return `${abs < 10 ? abs.toFixed(1) : fmtNumber(abs)}%`;
  }
</script>

<section class="card cleanup-card" aria-labelledby="cleanup-h">
  <header class="card-head">
    <div>
      <h2 class="card-h" id="cleanup-h">What Verenu fixed</h2>
      <p class="card-sub">
        {#if editRate !== null && editRate > 0}
          About 1 in {fmtNumber(Math.max(1, Math.round(1 / Math.min(1, editRate))))} words changed in cleanup
        {:else}
          Changes cleanup made to your words
        {/if}
      </p>
    </div>
    <div class="readout">
      <span class="readout-num"><AnimatedNumber value={cleanup.edits_applied} from={0} /></span>
      <span class="readout-label">words changed</span>
    </div>
  </header>

  <div class="compare" aria-label="Words before and after cleanup">
    <div class="compare-row">
      <span class="compare-label">You said</span>
      <span class="compare-track"><span class="compare-fill raw" style:transform="scaleX({cleanup.raw_words / rawMax})"></span></span>
      <span class="compare-num">{fmtNumber(cleanup.raw_words)}</span>
    </div>
    <div class="compare-row">
      <span class="compare-label">Verenu wrote</span>
      <span class="compare-track"><span class="compare-fill clean" style:transform="scaleX({cleanup.clean_words / rawMax})"></span></span>
      <span class="compare-num">{fmtNumber(cleanup.clean_words)}</span>
    </div>
    <p class="compare-note">
      {#if textMissing}
        Dictation text for this range is no longer in history, so this comparison is unavailable.
      {:else if changePct === null}
        No cleaned dictations in this range yet.
      {:else if changePct < -0.05}
        Cleanup trimmed <strong>{fmtChange(changePct)}</strong> of the spoken words.
      {:else if changePct > 0.05}
        Cleanup added <strong>{fmtChange(changePct)}</strong> more words, such as expanded snippets.
      {:else}
        Cleanup kept your word count the same.
      {/if}
      {#if textPartial}{' '}Based on dictations still saved in history.{/if}
    </p>
  </div>

  <dl class="figures">
    <div>
      <dt>Vocabulary fixes</dt>
      <dd>{#if untracked}—{:else}{#if cleanup.activity_incomplete}≥{/if}<AnimatedNumber value={cleanup.vocabulary_replacements} from={0} />{/if}</dd>
    </div>
    <div>
      <dt>Snippet expansions</dt>
      <dd>{#if untracked}—{:else}{#if cleanup.activity_incomplete}≥{/if}<AnimatedNumber value={cleanup.snippet_expansions} from={0} />{/if}</dd>
    </div>
    <div>
      <dt>Terms learned</dt>
      <dd><AnimatedNumber value={cleanup.auto_learned_terms} from={0} /></dd>
    </div>
  </dl>
  {#if cleanup.activity_incomplete}
    <p class="hint">Some dictations lack detail or have expired from history, so counts are minimums.</p>
  {/if}
</section>

<style>
  .readout { text-align: right; white-space: nowrap; }
  .readout-num {
    display: block;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }
  .readout-label { font-size: 11px; color: var(--ink-mute); }

  .compare { display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px; }

  .compare-row {
    display: grid;
    grid-template-columns: 82px minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
  }

  .compare-label { font-size: 11.5px; color: var(--ink-mute); white-space: nowrap; }

  .compare-track {
    position: relative;
    height: 8px;
    border-radius: 999px;
    background: var(--paper-3);
    overflow: hidden;
  }

  .compare-fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    transform-origin: left;
    transition: transform 620ms var(--ui-ease-out);
    animation: fill-in 820ms var(--ui-ease-out) both 160ms;
  }
  .compare-fill.raw { background: color-mix(in srgb, var(--accent) 35%, var(--paper-3)); }
  .compare-fill.clean { background: var(--accent); animation-delay: 240ms; }

  @keyframes fill-in {
    from { scale: 0 1; }
    to { scale: 1 1; }
  }

  .compare-num {
    min-width: 48px;
    text-align: right;
    font-size: 11.5px;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }

  .compare-note {
    margin: 2px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--ink-mute);
  }
  .compare-note strong { color: var(--ink); font-weight: 500; }

  .figures {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
    margin: auto 0 0;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }
  .figures div { min-width: 0; }
  .figures dt { font-size: 11px; color: var(--ink-mute); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .figures dd {
    margin: 4px 0 0;
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--ink);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }

  .hint { margin: 10px 0 0; font-size: 11px; line-height: 1.45; color: var(--ink-faint); }

  @container insights (max-width: 420px) {
    .readout { text-align: left; }
  }

  @container insights (max-width: 380px) {
    .figures { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
    .compare-row { grid-template-columns: 74px minmax(0, 1fr) auto; }
  }

  @media (prefers-reduced-motion: reduce) {
    .compare-fill { animation: none; transition-duration: 1ms; }
  }
</style>
