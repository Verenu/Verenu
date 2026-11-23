<script lang="ts">
  type Colors = { background: string; foreground: string; sidebar: string; surface: string };

  let {
    colors,
    accent = null,
    split = null,
    compact = false,
  }: {
    colors: Colors;
    accent?: string | null;
    split?: Colors | null;
    /** Card thumbnails drop the second text line so the scene reads at small sizes. */
    compact?: boolean;
  } = $props();
</script>

{#snippet scene(c: Colors, a: string | null)}
  <div class="scene" style:background={c.background} style:--bg={c.background} style:--fg={c.foreground} style:--accent-color={a ?? c.foreground}>
    <div class="side" style:background={c.sidebar}>
      <i class="dot"></i>
      <i class="bar nav-active"></i><i class="bar short"></i>{#if !compact}<i class="bar"></i>{/if}
    </div>
    <div class="main">
      <i class="bar title"></i>
      <div class="card" style:background={c.surface}>
        <i class="bar"></i>{#if !compact}<i class="bar short"></i>{/if}
      </div>
      <div class="foot">
        <i class="chip"></i>
        <i class="switch"><i class="knob"></i></i>
      </div>
    </div>
  </div>
{/snippet}

<div class="preview" class:compact aria-hidden="true">
  {@render scene(colors, accent)}
  {#if split}
    <div class="half">{@render scene(split, accent)}</div>
  {/if}
</div>

<style>
  .preview { border-radius: 8px; aspect-ratio: 2.2 / 1; min-height: 64px; overflow: hidden; position: relative; width: 100%; }
  .preview.compact { aspect-ratio: 1.75 / 1; min-height: 0; }
  .half { clip-path: inset(0 0 0 50%); inset: 0; position: absolute; }
  .scene { display: flex; height: 100%; transition: background-color var(--ui-duration-base) var(--ui-ease-out); width: 100%; }
  .side { align-items: flex-start; display: flex; flex: 0 0 26%; flex-direction: column; gap: 5px; padding: 9% 7%; }
  .main { display: flex; flex: 1; flex-direction: column; gap: 6px; min-width: 0; padding: 9% 9% 7%; }
  .bar { background: var(--fg); border-radius: 2px; display: block; flex: none; height: 4px; opacity: 0.32; width: 100%; }
  .bar.short { width: 60%; }
  .bar.nav-active { background: var(--accent-color); opacity: 0.85; width: 80%; }
  .bar.title { height: 5px; opacity: 0.85; width: 55%; }
  .dot { background: var(--accent-color); border-radius: 50%; display: block; flex: none; height: 7px; margin-bottom: 3px; width: 7px; }
  .card { border: 1px solid color-mix(in srgb, var(--fg) 14%, transparent); border-radius: 4px; display: flex; flex-direction: column; gap: 4px; padding: 7%; }
  .foot { align-items: center; display: flex; gap: 6px; justify-content: space-between; margin-top: auto; }
  .chip { background: var(--accent-color); border-radius: 3px; display: block; height: 8px; width: 34%; }
  .switch { background: var(--accent-color); border-radius: 999px; display: block; height: 8px; position: relative; width: 15px; }
  .knob { background: var(--bg); border-radius: 50%; display: block; height: 6px; position: absolute; right: 1px; top: 1px; width: 6px; }
  .compact .side { gap: 4px; }
  .compact .main { gap: 5px; }
  .compact .dot { height: 6px; width: 6px; }
  @media (prefers-reduced-motion: reduce) {
    .scene { transition-duration: 1ms; }
  }
</style>
