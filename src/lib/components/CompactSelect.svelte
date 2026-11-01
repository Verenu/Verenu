<script lang="ts" generics="T extends string | number">
  import { fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import Dropdown from './Dropdown.svelte';
  import { MOTION_MS, motionMs } from '../motion';

  let {
    value,
    options,
    label,
    onchange,
    class: className = '',
    menuId,
    width,
    menuMinWidth,
  }: {
    value: T;
    options: { value: T; label: string; description?: string }[];
    label: string;
    onchange: (value: T) => void;
    class?: string;
    menuId?: string;
    /** Fixed trigger width in px, so a changing label never moves its neighbors. */
    width?: number;
    menuMinWidth?: number;
  } = $props();

  let open = $state(false);
  const id = `compact-select-${Math.random().toString(36).slice(2, 9)}`;
  const current = $derived(options.find((option) => option.value === value)?.label ?? '');

  function pick(next: T) {
    open = false;
    if (next !== value) onchange(next);
  }
</script>

<Dropdown bind:open closeSelector={`.${id}`}>
  <div class="ui-dropdown compact-select {id} {className}" class:fixed={width !== undefined}>
    <button
      type="button"
      class="ui-dropdown-trigger ui-dropdown-trigger--compact"
      style:width={width !== undefined ? `${width}px` : undefined}
      aria-label={label}
      aria-expanded={open}
      aria-haspopup="listbox"
      aria-controls={menuId}
      onclick={() => (open = !open)}
    >
      <span class="compact-select-label">{current}</span>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
    </button>
    {#if open}
      <div
        id={menuId}
        class="ui-dropdown-menu ui-dropdown-menu--padded compact-select-menu scroll-styled scroll-thumb-elev"
        style:min-width={menuMinWidth !== undefined ? `${menuMinWidth}px` : undefined}
        role="listbox"
        tabindex="-1"
        aria-label={label}
        in:fly={{ y: 5, duration: motionMs(MOTION_MS.fast), easing: expoOut }}
        out:fly={{ y: 3, duration: motionMs(110), easing: expoOut }}
      >
        {#each options as option (option.value)}
          <button
            type="button"
            class="ui-dropdown-option"
            class:active={option.value === value}
            role="option"
            aria-selected={option.value === value}
            onclick={() => pick(option.value)}
          >
            {option.label}
            {#if option.description}<span class="ui-dropdown-option-desc">{option.description}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</Dropdown>

<style>
  .compact-select {
    --ui-dropdown-trigger-height: 28px;
  }

  .compact-select .ui-dropdown-trigger {
    gap: 6px;
    white-space: nowrap;
  }

  .compact-select.fixed .ui-dropdown-trigger {
    justify-content: space-between;
  }

  .compact-select-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .compact-select-menu {
    max-width: min(320px, 100cqw);
    min-width: 100%;
    width: max-content;
  }

  .compact-select-menu :global(.ui-dropdown-option) {
    white-space: normal;
  }

  @container settings-panel (max-width: 440px) {
    .compact-select .ui-dropdown-trigger {
      min-height: 44px;
    }

    .compact-select-menu :global(.ui-dropdown-option) {
      min-height: 44px;
      padding-block: 10px;
    }
  }
</style>
