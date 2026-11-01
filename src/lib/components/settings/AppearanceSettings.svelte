<script lang="ts">
  import { isAndroid, isLinux, isMac } from '../../platform';
  import { appStore } from '../../stores';
  import type { AppearanceMode } from '../../settings';
  import {
    CUSTOM_THEME_PRESETS,
    STOCK_PREVIEWS,
    basicThemePalette,
    findActiveSavedTheme,
    previewColors,
    sameThemeColors,
    type CustomTheme,
    type SavedTheme,
  } from '../../customTheme';
  import { applyPalette, setAppearanceMode, withAppearanceLock } from '../../appearanceActions';
  import { guardThemeEditor, openThemeEditor, themeEditor } from '../../themeEditor.svelte';

  const modes: { id: AppearanceMode; label: string }[] = $derived([
    { id: 'system', label: 'System' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' },
    ...(isLinux && !isAndroid && appStore.appearanceMode === 'omarchy'
      ? [{ id: 'omarchy' as const, label: 'Omarchy' }]
      : []),
  ]);

  const systemDescription = isMac
    ? 'Follow macOS'
    : isAndroid
      ? 'Follow Android'
      : isLinux
        ? 'Follow your desktop'
        : 'Follow Windows';

  const modeDescriptions: Record<string, string> = {
    light: 'Always use the bright look',
    dark: 'Always use the dim look',
    omarchy: 'Match your Omarchy theme',
  };

  let error = $state('');
  const busy = $derived(appStore.appearanceSaving || themeEditor.saving);
  let modeGroup: HTMLElement | null = $state(null);

  const activeSaved = $derived(
    appStore.appearanceMode === 'custom'
      ? findActiveSavedTheme(appStore.savedThemes, appStore.customTheme, appStore.activeThemeId)
      : null,
  );

  function activePreset(theme: CustomTheme): boolean {
    return appStore.appearanceMode === 'custom' && !activeSaved && sameThemeColors(theme, appStore.customTheme);
  }

  function omarchyColors() {
    const colors = appStore.omarchyTheme?.colors;
    return colors
      ? previewColors({
          background: colors.background,
          foreground: colors.foreground,
          sidebar: colors.dark_background ?? null,
          surface: colors.lighter_background ?? null,
        })
      : STOCK_PREVIEWS.dark;
  }

  async function chooseMode(mode: AppearanceMode) {
    if (busy || (mode === appStore.appearanceMode && !themeEditor.open)) return;
    guardThemeEditor(async () => {
      error = '';
      await withAppearanceLock(async () => {
        try {
          await setAppearanceMode(mode);
        } catch {
          error = 'Could not save the appearance. Your previous choice was restored.';
        }
      });
    });
  }

  async function applyTheme(palette: CustomTheme, options: Parameters<typeof applyPalette>[1]) {
    if (busy) return;
    guardThemeEditor(async () => {
      error = '';
      if ((await withAppearanceLock(() => applyPalette(palette, options))) === false) {
        error = 'Could not save the theme. Your previous look was restored.';
      }
    });
  }

  function chooseSaved(theme: SavedTheme) {
    return applyTheme(theme.palette, { accent: theme.accent, themeId: theme.id });
  }

  function choosePreset(theme: CustomTheme) {
    return applyTheme(theme, { themeId: null });
  }

  function customizePreset(label: string, theme: CustomTheme) {
    if (busy) return;
    guardThemeEditor(() => openThemeEditor({ seed: theme, name: `${label} copy` }));
  }

  function createTheme() {
    if (busy) return;
    guardThemeEditor(() => {
      const dark = document.documentElement.dataset.theme === 'dark';
      const seed = appStore.appearanceMode === 'custom' && appStore.customTheme
        ? appStore.customTheme
        : basicThemePalette(dark);
      openThemeEditor({ seed, name: 'My theme', accent: appStore.appearanceMode === 'custom' ? appStore.accentColor : null });
    });
  }

  function handleModeKeydown(event: KeyboardEvent, index: number) {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = (index + step + modes.length) % modes.length;
    modeGroup?.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus();
    void chooseMode(modes[next].id);
  }

  const tabbableMode = $derived(modes.some((m) => m.id === appStore.appearanceMode) ? appStore.appearanceMode : modes[0].id);
</script>

{#snippet swatch(c: { background: string; foreground: string; sidebar: string; surface: string }, accent: string | null = null)}
  <span class="swatch" style:background={c.background} aria-hidden="true">
    <i class="swatch-side" style:background={c.sidebar}></i>
    <i class="swatch-card" style:background={c.surface}></i>
    <i class="swatch-dot" style:background={accent ?? c.foreground}></i>
  </span>
{/snippet}

<div class="option-list" role="radiogroup" aria-label="Appearance" data-setting-target="general-appearance" bind:this={modeGroup}>
  {#each modes as mode, index}
    {@const checked = appStore.appearanceMode === mode.id}
    <button
      class="option appearance-option"
      class:active={checked}
      role="radio"
      aria-label={mode.label}
      aria-describedby={`appearance-${mode.id}-desc`}
      aria-checked={checked}
      aria-disabled={busy}
      tabindex={tabbableMode === mode.id ? 0 : -1}
      onclick={() => chooseMode(mode.id)}
      onkeydown={(event) => handleModeKeydown(event, index)}
    >
      <span class="swatch-wrap" aria-hidden="true">
        {#if mode.id === 'system'}
          <span class="swatch-split">
            {@render swatch(STOCK_PREVIEWS.light)}
            <span class="swatch-half">{@render swatch(STOCK_PREVIEWS.dark)}</span>
          </span>
        {:else if mode.id === 'light'}
          {@render swatch(STOCK_PREVIEWS.light)}
        {:else if mode.id === 'dark'}
          {@render swatch(STOCK_PREVIEWS.dark)}
        {:else}
          {@render swatch(omarchyColors())}
        {/if}
      </span>
      <span class="option-text">
        <span class="option-name">{mode.label}</span>
        <span class="option-desc" id={`appearance-${mode.id}-desc`}>{mode.id === 'system' ? systemDescription : modeDescriptions[mode.id]}</span>
      </span>
      <svg class="check" class:shown={checked} viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
    </button>
  {/each}
</div>

{#if !isAndroid}
  <div class="list-head" data-setting-target="general-themes">
    <div>
      <div class="label">Themes</div>
      <div class="desc">Pick a palette, or create and save your own. Saved themes stay on this device.</div>
    </div>
    <button class="btn-ghost btn-compact" aria-disabled={busy} onclick={createTheme}>Create theme</button>
  </div>
  <div class="option-list" role="group" aria-label="Themes">
    {#each appStore.savedThemes as theme (theme.id)}
      {@const selected = activeSaved?.id === theme.id}
      <div class="theme-row" class:active={selected} class:editing={themeEditor.editingId === theme.id}>
        <button class="option theme-select" aria-disabled={busy} aria-pressed={selected} onclick={() => chooseSaved(theme)}>
          <span class="swatch-wrap">{@render swatch(previewColors(theme.palette), theme.accent)}</span>
          <span class="option-text"><span class="option-name">{theme.name}</span></span>
          <svg class="check" class:shown={selected} viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
        </button>
        <button class="theme-edit" aria-disabled={busy} aria-label={`Edit ${theme.name}`} title="Edit" onclick={() => !busy && guardThemeEditor(() => openThemeEditor({ theme }))}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.7 2.5 2.5M2.5 13.5l.6-2.7 7.7-7.7 2.1 2.1-7.7 7.7-2.7.6Z"/></svg>
        </button>
      </div>
    {/each}
    {#each CUSTOM_THEME_PRESETS as preset}
      {@const selected = activePreset(preset.theme)}
      <div class="theme-row" class:active={selected}>
        <button class="option theme-select" aria-disabled={busy} aria-pressed={selected} onclick={() => choosePreset(preset.theme)}>
          <span class="swatch-wrap">{@render swatch(previewColors(preset.theme))}</span>
          <span class="option-text"><span class="option-name">{preset.label}</span></span>
          <svg class="check" class:shown={selected} viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
        </button>
        <button class="theme-edit" aria-disabled={busy} aria-label={`Customize ${preset.label}`} title="Customize" onclick={() => customizePreset(preset.label, preset.theme)}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.7 2.5 2.5M2.5 13.5l.6-2.7 7.7-7.7 2.1 2.1-7.7 7.7-2.7.6Z"/></svg>
        </button>
      </div>
    {/each}
  </div>
{/if}
{#if error}
  <p class="appearance-error" role="alert">{error}</p>
{/if}

<style>
  .option-list {
    border: 1px solid var(--line);
    border-radius: 10px;
    margin: 4px 0 16px;
    overflow: hidden;
  }
  .option-list > :global(* + *) { border-top: 1px solid var(--line); }
  .option {
    align-items: center;
    background: transparent;
    border: 0;
    color: var(--ink-soft);
    cursor: pointer;
    display: flex;
    font-family: var(--sans);
    gap: 14px;
    min-height: 52px;
    padding: 8px 14px;
    text-align: left;
    transition: background var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    width: 100%;
  }
  .option[aria-disabled='true'] { cursor: default; opacity: 0.6; }
  .option:hover { background: var(--control-hover); color: var(--ink); }
  .option:focus-visible,
  .theme-edit:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .option.active, .theme-row.active .option { background: color-mix(in srgb, var(--accent) 6%, transparent); color: var(--ink); }
  .option-text { display: flex; flex: 1; flex-direction: column; gap: 1px; min-width: 0; }
  .option-name { font-size: 13px; font-weight: 500; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .option-desc { color: var(--ink-mute); font-size: 12px; }
  .check { fill: none; flex: none; height: 12px; opacity: 0; stroke: var(--accent); stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.8; width: 12px; }
  .check.shown { opacity: 1; }
  .swatch-wrap { display: block; flex: none; height: 28px; position: relative; width: 46px; }
  .swatch, .swatch-split { border-radius: 6px; display: block; height: 100%; overflow: hidden; position: relative; width: 100%; box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ink) 14%, transparent); }
  .swatch-split { box-shadow: none; }
  .swatch-half { clip-path: inset(0 0 0 50%); inset: 0; position: absolute; }
  .swatch-side { bottom: 0; left: 0; position: absolute; top: 0; width: 28%; }
  .swatch-card { border-radius: 2px; height: 9px; left: 38%; position: absolute; right: 8%; top: 7px; }
  .swatch-dot { border-radius: 50%; bottom: 5px; height: 5px; left: 38%; position: absolute; width: 5px; }
  .list-head { align-items: center; display: flex; gap: 16px; justify-content: space-between; margin-top: 20px; }
  .list-head .btn-ghost { flex: none; }
  .list-head .btn-ghost[aria-disabled='true'] { cursor: default; opacity: 0.6; }
  .theme-row { align-items: stretch; display: flex; position: relative; }
  .theme-row .theme-select { flex: 1; min-height: 44px; min-width: 0; }
  .theme-row.editing { background: var(--control-hover); }
  .theme-edit {
    align-items: center;
    background: transparent;
    border: 0;
    color: var(--ink-mute);
    cursor: pointer;
    display: flex;
    flex: none;
    justify-content: center;
    transition: color var(--ui-duration-fast) var(--ui-ease-out), background var(--ui-duration-fast) var(--ui-ease-out);
    width: 44px;
  }
  .theme-edit:hover { background: var(--control-hover); color: var(--ink); }
  .theme-edit[aria-disabled='true'] { cursor: default; opacity: 0.6; }
  .theme-edit svg { fill: none; height: 13px; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; width: 13px; }
  .appearance-error { color: var(--danger); font-size: 12px; line-height: 1.45; margin: -6px 0 12px; }
  @media (prefers-reduced-motion: reduce) {
    .option, .theme-edit { transition-duration: 1ms; }
  }
</style>
