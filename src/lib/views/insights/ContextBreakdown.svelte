<script lang="ts">
  import ContextIcon from '../../components/ContextIcon.svelte';
  import { fmtNumber } from './helpers';
  import { contextShares, fmtPercent } from './analytics';
  import type { InsightsContextUsage } from './types';

  interface ContextMeta {
    id: number;
    name: string;
    icon: string | null;
    color: string | null;
    is_everywhere?: boolean;
  }

  let {
    usage = [],
    totalWords,
    contexts,
    onpick,
  }: {
    usage?: InsightsContextUsage[];
    totalWords: number;
    contexts: ContextMeta[];
    onpick: (contextId: number) => void;
  } = $props();

  const byId = $derived(new Map(contexts.map((c) => [c.id, c])));
  const rows = $derived(
    contextShares(usage, totalWords).map((row, i) => {
      const meta = row.contextId === null ? null : byId.get(row.contextId) ?? null;
      return {
        ...row,
        key: row.contextId === null ? 'other' : `ctx-${row.contextId}`,
        // A context deleted since these words were dictated has no metadata left.
        name: meta?.name ?? (row.contextId === null ? 'Other' : 'Deleted context'),
        meta,
        color: shade(i),
      };
    }),
  );
  const top = $derived(rows[0] ?? null);

  let hover = $state<string | null>(null);

  /** Monochrome ramp from the accent, so the bar reads in every accent theme. */
  function shade(index: number): string {
    const steps = [100, 70, 48, 32, 20, 12];
    return `color-mix(in srgb, var(--accent) ${steps[Math.min(index, steps.length - 1)]}%, var(--paper-3))`;
  }
</script>

<section class="card contexts-card" aria-labelledby="contexts-h">
  <header class="card-head">
    <div>
      <h2 class="card-h" id="contexts-h">Where you dictate</h2>
      <p class="card-sub">
        {#if top && top.contextId !== null}
          {fmtPercent(top.share)} of your words went to {top.name}
        {:else}
          Words by context group
        {/if}
      </p>
    </div>
  </header>

  {#if rows.length === 0}
    <p class="empty">Context groups you dictate in will show up here.</p>
  {:else}
    <div class="stack" aria-hidden="true">
      {#each rows as row, i (row.key)}
        <span
          class="seg"
          class:dim={hover !== null && hover !== row.key}
          style:flex-grow={row.words}
          style:background={row.color}
          style:--i={i}
        ></span>
      {/each}
    </div>

    <ul class="ctx-list">
      {#each rows as row, i (row.key)}
        <li style:--i={i}>
          {#if row.contextId !== null && row.meta}
            {@const contextId = row.contextId}
            <button
              type="button"
              class="ctx-line"
              class:dim={hover !== null && hover !== row.key}
              onpointerenter={() => (hover = row.key)}
              onpointerleave={() => (hover = null)}
              onfocus={() => (hover = row.key)}
              onblur={() => (hover = null)}
              onclick={() => onpick(contextId)}
              title="Show insights for {row.name}"
            >
              {@render line(row)}
            </button>
          {:else}
            <div
              class="ctx-line static"
              class:dim={hover !== null && hover !== row.key}
              role="presentation"
              onpointerenter={() => (hover = row.key)}
              onpointerleave={() => (hover = null)}
            >
              {@render line(row)}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#snippet line(row: (typeof rows)[number])}
  <span class="swatch" style:background={row.color} aria-hidden="true"></span>
  <span class="ctx-icon" style:color={row.meta?.color ?? undefined} aria-hidden="true">
    {#if row.meta?.is_everywhere}
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><circle cx="12" cy="12" r="8"/><path d="M4 12h16M12 4c2 2.3 3 5 3 8s-1 5.7-3 8c-2-2.3-3-5-3-8s1-5.7 3-8Z"/></svg>
    {:else if row.meta}
      <ContextIcon icon={row.meta.icon} size={13} />
    {:else}
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><circle cx="6" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18" cy="12" r="1.2"/></svg>
    {/if}
  </span>
  <span class="ctx-name">{row.name}</span>
  <span class="ctx-words">{fmtNumber(row.words)}</span>
  <span class="ctx-share">{fmtPercent(row.share)}</span>
{/snippet}

<style>
  .stack {
    display: flex;
    gap: 3px;
    height: 10px;
    margin: 2px 0 14px;
    border-radius: 999px;
    overflow: hidden;
    animation: stack-in 760ms var(--ui-ease-out) both 120ms;
  }

  @keyframes stack-in {
    from { clip-path: inset(0 100% 0 0 round 999px); }
    to { clip-path: inset(0 0 0 0 round 999px); }
  }

  .seg {
    flex-basis: 0;
    min-width: 4px;
    height: 100%;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out);
  }
  .seg.dim { opacity: 0.3; }

  .ctx-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .ctx-list li {
    min-width: 0;
    animation: row-in 420ms var(--ui-ease-out) both;
    animation-delay: calc(var(--i) * 40ms + 160ms);
  }

  @keyframes row-in {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: none; }
  }

  .ctx-line {
    display: grid;
    grid-template-columns: 8px 16px minmax(0, 1fr) auto 38px;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 32px;
    padding: 0 8px;
    margin: 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--ink);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      opacity var(--ui-duration-fast) var(--ui-ease-out);
  }
  .ctx-line.static { cursor: default; }
  .ctx-line:hover,
  .ctx-line:focus-visible { background: var(--control-hover); }
  .ctx-line:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .ctx-line.dim { opacity: 0.6; }

  .swatch { width: 8px; height: 8px; border-radius: 2px; }
  .ctx-icon { display: grid; place-items: center; color: var(--ink-mute); }

  .ctx-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
  }

  .ctx-words,
  .ctx-share {
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    white-space: nowrap;
  }
  .ctx-words { color: var(--ink-mute); }
  .ctx-share { color: var(--ink); font-weight: 500; }

  .empty { margin: 0; font-size: 11.5px; color: var(--ink-mute); }

  @container insights (max-width: 420px) {
    .ctx-line { grid-template-columns: 8px 16px minmax(0, 1fr) 38px; }
    .ctx-words { display: none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .stack, .ctx-list li { animation: none; }
  }
</style>
