<script lang="ts">
  import Dropdown from '../Dropdown.svelte';
  import { popoverPop, MOTION_MS } from '../../motion';

  let {
    label,
    triggerId,
    items,
  }: {
    label: string;
    triggerId: string;
    items: { label: string; icon: string; disabled?: boolean; danger?: boolean; onselect: () => void }[];
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
      class:open
      aria-label={label}
      aria-haspopup="menu"
      aria-expanded={open}
      onclick={() => (open = !open)}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>
    </button>

    {#if open}
      <div
        class="ui-dropdown-menu ui-dropdown-menu--padded integration-menu-popover"
        role="menu"
        aria-label={label}
        in:popoverPop={{ duration: MOTION_MS.base }}
        out:popoverPop={{ duration: 110 }}
      >
        {#each items as item (item.label)}
          {#if item.danger}<div class="menu-rule" role="separator"></div>{/if}
          <button
            type="button"
            role="menuitem"
            class="ui-dropdown-option menu-item"
            class:danger={item.danger}
            aria-disabled={item.disabled ? 'true' : undefined}
            onclick={() => choose(item)}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html item.icon}</svg>
            <span>{item.label}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</Dropdown>

<style>
  .menu-trigger { height: 30px; width: 30px; }
  .menu-trigger.open { background: var(--control-active); color: var(--ink-strong); }
  .integration-menu-popover.ui-dropdown-menu { gap: 1px; left: auto; min-width: 176px; padding: 5px; right: 0; top: calc(100% + 6px); }
  .integration-menu-popover .menu-item { align-items: center; display: flex; gap: 10px; padding: 8px 10px; }
  .menu-item svg { color: var(--ink-mute); flex: 0 0 auto; transition: color var(--ui-duration-fast) var(--ui-ease-out); }
  .menu-item:hover svg, .menu-item:focus-visible svg { color: var(--ink-strong); }
  .integration-menu-popover .menu-item:focus-visible { background: var(--control-hover); box-shadow: none; color: var(--ink); outline: none; }
  .menu-item.danger, .menu-item.danger svg { color: var(--danger); }
  .integration-menu-popover .menu-item.danger:hover, .integration-menu-popover .menu-item.danger:focus-visible { background: var(--danger-bg); color: var(--danger); }
  .menu-rule { background: var(--line); height: 1px; margin: 4px 6px; }
  .menu-item[aria-disabled='true'] { cursor: default; opacity: var(--ui-disabled-opacity); }
  .integration-menu-popover .menu-item[aria-disabled='true']:hover { background: transparent; }
</style>
