<script lang="ts">
  import { estimateCost, type PricingSnapshot } from './pricing';
  import { fmtDuration, fmtNumber, fmtUsd } from './helpers';
  import DonutChart, { type DonutSegment } from './DonutChart.svelte';
  import { isAndroid } from '../../platform';
  import { costPerThousandWords } from './analytics';
  import type { InsightsProviderUsage } from './types';

  let { providers, rangeLabel, pricing, wordsInRange = 0 }: {
    providers: InsightsProviderUsage[];
    rangeLabel: string;
    pricing: PricingSnapshot | null;
    wordsInRange?: number;
  } = $props();

  const summary = $derived(estimateCost(providers, pricing));
  const perThousand = $derived(costPerThousandWords(summary.rows.some((row) => row.cost !== null) ? summary.total : null, wordsInRange));

  function segmentColor(index: number, count: number): string {
    if (count <= 1) return 'var(--accent)';
    const pct = 100 - Math.round((index / Math.max(1, count - 1)) * 62);
    return `color-mix(in srgb, var(--accent) ${pct}%, var(--paper-2))`;
  }

  const segments = $derived.by((): DonutSegment[] => {
    const priced = summary.rows.filter((row) => (row.cost ?? 0) > 0);
    return priced.map((row, i) => ({
      id: `${row.provider}:${row.model}:${row.task}`,
      name: row.model,
      color: segmentColor(i, priced.length),
      value: row.cost ?? 0,
      valueLabel: fmtUsd(row.cost),
    }));
  });

  function usageLabel(row: InsightsProviderUsage): string {
    if (row.task === 'transcription') return fmtDuration(row.audio_ms);
    const tokens = (row.input_chars + row.output_chars) / 4;
    return `~${fmtNumber(tokens)} tokens`;
  }

  function averageUsageLabel(row: InsightsProviderUsage): string {
    if (row.calls <= 0) return '—';
    if (row.task === 'transcription') return `${fmtDuration(row.audio_ms / row.calls)} / transcription`;
    const tokens = (row.input_chars + row.output_chars) / 4;
    return `~${fmtNumber(tokens / row.calls)} tokens / cleanup`;
  }

  function averageCostLabel(row: { calls: number; cost: number | null }): string {
    return row.calls > 0 && row.cost !== null ? fmtUsd(row.cost / row.calls) : '—';
  }
</script>

<section class="card">
  <header class="card-head">
    <div>
      <h2 class="card-h">Estimated API cost</h2>
      <p class="card-sub">{rangeLabel} · billed by your provider, not by Verenu</p>
    </div>
    {#if perThousand !== null}
      <div class="readout">
        <span class="readout-num">{perThousand > 0 && perThousand < 1 ? `$${perThousand.toFixed(3)}` : fmtUsd(perThousand)}</span>
        <span class="readout-label">per 1,000 words</span>
      </div>
    {/if}
  </header>

  {#if summary.rows.length === 0}
    <p class="foot">No API usage recorded in this range.</p>
  {:else}
    {#if segments.length > 0}
      <DonutChart {segments} primaryLabel={summary.total > 0 && summary.total < 0.01 ? "<$0.01" : fmtUsd(summary.total)} secondaryLabel="total" showLegend={!isAndroid} />
    {/if}

    <div class="cost-scroll scroll-styled">
    <table class="cost-table">
      <thead>
        <tr>
          <th scope="col">Model</th>
          <th scope="col" class="num">Total usage</th>
          <th scope="col" class="num avg-usage">Average usage</th>
          <th scope="col" class="num">Est. cost</th>
          <th scope="col" class="num avg-cost">Avg. cost</th>
          <th scope="col" class="num">Share</th>
        </tr>
      </thead>
      <tbody>
        {#each summary.rows as row}
          <tr>
            <th scope="row">
              <span class="model">{row.model}</span>
              <span class="task">{row.task} · {row.provider} · {fmtNumber(row.calls)} calls</span>
            </th>
            <td class="num">{usageLabel(row)}</td>
            <td class="num avg-usage">{averageUsageLabel(row)}</td>
            <td class="num">{fmtUsd(row.cost)}</td>
            <td class="num avg-cost">{averageCostLabel(row)}</td>
            <td class="num">{row.cost === null ? '—' : `${(row.share * 100).toFixed(1)}%`}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    </div>

    {#if summary.hasUnpriced}
      <p class="foot">Models shown as — have no published rate on file, so they are missing from the total.</p>
    {/if}
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

  .cost-scroll {
    margin-top: 16px;
    overflow-x: auto;
    min-width: 0;
  }

  .cost-table {
    width: 100%;
    min-width: 520px;
    border-collapse: collapse;
    font-size: 11.5px;
    table-layout: fixed;
  }
  .cost-table th,
  .cost-table td {
    text-align: left;
    padding: 9px 6px;
    border-bottom: 1px solid var(--line);
    font-weight: 400;
  }
  .cost-table th:first-child { width: 30%; padding-left: 0; }
  .cost-table th:last-child, .cost-table td:last-child { padding-right: 0; }
  .cost-table thead th:nth-child(2) { width: 16%; }
  .cost-table thead th:nth-child(3) { width: 21%; }
  .cost-table thead th:nth-child(4) { width: 11%; }
  .cost-table thead th:nth-child(5) { width: 13%; }
  .cost-table thead th {
    font-size: 11px;
    color: var(--ink-mute);
    padding-top: 0;
  }
  .cost-table tbody tr:last-child th,
  .cost-table tbody tr:last-child td { border-bottom: 0; }

  .cost-table .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--ink-soft);
    white-space: nowrap;
  }

  .model {
    display: block;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--ink);
    overflow-wrap: anywhere;
  }
  .task {
    display: block;
    font-size: 10.5px;
    color: var(--ink-mute);
  }

  @container insights (max-width: 560px) {
    .cost-table { table-layout: auto; min-width: 0; }
    .avg-usage { display: none; }
  }

  /* On a phone each model becomes a stacked row: name on top, figures beneath. */
  @container insights (max-width: 420px) {
    .cost-table, .cost-table tbody { display: block; min-width: 0; }
    .cost-table thead { display: none; }
    .cost-table tr {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto auto;
      column-gap: 12px;
      padding: 10px 0;
      border-bottom: 1px solid var(--line);
    }
    .cost-table tbody tr:last-child { border-bottom: 0; }
    .cost-table th,
    .cost-table td { padding: 0; border: 0; }
    .cost-table th[scope="row"] { grid-column: 1 / -1; width: auto; padding: 0 0 6px; }
    .cost-table .avg-usage,
    .cost-table .avg-cost { display: none; }
    .cost-table td.num { text-align: left; }
    .cost-table td.num:last-child { text-align: right; }
    .model { overflow-wrap: anywhere; }
    .readout { text-align: left; }
  }

  .foot {
    margin: 12px 0 0;
    font-size: 11px;
    color: var(--ink-mute);
    line-height: 1.45;
  }

</style>
