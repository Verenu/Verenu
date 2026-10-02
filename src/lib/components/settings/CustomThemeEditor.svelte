<script lang="ts">
  import { normalizeAccentColor } from '../../accentTheme';
  import {
    CUSTOM_THEME_FIELDS,
    CUSTOM_THEME_PRESETS,
    type CustomTheme,
    type CustomThemeField,
  } from '../../customTheme';

  let {
    value,
    onchange,
    onreset,
  }: {
    value: CustomTheme;
    onchange: (value: CustomTheme) => void | Promise<void>;
    onreset: () => void | Promise<void>;
  } = $props();

  let drafts = $state<Record<CustomThemeField, string>>({
    background: '', foreground: '', sidebar: '', surface: '',
  });
  let invalid = $state<Partial<Record<CustomThemeField, boolean>>>({});

  $effect(() => {
    for (const field of CUSTOM_THEME_FIELDS) drafts[field.id] = value[field.id] ?? '';
    invalid = {};
  });

  function colorOf(field: CustomThemeField): string | null {
    return normalizeAccentColor(value[field]);
  }

  function commit(field: CustomThemeField, raw: string | null) {
    const normalized = raw === null ? null : normalizeAccentColor(raw);
    if (raw !== null && !normalized) {
      invalid[field] = true;
      return;
    }
    invalid[field] = false;
    if (normalized === (colorOf(field) ?? null)) return;
    void onchange({ ...value, [field]: normalized });
  }

  function handleInput(field: CustomThemeField, event: Event) {
    const text = (event.currentTarget as HTMLInputElement).value.replace(/^#+/, '').slice(0, 6);
    drafts[field] = text ? `#${text}` : '';
    invalid[field] = false;
    // Apply as soon as a complete code is typed so the UI previews live.
    if (/^[0-9a-f]{6}$/i.test(text)) commit(field, `#${text}`);
  }

  function handleBlur(field: CustomThemeField) {
    const optional = CUSTOM_THEME_FIELDS.find((f) => f.id === field)?.optional;
    if (!drafts[field] && optional) return commit(field, null);
    if (drafts[field] !== (value[field] ?? '')) commit(field, drafts[field]);
  }

  function matches(preset: CustomTheme): boolean {
    return CUSTOM_THEME_FIELDS.every(({ id }) =>
      (normalizeAccentColor(preset[id]) ?? null) === colorOf(id),
    );
  }
</script>

<div class="custom-theme">
  <div class="presets" role="group" aria-label="Color presets">
    {#each CUSTOM_THEME_PRESETS as preset}
      <button
        class="preset"
        class:selected={matches(preset.theme)}
        aria-pressed={matches(preset.theme)}
        onclick={() => void onchange(preset.theme)}
      >
        <span class="preset-swatch" style:background={preset.theme.background} aria-hidden="true">
          <span style:background={preset.theme.foreground}></span>
        </span>
        {preset.label}
      </button>
    {/each}
  </div>

  <div class="fields">
    {#each CUSTOM_THEME_FIELDS as field}
      <div class="field">
        <label class="field-label" for={`custom-theme-${field.id}`}>
          {field.label}{#if field.optional}<span class="auto">· optional</span>{/if}
        </label>
        <div class="field-row">
          <label class="well" style:background={colorOf(field.id) ?? undefined} class:empty={!colorOf(field.id)}>
            <input
              type="color"
              value={colorOf(field.id) ?? colorOf('background') ?? '#000000'}
              aria-label={`${field.label} color picker`}
              onchange={(event) => commit(field.id, (event.currentTarget as HTMLInputElement).value)}
            />
          </label>
          <div class="hex-field" class:invalid={invalid[field.id]}>
            {#if drafts[field.id]}<span aria-hidden="true">#</span>{/if}
            <input
              id={`custom-theme-${field.id}`}
              value={drafts[field.id].replace(/^#/, '')}
              placeholder={field.optional ? 'auto' : ''}
              aria-label={`${field.label} hex value`}
              aria-invalid={!!invalid[field.id]}
              maxlength="7"
              spellcheck="false"
              oninput={(event) => handleInput(field.id, event)}
              onblur={() => handleBlur(field.id)}
              onkeydown={(event) => event.key === 'Enter' && (event.currentTarget as HTMLInputElement).blur()}
            />
          </div>
        </div>
      </div>
    {/each}
  </div>

  {#if Object.values(invalid).some(Boolean)}
    <div class="error" role="alert">Enter a six-digit hex color, like #1E1E2E.</div>
  {/if}

  <button class="btn-ghost btn-compact reset" onclick={() => void onreset()}>Reset colors</button>
</div>

<style>
  .custom-theme { display: flex; flex-direction: column; gap: 14px; padding: 4px 0 18px; }
  .presets { display: flex; flex-wrap: wrap; gap: 8px; }
  .preset {
    align-items: center;
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 999px;
    color: var(--ink-soft);
    cursor: pointer;
    display: inline-flex;
    font-family: var(--sans);
    font-size: 11.5px;
    gap: 7px;
    padding: 4px 11px 4px 5px;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .preset:hover { border-color: var(--line-strong); color: var(--ink); }
  .preset.selected { border-color: var(--accent); color: var(--ink); }
  .preset:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .preset-swatch {
    align-items: center;
    border: 1px solid color-mix(in srgb, var(--ink) 16%, transparent);
    border-radius: 50%;
    display: flex;
    height: 18px;
    justify-content: center;
    width: 18px;
  }
  .preset-swatch > span { border-radius: 50%; height: 6px; width: 6px; }
  .fields { display: grid; gap: 12px 18px; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .field-label { color: var(--ink-mute); font-size: 11px; font-weight: 500; }
  .auto { color: var(--ink-faint); font-weight: 400; margin-left: 2px; }
  .field-row { align-items: center; display: flex; gap: 8px; }
  .well {
    border: 1px solid var(--line-strong);
    border-radius: 6px;
    cursor: pointer;
    flex: none;
    height: 28px;
    position: relative;
    width: 28px;
  }
  .well.empty {
    background: repeating-conic-gradient(var(--paper-3) 0% 25%, var(--paper) 0% 50%) 50% / 8px 8px;
  }
  .well:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
  .well input { cursor: pointer; height: 100%; inset: 0; opacity: 0; position: absolute; width: 100%; }
  .hex-field {
    align-items: center;
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--ink-faint);
    display: flex;
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), box-shadow var(--ui-duration-fast) var(--ui-ease-out);
  }
  .hex-field:focus-within { border-color: var(--accent); box-shadow: var(--ui-focus-ring); }
  .hex-field.invalid { border-color: var(--danger); }
  .hex-field input {
    background: transparent;
    border: 0;
    color: var(--ink);
    font-family: var(--mono);
    font-size: 11px;
    outline: 0;
    padding: 0;
    text-transform: uppercase;
    width: 64px;
  }
  .hex-field input::placeholder { color: var(--ink-faint); text-transform: none; }
  .error { color: var(--danger); font-size: 10.5px; }
  .reset { align-self: flex-start; }

  @media (prefers-reduced-motion: reduce) {
    .preset, .hex-field { transition-duration: 1ms; }
  }
</style>
