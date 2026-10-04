<script lang="ts">
  type Colors = { background: string; foreground: string; sidebar: string; surface: string };

  let {
    colors,
    accent = null,
    split = null,
  }: {
    colors: Colors;
    accent?: string | null;
    /** Second palette drawn on the right half, for the System card. */
    split?: Colors | null;
  } = $props();
</script>

{#snippet scene(c: Colors, a: string | null)}
  <div class="scene" style:background={c.background} style:--fg={c.foreground}>
    <div class="side" style:background={c.sidebar}>
      <i class="dot" style:background={a ?? c.foreground}></i>
      <i class="bar"></i><i class="bar short"></i><i class="bar"></i>
    </div>
    <div class="main">
      <i class="bar title"></i>
      <div class="card" style:background={c.surface}>
        <i class="bar"></i><i class="bar short"></i>
      </div>
      <i class="chip" style:background={a ?? c.foreground}></i>
    </div>
  </div>
{/snippet}

<div class="preview" aria-hidden="true">
  {@render scene(colors, accent)}
  {#if split}
    <div class="half">{@render scene(split, accent)}</div>
  {/if}
</div>

<style>
  .preview { border-radius: 8px; aspect-ratio: 2.2 / 1; min-height: 64px; overflow: hidden; position: relative; width: 100%; }
  .half { clip-path: inset(0 0 0 50%); inset: 0; position: absolute; }
  .scene { display: flex; height: 100%; width: 100%; }
  .side { align-items: flex-start; display: flex; flex: 0 0 26%; flex-direction: column; gap: 5px; padding: 9px 8px; }
  .main { display: flex; flex: 1; flex-direction: column; gap: 6px; min-width: 0; padding: 9px 10px 8px; }
  .bar { background: var(--fg); border-radius: 2px; display: block; height: 4px; opacity: 0.4; width: 100%; }
  .bar.short { width: 60%; }
  .bar.title { height: 5px; opacity: 0.85; width: 55%; }
  .dot { border-radius: 50%; display: block; height: 8px; margin-bottom: 3px; width: 8px; }
  .card { border: 1px solid color-mix(in srgb, var(--fg) 14%, transparent); border-radius: 4px; display: flex; flex-direction: column; gap: 4px; padding: 7px; }
  .chip { border-radius: 3px; display: block; height: 8px; width: 30px; }
</style>
