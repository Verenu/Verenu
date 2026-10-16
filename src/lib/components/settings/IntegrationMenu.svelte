<script lang="ts">
  import { fly } from 'svelte/transition';
  import Dropdown from '../Dropdown.svelte';
  import { motionMs, motionPx, MOTION_MS, MOTION_PX } from '../../motion';

  let {
    label,
    triggerId,
    items,
  }: {
    label: string;
    triggerId: string;
    items: { label: string; icon: string; disabled?: boolean; onselect: () => void }[];
  } = $props();

  let open = $state(false);

  function choose(item: (typeof items)[number]) {
    if (item.disabled) return;
    open = false;
    item.onselect();
  }
</script>

<Dropdown bind:open closeSelector=".integration-menu">
  <div class="ui-dropdown integration-menu">
    <button
      id={triggerId}
      type="button"
      class="icon-btn menu-trigger"
      aria-label={label}
      aria-haspopup="menu"
      aria-expanded={open}
      onclick={() => (open = !open)}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></svg>
    </button>

    {#if open}
      <div
        class="ui-dropdown-menu ui-dropdown-menu--padded integration-menu-popover"
        role="menu"
        aria-label={label}
        in:fly={{ y: motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast) }}
        out:fly={{ y: motionPx(MOTION_PX.nudge) * 0.6, duration: motionMs(120) }}
      >
        {#each items as item (item.label)}
          <button
            type="button"
            role="menuitem"
            class="ui-dropdown-option menu-item"
            aria-disabled={item.disabled ? 'true' : undefined}
            onclick={() => choose(item)}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html item.icon}</svg>
            <span>{item.label}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</Dropdown>

<style>
  .menu-trigger { width: 28px; height: 28px; }
  .integration-menu-popover { min-width: 184px; }
  .menu-item { align-items: center; display: flex; gap: 10px; }
  .menu-item svg { flex: 0 0 auto; }
  .menu-item[aria-disabled='true'] { cursor: default; opacity: var(--ui-disabled-opacity); }
  .menu-item[aria-disabled='true']:hover { background: transparent; }
</style>
