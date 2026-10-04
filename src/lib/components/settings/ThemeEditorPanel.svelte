<script lang="ts">
  import { onMount } from 'svelte';
  import { normalizeAccentColor } from '../../accentTheme';
  import {
    CUSTOM_THEME_FIELDS,
    THEME_NAME_MAX,
    basicThemePalette,
    deriveForeground,
    hasPaletteOverrides,
    isDarkColor,
    previewColors,
    type CustomThemeField,
  } from '../../customTheme';
  import {
    confirmDiscardThemeEditor,
    deleteThemeFromEditor,
    keepEditingTheme,
    requestCloseThemeEditor,
    saveThemeEditor,
    themeEditor,
    themeNameError,
    updateThemeDraft,
  } from '../../themeEditor.svelte';
  import ThemePreview from './ThemePreview.svelte';

  type FieldId = CustomThemeField | 'accent';

  const allFields: { id: FieldId; label: string; optional: boolean }[] = [
    ...CUSTOM_THEME_FIELDS,
    { id: 'accent', label: 'Accent', optional: true },
  ];
  const fields = allFields;
  const BASIC: FieldId[] = ['background', 'accent'];
  const basicFields = allFields.filter((f) => BASIC.includes(f.id));
  const advancedFields = allFields.filter((f) => !BASIC.includes(f.id));

  // Palettes with values the basic editor would not have chosen open Advanced,
  // so those values stay discoverable.
  let advancedOpen = $state(hasPaletteOverrides(themeEditor.draft.palette));
  const dark = $derived(isDarkColor(themeEditor.draft.palette.background));

  function currentValue(field: FieldId): string | null {
    return field === 'accent' ? themeEditor.draft.accent : (themeEditor.draft.palette[field] ?? null);
  }

  // Text being typed, kept apart from the committed color so a half-typed hex
  // never reaches the live preview.
  let texts = $state<Record<FieldId, string>>(
    Object.fromEntries(fields.map((f) => [f.id, currentValue(f.id) ?? ''])) as Record<FieldId, string>,
  );
  let invalid = $state<Partial<Record<FieldId, boolean>>>({});
  let nameInput: HTMLInputElement | null = $state(null);
  let nameTouched = $state(false);

  const nameError = $derived(nameTouched || themeEditor.error ? themeNameError() : null);
  const editing = $derived(themeEditor.editingId !== null);
  const colors = $derived(previewColors(themeEditor.draft.palette));

  onMount(() => {
    const opener = document.activeElement as HTMLElement | null;
    nameInput?.focus();
    nameInput?.select();
    return () => {
      if (opener?.isConnected) opener.focus();
    };
  });

  function colorOf(field: FieldId): string | null {
    return normalizeAccentColor(currentValue(field));
  }

  function syncTexts() {
    for (const f of allFields) texts[f.id] = currentValue(f.id) ?? '';
    invalid = {};
  }

  function commit(field: FieldId, value: string | null) {
    if (field === 'accent') {
      updateThemeDraft({ accent: value });
    } else if (field === 'background' && value && !hasPaletteOverrides(themeEditor.draft.palette)) {
      // While the palette is automatic, text follows the background and the
      // sidebar and surface stay derived.
      updateThemeDraft({ palette: { background: value, foreground: deriveForeground(value), sidebar: null, surface: null } });
      syncTexts();
      return;
    } else {
      updateThemeDraft({ palette: { ...themeEditor.draft.palette, [field]: value } });
    }
    texts[field] = value ?? '';
  }

  function chooseMode(next: boolean) {
    if (next === dark) return;
    // A fresh palette for the other mode; the chosen accent carries over.
    updateThemeDraft({ palette: basicThemePalette(next) });
    syncTexts();
    advancedOpen = false;
  }

  function handleInput(field: FieldId, event: Event) {
    const text = (event.currentTarget as HTMLInputElement).value.replace(/^#+/, '').slice(0, 6);
    texts[field] = text ? `#${text}` : '';
    invalid[field] = false;
    // Apply as soon as a complete code is typed so the app previews live.
    if (/^[0-9a-f]{6}$/i.test(text)) commit(field, `#${text}`.toUpperCase());
  }

  function handleBlur(field: FieldId, optional: boolean) {
    const text = texts[field];
    if (!text) {
      if (optional) {
        invalid[field] = false;
        if (currentValue(field)) commit(field, null);
      } else {
        texts[field] = currentValue(field) ?? '';
      }
      return;
    }
    if (!normalizeAccentColor(text)) invalid[field] = true;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    requestCloseThemeEditor();
  }

  function handleSubmit(event: SubmitEvent) {
    event.preventDefault();
    (document.activeElement as HTMLElement | null)?.blur();
    nameTouched = true;
    void saveThemeEditor();
  }

  const anyInvalid = $derived(Object.values(invalid).some(Boolean));
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
  class="panel"
  role="group"
  aria-label={editing ? 'Edit theme' : 'Create theme'}
  onsubmit={handleSubmit}
  onkeydown={handleKeydown}
>
  <div class="head">
    <h3>{editing ? 'Edit theme' : 'Create theme'}</h3>
    <button type="button" class="close" aria-label="Close theme editor" onclick={requestCloseThemeEditor}>
      <svg viewBox="0 0 12 12" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9"/></svg>
    </button>
  </div>

  <ThemePreview {colors} accent={themeEditor.draft.accent ?? colors.foreground} />

  <label class="name-field">
    <span class="field-label">Name</span>
    <input
      bind:this={nameInput}
      class="name-input"
      class:invalid={!!nameError}
      value={themeEditor.draft.name}
      maxlength={THEME_NAME_MAX}
      autocomplete="off"
      spellcheck="false"
      aria-invalid={!!nameError}
      aria-describedby={nameError ? 'theme-name-error' : undefined}
      oninput={(event) => updateThemeDraft({ name: (event.currentTarget as HTMLInputElement).value })}
      onblur={() => (nameTouched = true)}
    />
  </label>
  {#if nameError}<div id="theme-name-error" class="error">{nameError}</div>{/if}

  <div class="mode" role="radiogroup" aria-label="Appearance">
    <span class="field-label" aria-hidden="true">Appearance</span>
    <div class="mode-seg">
      <button type="button" class="mode-btn" class:on={!dark} role="radio" aria-checked={!dark} onclick={() => chooseMode(false)}>Light</button>
      <button type="button" class="mode-btn" class:on={dark} role="radio" aria-checked={dark} onclick={() => chooseMode(true)}>Dark</button>
    </div>
  </div>

  {#snippet fieldRow(field: { id: FieldId; label: string; optional: boolean })}
    <div class="field">
      <label class="field-label" for={`theme-editor-${field.id}`}>
        {field.label}{#if field.optional}<span class="auto">· optional</span>{/if}
      </label>
      <div class="field-row">
        <label class="well" style:background={colorOf(field.id) ?? undefined} class:empty={!colorOf(field.id)}>
          <input
            type="color"
            value={colorOf(field.id) ?? colorOf('background') ?? '#000000'}
            aria-label={`${field.label} color picker`}
            oninput={(event) => commit(field.id, (event.currentTarget as HTMLInputElement).value.toUpperCase())}
          />
        </label>
        <div class="hex-field" class:invalid={invalid[field.id]}>
          {#if texts[field.id]}<span aria-hidden="true">#</span>{/if}
          <input
            id={`theme-editor-${field.id}`}
            value={texts[field.id].replace(/^#/, '')}
            placeholder={field.optional ? 'auto' : ''}
            aria-label={`${field.label} hex value`}
            aria-invalid={!!invalid[field.id]}
            maxlength="7"
            spellcheck="false"
            oninput={(event) => handleInput(field.id, event)}
            onblur={() => handleBlur(field.id, field.optional)}
          />
        </div>
      </div>
    </div>
  {/snippet}

  <div class="fields">
    {#each basicFields as field}{@render fieldRow(field)}{/each}
  </div>

  <button type="button" class="adv-toggle" aria-expanded={advancedOpen} aria-controls="theme-advanced" onclick={() => (advancedOpen = !advancedOpen)}>
    <svg class:open={advancedOpen} viewBox="0 0 12 12" aria-hidden="true"><path d="m3.25 4.75 2.75 2.5 2.75-2.5"/></svg>
    Advanced
  </button>
  {#if advancedOpen}
    <div id="theme-advanced" class="fields">
      {#each advancedFields as field}{@render fieldRow(field)}{/each}
    </div>
  {/if}

  {#if anyInvalid}
    <div class="error" role="alert">Enter a six-digit hex color, like #1E1E2E.</div>
  {/if}
  {#if themeEditor.error}
    <div class="error" role="alert">{themeEditor.error}</div>
  {/if}

  {#if themeEditor.pendingAction}
    <div class="discard" role="alertdialog" aria-label="Discard unsaved changes">
      <p>Discard your unsaved changes?</p>
      <div class="actions">
        <button type="button" class="btn-ghost btn-compact" onclick={keepEditingTheme}>Keep editing</button>
        <button type="button" class="btn-danger btn-compact" onclick={confirmDiscardThemeEditor}>Discard</button>
      </div>
    </div>
  {:else if themeEditor.confirmingDelete}
    <div class="discard" role="alertdialog" aria-label="Delete theme">
      <p>Delete “{themeEditor.draft.name}”? The colors stay applied until you pick another theme.</p>
      <div class="actions">
        <button type="button" class="btn-ghost btn-compact" disabled={themeEditor.saving} onclick={() => (themeEditor.confirmingDelete = false)}>Keep theme</button>
        <button type="button" class="btn-danger btn-compact" disabled={themeEditor.saving} onclick={deleteThemeFromEditor}>Delete</button>
      </div>
    </div>
  {:else}
    <div class="actions">
      {#if editing}
        <button type="button" class="btn-ghost btn-compact delete" disabled={themeEditor.saving} onclick={() => (themeEditor.confirmingDelete = true)}>Delete</button>
      {/if}
      <button type="button" class="btn-ghost btn-compact" disabled={themeEditor.saving} onclick={requestCloseThemeEditor}>Cancel</button>
      <button type="submit" class="btn-primary btn-compact" disabled={themeEditor.saving || anyInvalid}>
        {themeEditor.saving ? 'Saving...' : 'Save theme'}
      </button>
    </div>
  {/if}
</form>

<style>
  .panel { display: flex; flex-direction: column; gap: 12px; padding: 14px; }
  .head { align-items: center; display: flex; justify-content: space-between; }
  h3 { color: var(--ink); font-family: var(--sans); font-size: 14px; font-weight: 600; letter-spacing: -0.01em; margin: 0; }
  .close {
    align-items: center;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--ink-mute);
    cursor: pointer;
    display: flex;
    height: 24px;
    justify-content: center;
    width: 24px;
  }
  .close:hover { background: var(--control-hover); color: var(--ink); }
  .close:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .close svg { fill: none; height: 12px; stroke: currentColor; stroke-linecap: round; stroke-width: 1.5; width: 12px; }
  .name-field { display: flex; flex-direction: column; gap: 5px; }
  .name-input {
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--ink);
    font-family: var(--sans);
    font-size: 12.5px;
    height: 30px;
    outline: 0;
    padding: 0 9px;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), box-shadow var(--ui-duration-fast) var(--ui-ease-out);
  }
  .name-input:focus { border-color: var(--accent); box-shadow: var(--ui-focus-ring); }
  .name-input.invalid { border-color: var(--danger); }
  .mode { align-items: center; display: flex; justify-content: space-between; }
  .mode-seg { background: var(--paper); border: 1px solid var(--line); border-radius: 7px; display: inline-flex; gap: 2px; padding: 3px; }
  .mode-btn {
    background: transparent;
    border: 0;
    border-radius: 4px;
    color: var(--ink-mute);
    cursor: pointer;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    height: 22px;
    padding: 0 12px;
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .mode-btn:hover { color: var(--ink-strong); }
  .mode-btn.on { background: var(--bg-elev); color: var(--ink); box-shadow: 0 0 0 1px var(--line); }
  .mode-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .adv-toggle {
    align-items: center;
    align-self: flex-start;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--ink-mute);
    cursor: pointer;
    display: inline-flex;
    font-family: var(--sans);
    font-size: 11.5px;
    font-weight: 500;
    gap: 4px;
    padding: 2px 4px 2px 0;
  }
  .adv-toggle:hover { color: var(--ink); }
  .adv-toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .adv-toggle svg { fill: none; height: 12px; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.25; transition: transform var(--ui-duration-fast) var(--ui-ease-out); width: 12px; }
  .adv-toggle svg.open { transform: rotate(180deg); }
  .fields { display: grid; gap: 10px 12px; grid-template-columns: 1fr 1fr; }
  .field { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
  .field-label { color: var(--ink-mute); font-size: 11px; font-weight: 500; }
  .auto { color: var(--ink-faint); font-weight: 400; margin-left: 2px; }
  .field-row { align-items: center; display: flex; gap: 7px; }
  .well {
    border: 1px solid var(--line-strong);
    border-radius: 6px;
    cursor: pointer;
    flex: none;
    height: 28px;
    position: relative;
    width: 28px;
  }
  .well.empty { background: repeating-conic-gradient(var(--paper-3) 0% 25%, var(--paper) 0% 50%) 50% / 8px 8px; }
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
    padding: 0 7px;
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
    min-width: 0;
    outline: 0;
    padding: 0;
    text-transform: uppercase;
    width: 100%;
  }
  .hex-field input::placeholder { color: var(--ink-faint); text-transform: none; }
  .error { color: var(--danger); font-size: 11px; line-height: 1.4; }
  .actions { align-items: center; display: flex; gap: 8px; justify-content: flex-end; }
  .delete { margin-right: auto; }
  .discard { background: var(--paper-2); border-radius: 8px; display: flex; flex-direction: column; gap: 8px; padding: 10px; }
  .discard p { color: var(--ink-soft); font-size: 12px; line-height: 1.45; margin: 0; }
  @media (prefers-reduced-motion: reduce) {
    .name-input, .hex-field, .mode-btn, .adv-toggle svg { transition-duration: 1ms; }
  }
</style>
