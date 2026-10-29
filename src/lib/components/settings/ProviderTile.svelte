<script lang="ts">
  import { monogram } from '../../customProviderPresets';

  let {
    name,
    logo = null,
    plate = 'none',
    color = '#6B7280',
    mark,
    big = false,
  }: { name: string; logo?: string | null; plate?: 'bleed' | 'none'; color?: string; mark?: string; big?: boolean } = $props();
</script>

<span class="tile" class:big class:bleed={!!logo && plate === 'bleed'} class:logo={!!logo} style:--tile={color} aria-hidden="true">
  {#if logo}{@html logo}{:else}{mark ?? monogram(name)}{/if}
</span>

<style>
  .tile { --tile: #6b7280; flex: none; width: 36px; height: 36px; border-radius: 10px; display: flex; align-items: center; justify-content: center; overflow: hidden; line-height: 1; background: var(--tile); color: #fff; font-size: 12px; font-weight: 700; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.14); }
  .tile.logo { background: #fff; color: #111; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.08); }
  .tile.bleed { background: transparent; box-shadow: none; }
  .tile :global(svg) { width: 58%; height: 58%; display: block; }
  .tile.bleed :global(svg) { width: 100%; height: 100%; }
  .tile :global(.mark-dark) { display: none !important; }
  .tile :global(.mark-light) { display: block !important; }
  .tile.big { width: 44px; height: 44px; border-radius: 12px; font-size: 14px; }
</style>
