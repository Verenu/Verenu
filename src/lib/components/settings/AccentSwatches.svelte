<script lang="ts">
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../../motion';
  import { normalizeAccentColor } from '../../accentTheme';

  let {
    value,
    disabled = false,
    onchange,
  }: {
    value: string | null;
    disabled?: boolean;
    onchange: (value: string | null) => void | Promise<void>;
  } = $props();

  const presets = [
    { label: 'Blue', value: '#4F7FD8' },
    { label: 'Teal', value: '#398E94' },
    { label: 'Green', value: '#4F9C78' },
    { label: 'Amber', value: '#C9912E' },
    { label: 'Orange', value: '#D9733B' },
    { label: 'Rose', value: '#D2637A' },
    { label: 'Violet', value: '#8B6FD6' },
    { label: 'Slate', value: '#64748B' },
  ] as const;

  const presetMatch = $derived(presets.find((preset) => preset.value === value) ?? null);
  const custom = $derived(value !== null && !presetMatch);
  const currentLabel = $derived(value === null ? 'Default' : presetMatch?.label ?? 'Custom');

  let customOpen = $state(false);
  let draft = $state('');
  let invalid = $state(false);
  let colorInput: HTMLInputElement | null = $state(null);

  function defaultAccentColor(): string {
    if (typeof document === 'undefined') return '#000000';
    const styles = getComputedStyle(document.documentElement);
    return normalizeAccentColor(styles.getPropertyValue('--accent')) ?? '#000000';
  }

  $effect(() => {
    if (!customOpen) draft = (value ?? '').replace(/^#/, '');
  });

  async function choose(next: string | null) {
    if (disabled) return;
    invalid = false;
    await onchange(next);
  }

  function toggleCustom() {
    if (disabled) return;
    if (!customOpen) draft = (value ?? defaultAccentColor()).replace(/^#/, '');
    invalid = false;
    customOpen = !customOpen;
  }

  async function commitDraft() {
    const normalized = normalizeAccentColor(`#${draft.replace(/^#+/, '')}`);
    if (!normalized) {
      invalid = true;
      return;
    }
    draft = normalized.slice(1);
    await choose(normalized);
  }

  function handleHexKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    void commitDraft();
  }
</script>

<div class="accent-swatches" data-setting-target="appearance-accent">
  <div class="swatch-row" role="radiogroup" aria-label="Accent color">
    <button
      class="accent-swatch default"
      class:selected={value === null}
      role="radio"
      aria-checked={value === null}
      aria-disabled={disabled}
      aria-label="Default"
      title="Default"
      onclick={() => choose(null)}
    >
      <span class="fill"></span>
      <svg class="tick" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
    </button>
    {#each presets as preset (preset.value)}
      {@const selected = value === preset.value}
      <button
        class="accent-swatch"
        class:selected
        role="radio"
        aria-checked={selected}
        aria-disabled={disabled}
        aria-label={preset.label}
        title={preset.label}
        style:--swatch={preset.value}
        onclick={() => choose(preset.value)}
      >
        <span class="fill"></span>
        <svg class="tick" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
      </button>
    {/each}
    <span class="divider" aria-hidden="true"></span>
    <button
      class="accent-swatch custom"
      class:selected={custom}
      class:has-color={custom}
      aria-pressed={custom}
      aria-expanded={customOpen}
      aria-controls="accent-custom-panel"
      aria-disabled={disabled}
      aria-label="Custom accent color"
      title="Custom color"
      style:--swatch={custom ? value : undefined}
      onclick={toggleCustom}
    >
      <span class="fill"></span>
      {#if custom}
        <svg class="tick" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
      {:else}
        <svg class="plus" viewBox="0 0 12 12" aria-hidden="true"><path d="M6 2.5v7M2.5 6h7"/></svg>
      {/if}
    </button>
  </div>
  <span class="current" aria-live="polite">{currentLabel}{#if value}<span class="hex">{value}</span>{/if}</span>
</div>

{#if customOpen}
  <div
    id="accent-custom-panel"
    class="custom-panel"
    transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}
  >
    <div class="custom-row">
      <button class="color-well" aria-label="Open color picker" onclick={() => colorInput?.click()} style:--swatch={normalizeAccentColor(`#${draft}`) ?? value ?? undefined}>
        <span></span>
      </button>
      <input
        bind:this={colorInput}
        class="native-color-input"
        type="color"
        tabindex="-1"
        value={normalizeAccentColor(`#${draft}`)?.toLowerCase() ?? '#000000'}
        aria-label="Custom accent color picker"
        oninput={(event) => { draft = (event.currentTarget as HTMLInputElement).value.slice(1).toUpperCase(); invalid = false; }}
        onchange={(event) => choose((event.currentTarget as HTMLInputElement).value.toUpperCase())}
      />
      <div class="hex-field" class:invalid>
        <span aria-hidden="true">#</span>
        <input
          bind:value={draft}
          aria-label="Accent color hex value"
          aria-invalid={invalid}
          maxlength="7"
          spellcheck="false"
          autocomplete="off"
          oninput={() => { invalid = false; }}
          onkeydown={handleHexKeydown}
        />
      </div>
      <button class="btn-primary btn-compact" aria-disabled={disabled} onclick={() => void commitDraft()}>Apply</button>
    </div>
    {#if invalid}
      <div class="color-error" role="alert">Enter a six-digit hex color, like #4F7FD8.</div>
    {/if}
  </div>
{/if}

<style>
  .accent-swatches { align-items: center; display: flex; flex-wrap: wrap; gap: 10px 16px; justify-content: space-between; }
  .swatch-row { align-items: center; display: flex; flex-wrap: wrap; gap: 6px; }
  .divider { background: var(--line); height: 18px; margin: 0 4px; width: 1px; }
  .accent-swatch {
    --swatch: var(--accent-theme-default);
    --ring: transparent;
    align-items: center;
    background: transparent;
    border: 0;
    border-radius: 50%;
    cursor: pointer;
    display: inline-flex;
    flex: none;
    height: 30px;
    justify-content: center;
    padding: 0;
    position: relative;
    transition: transform var(--ui-duration-fast) var(--ui-ease-out);
    width: 30px;
  }
  /* The ring is its own layer so selection only animates opacity and scale. */
  .accent-swatch::before {
    border: 2px solid var(--swatch);
    border-radius: 50%;
    content: '';
    inset: 0;
    opacity: 0;
    position: absolute;
    transform: scale(0.8);
    transition: opacity var(--ui-duration-base) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .accent-swatch .fill {
    background: var(--swatch);
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ink) 14%, transparent);
    inset: 4px;
    position: absolute;
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .accent-swatch:hover:not([aria-disabled='true']) .fill { transform: scale(1.08); }
  .accent-swatch:active:not([aria-disabled='true']) { transform: scale(0.94); }
  .accent-swatch.selected::before { opacity: 1; transform: scale(1); }
  .accent-swatch.selected .fill { transform: scale(0.86); }
  .accent-swatch[aria-disabled='true'] { cursor: default; opacity: 0.6; }
  .accent-swatch:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .default { --swatch: var(--accent-theme-default); }
  .custom:not(.has-color) { --swatch: var(--ink-mute); }
  .custom:not(.has-color) .fill {
    background: conic-gradient(from 210deg, #d2637a, #c9912e, #4f9c78, #398e94, #4f7fd8, #8b6fd6, #d2637a);
  }
  .tick, .plus {
    fill: none;
    height: 12px;
    opacity: 0;
    pointer-events: none;
    position: relative;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.9;
    transform: scale(0.6);
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out);
    width: 12px;
  }
  .tick { stroke: var(--on-accent); }
  .default .tick { stroke: var(--paper); }
  .selected .tick { opacity: 1; transform: scale(1); }
  .plus { opacity: 1; stroke: #fff; transform: none; filter: drop-shadow(0 0 1px rgba(0, 0, 0, 0.5)); }
  .current { align-items: baseline; color: var(--ink-soft); display: inline-flex; font-size: 12.5px; font-weight: 500; gap: 8px; }
  .hex { color: var(--ink-mute); font-family: var(--mono); font-size: 11px; font-weight: 400; }
  .custom-panel { padding-top: 12px; }
  .custom-row { align-items: center; display: flex; gap: 8px; }
  .color-well {
    --swatch: var(--accent);
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    cursor: pointer;
    flex: none;
    height: 30px;
    padding: 3px;
    width: 30px;
  }
  .color-well span { background: var(--swatch); border-radius: 5px; display: block; height: 100%; width: 100%; }
  .color-well:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .native-color-input { height: 0; opacity: 0; pointer-events: none; position: absolute; width: 0; }
  .hex-field {
    align-items: center;
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--ink-faint);
    display: flex;
    font-family: var(--mono);
    font-size: 12px;
    height: 30px;
    padding: 0 9px;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out);
    width: 112px;
  }
  .hex-field:focus-within { border-color: var(--line-strong); }
  .hex-field.invalid { border-color: var(--danger); }
  .hex-field input {
    background: transparent;
    border: 0;
    color: var(--ink);
    font-family: var(--mono);
    font-size: 12px;
    min-width: 0;
    outline: 0;
    padding: 0 0 0 2px;
    text-transform: uppercase;
    width: 100%;
  }
  .color-error { color: var(--danger); font-size: 11.5px; margin-top: 6px; }
  @media (prefers-reduced-motion: reduce) {
    .accent-swatch, .accent-swatch::before, .accent-swatch .fill, .tick, .plus, .hex-field { transition-duration: 1ms; }
  }
</style>
