<script lang="ts" generics="T extends string | number">
  // The compact select used in the Insights header: a bordered compact
  // trigger with a chevron and a padded menu that flies in. Use this for a
  // small single-choice control; DESIGN.md "Dropdowns" describes the anatomy.
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
  }: {
    value: T;
    options: { value: T; label: string }[];
    label: string;
    onchange: (value: T) => void;
    class?: string;
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
  <div class="ui-dropdown compact-select {id} {className}">
    <button
      type="button"
      class="ui-dropdown-trigger ui-dropdown-trigger--compact"
      aria-label={label}
      aria-expanded={open}
      aria-haspopup="listbox"
      onclick={() => (open = !open)}
    >
      {current}
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
    </button>
    {#if open}
      <div
        class="ui-dropdown-menu ui-dropdown-menu--padded"
        role="listbox"
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
          >{option.label}</button>
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
</style>
