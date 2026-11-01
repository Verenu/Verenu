<script lang="ts">
  import type { Snippet } from 'svelte';
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../motion';

  let {
    label,
    open = $bindable(false),
    id,
    class: className = '',
    children,
  }: {
    label: string;
    open?: boolean;
    /** Id of the revealed region, so a test or `aria-controls` consumer can find it. */
    id: string;
    class?: string;
    children: Snippet;
  } = $props();
</script>

<div class="disclosure {className}">
  <button
    type="button"
    class="disclosure-trigger ui-focus-ring"
    aria-expanded={open}
    aria-controls={open ? id : undefined}
    onclick={() => (open = !open)}
  >
    <span>{label}</span>
    <svg class="ui-chevron" class:open width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
  </button>
  {#if open}
    <div {id} class="disclosure-body" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .disclosure-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 28px;
    margin: 0;
    padding: 4px 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--ink-soft);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .disclosure-trigger:hover { color: var(--ink-strong); }
</style>
