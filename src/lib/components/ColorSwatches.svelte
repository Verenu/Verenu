<script lang="ts">
  import { isHexColor } from '../contextIcon';

  type Choice = { id: string; label: string; value: string };
  let { label, choices, value, onchange, defaultLabel = null, customFallback = '#7c9ebc' }: {
    label: string;
    choices: readonly Choice[];
    value: string | null;
    onchange: (color: string | null) => void;
    defaultLabel?: string | null;
    customFallback?: string;
  } = $props();

  const norm = (color: string | null) => color?.toLowerCase() ?? null;
  const isPreset = $derived(choices.some((choice) => norm(choice.value) === norm(value)));
  const customActive = $derived(!!value && !isPreset && isHexColor(value));
</script>

<div class="swatch-row" role="group" aria-label={label}>
  {#if defaultLabel}
    <button type="button" class="swatch swatch-default" class:is-selected={!value} aria-pressed={!value} aria-label={defaultLabel} title={defaultLabel} onclick={() => onchange(null)}>
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
    </button>
  {/if}
  {#each choices as choice (choice.id)}
    <button type="button" class="swatch" class:is-selected={norm(value) === norm(choice.value)} aria-pressed={norm(value) === norm(choice.value)} style:background={choice.value} aria-label={choice.label} title={choice.label} onclick={() => onchange(choice.value)}></button>
  {/each}
  <span class="swatch swatch-custom" class:is-selected={customActive} style:background={customActive ? value : undefined} title="Custom color">
    <input type="color" aria-label="{label}: custom color" value={value && isHexColor(value) ? value : customFallback} oninput={(event) => onchange(event.currentTarget.value)} />
    {#if !customActive}
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>
    {/if}
  </span>
</div>

<style>
  .swatch-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 3px 2px; }
  .swatch {
    position: relative; display: grid; place-items: center; flex-shrink: 0;
    width: 24px; height: 24px; padding: 0; border-radius: 999px; cursor: pointer;
    border: 1px solid color-mix(in srgb, var(--ink) 14%, transparent);
    transition: transform .12s ease, box-shadow .12s ease;
  }
  .swatch:hover { transform: scale(1.1); }
  .swatch:focus-within, button.swatch:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .swatch.is-selected { box-shadow: 0 0 0 2px var(--bg-elev), 0 0 0 4px var(--ink-soft); }
  .swatch-default { background: var(--bg-elev); color: var(--ink-faint); }
  .swatch-custom { background: var(--bg-elev); color: var(--ink-mute); border-style: dashed; border-color: var(--line-strong); }
  .swatch-custom.is-selected { border-style: solid; }
  .swatch-custom input[type="color"] { position: absolute; inset: 0; width: 100%; height: 100%; padding: 0; border: 0; opacity: 0; cursor: pointer; }
  .swatch-custom svg { pointer-events: none; }
  @media (pointer: coarse) { .swatch { width: 30px; height: 30px; } .swatch-row { gap: 8px; } }
  @media (prefers-reduced-motion: reduce) { .swatch { transition: none; } }
</style>
