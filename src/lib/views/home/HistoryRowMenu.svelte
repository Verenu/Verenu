<script lang="ts">
  import { fly } from 'svelte/transition';
  import Dropdown from '../../components/Dropdown.svelte';
  import { icons } from '../../icons';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';

  let { copied = false, oncopy, onviewlogs, ondelete }: {
    copied?: boolean;
    oncopy: () => void;
    onviewlogs: () => void;
    ondelete: () => void;
  } = $props();

  let open = $state(false);
  let confirmingDelete = $state(false);
  const menuId = `history-row-menu-${Math.random().toString(36).slice(2, 9)}`;

  $effect(() => {
    if (!open) confirmingDelete = false;
  });

  function choose(action: () => void) {
    open = false;
    action();
  }

  function onDeleteClick() {
    if (!confirmingDelete) {
      confirmingDelete = true;
      return;
    }
    choose(ondelete);
  }
</script>

<Dropdown bind:open closeSelector=".history-row-menu" optionSelector=".history-row-menu-item">
  <div class="ui-dropdown history-row-menu" class:open class:copied>
    <button
      type="button"
      class="row-menu-btn"
      aria-haspopup="menu"
      aria-expanded={open}
      aria-controls={open ? menuId : undefined}
      aria-label="More actions"
      title="More actions"
      onclick={() => (open = !open)}
    >
      {#if copied}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.check}</svg>
      {:else}
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <circle cx="12" cy="5" r="1.8" /><circle cx="12" cy="12" r="1.8" /><circle cx="12" cy="19" r="1.8" />
        </svg>
      {/if}
    </button>
    {#if open}
      <div
        id={menuId}
        class="ui-dropdown-menu ui-dropdown-menu--padded history-row-menu-list"
        role="menu"
        aria-label="Dictation actions"
        transition:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast) }}
      >
        <button type="button" role="menuitem" class="ui-dropdown-option history-row-menu-item" onclick={() => choose(oncopy)}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.copy}</svg>
          Copy to clipboard
        </button>
        <button type="button" role="menuitem" class="ui-dropdown-option history-row-menu-item" onclick={() => choose(onviewlogs)}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h10" /></svg>
          View logs
        </button>
        <button
          type="button"
          role="menuitem"
          class="ui-dropdown-option history-row-menu-item history-row-menu-danger"
          class:confirming={confirmingDelete}
          onclick={onDeleteClick}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4h8v2m-9 0 1 14h8l1-14" /></svg>
          {confirmingDelete ? 'Confirm delete' : 'Delete'}
        </button>
      </div>
    {/if}
  </div>
</Dropdown>

<style>
  .history-row-menu { display: flex; align-self: start; }

  .row-menu-btn {
    all: unset;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--ink-mute);
    opacity: 0;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast), background var(--ui-duration-fast);
  }
  .row-menu-btn svg { width: 14px; height: 14px; }
  :global(.day-row:hover) .row-menu-btn,
  :global(.day-row:focus-within) .row-menu-btn,
  .row-menu-btn:focus-visible,
  .history-row-menu.open .row-menu-btn {
    opacity: 1;
  }
  .history-row-menu.copied .row-menu-btn { opacity: 1; color: var(--accent); }
  .row-menu-btn:hover,
  .history-row-menu.open .row-menu-btn { color: var(--ink-strong); background: var(--control-active); }
  .row-menu-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @media (hover: none) {
    .row-menu-btn { opacity: 1; }
  }

  .history-row-menu-list { min-width: 176px; z-index: 30; }
  .history-row-menu-item { display: flex; align-items: center; gap: 8px; white-space: nowrap; font-size: 13px; }
  .history-row-menu-item svg { width: 14px; height: 14px; flex: none; }
  .history-row-menu-danger { color: var(--danger); }
  .history-row-menu-danger.confirming { background: var(--danger-bg); font-weight: 500; }
</style>
