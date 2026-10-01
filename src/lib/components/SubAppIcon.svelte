<script lang="ts">
  // A sub-app's chosen icon, or its app's icon when none was picked.
  import AppIcon from './AppIcon.svelte';
  import { icons } from '../icons';
  import type { ContextSubApp } from '../stores';

  let { subApp, label, size = 16 }: { subApp: Pick<ContextSubApp, 'icon' | 'executable'>; label: string; size?: number } = $props();
  const glyph = $derived(subApp.icon ? icons[subApp.icon as keyof typeof icons] : undefined);
</script>

{#if glyph}
  <svg class="sub-app-glyph" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html glyph}</svg>
{:else}
  <AppIcon exe={subApp.executable} {label} {size} />
{/if}

<style>
  .sub-app-glyph {
    color: var(--ink-soft);
    flex-shrink: 0;
  }
</style>
