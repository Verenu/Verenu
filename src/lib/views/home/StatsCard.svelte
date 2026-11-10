<script lang="ts">
  import type { Stats } from './helpers';

  export let stats: Stats;
</script>

<div class="stat-card">
  <div class="stat-line">
    <span class="stat-num">
      {#if stats.total_words >= 1000}
        {(stats.total_words / 1000).toFixed(1)}<small>k</small>
      {:else}
        {stats.total_words}
      {/if}
    </span>
    <span class="stat-label">total words</span>
  </div>
  <div class="stat-line">
    <span class="stat-num">{Math.round(stats.avg_wpm) || '—'}</span>
    <span class="stat-label">wpm</span>
  </div>
  <div class="stat-line">
    <span class="stat-num">{stats.day_streak}</span>
    <span class="stat-label">day streak</span>
  </div>
</div>

<style>
  .stat-card { display: flex; align-items: flex-end; gap: 28px; }

  .stat-line {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 5px;
    min-width: 0;
  }

  .stat-num {
    font-family: var(--sans);
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
    line-height: 1;
    color: var(--ink);
  }
  .stat-num :global(small) {
    font-family: inherit;
    font-size: 13px;
    color: var(--ink-mute);
    margin-left: 1px;
    font-weight: 500;
  }

  .stat-label { font-size: 11.5px; line-height: 1.2; color: var(--ink-mute); white-space: nowrap; }
</style>
