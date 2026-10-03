<script lang="ts">
  import { icons } from '../icons';
  import { parseContextIcon } from '../contextIcon';

  let { icon = null, size = 14 }: { icon?: string | null; size?: number } = $props();
  const custom = $derived(parseContextIcon(icon));
  const pair = $derived(custom ? Array.from(new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(custom.text)).length > 1 : false);
  const glyph = $derived(icon ? icons[icon as keyof typeof icons] : undefined);
</script>

{#if custom?.kind === 'emoji'}
  <span class="custom-context-icon emoji" style:width="{size + 6}px" style:height="{size + 6}px" style:font-size="{pair ? size * 0.62 : size}px" aria-hidden="true">{custom.text}</span>
{:else if custom}
  <span class="custom-context-icon letters" style:width="{size + 6}px" style:height="{size + 6}px" style:font-size="{size * (pair && /\p{Extended_Pictographic}/u.test(custom.text) ? 0.56 : 0.7)}px" style:--badge-bg={custom.background} style:--badge-fg={custom.foreground} aria-hidden="true">{custom.text}</span>
{:else}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    {@html glyph ?? '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 8h10M7 12h6M7 16h3"/>'}
  </svg>
{/if}

<style>
  .custom-context-icon { display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0; border-radius: 4px; font-weight: 650; line-height: 1; white-space: nowrap; letter-spacing: -.03em; }
  .emoji { letter-spacing: 0; overflow: hidden; }
  /* Softened badge: the chosen colors stay distinct but sit lightly on any theme. */
  .letters { background: color-mix(in srgb, var(--badge-bg) 82%, transparent); color: color-mix(in srgb, var(--badge-fg) 94%, transparent); }
</style>
