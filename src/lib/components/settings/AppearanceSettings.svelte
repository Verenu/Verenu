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
  import ThemePreview from './ThemePreview.svelte';

  // On Omarchy, System already follows the active Omarchy theme. The separate
  // Omarchy choice is kept only for installs that saved it earlier.
  const modes: { id: AppearanceMode; label: string }[] = $derived([
    { id: 'system', label: 'System' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' },
    ...(isLinux && !isAndroid && appStore.appearanceMode === 'omarchy'
      ? [{ id: 'omarchy' as const, label: 'Omarchy' }]
      : []),
  ]);

  const description = isMac
    ? 'Follow macOS or force a specific theme'
    : isAndroid
      ? 'Follow Android or force a specific theme'
      : isLinux
        ? 'Follow your desktop or force a specific theme'
        : 'Follow Windows or force a specific theme';

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

  // Roving tabindex: the checked radio, or the first one while a custom theme is active.
  const tabbableMode = $derived(modes.some((m) => m.id === appStore.appearanceMode) ? appStore.appearanceMode : modes[0].id);
</script>

<div class="setting-row" data-setting-target="general-appearance">
  <div><div class="label">Appearance</div><div class="desc">{description}</div></div>
</div>
<div class="mode-cards" style:--mode-count={modes.length} role="radiogroup" aria-label="Appearance" bind:this={modeGroup}>
  {#each modes as mode, index}
    {@const checked = appStore.appearanceMode === mode.id}
    <button
      class="appearance-option card"
      class:active={checked}
      role="radio"
      aria-checked={checked}
      aria-disabled={busy}
      tabindex={tabbableMode === mode.id ? 0 : -1}
      onclick={() => chooseMode(mode.id)}
      onkeydown={(event) => handleModeKeydown(event, index)}
    >
      {#if mode.id === 'system'}
        <ThemePreview colors={STOCK_PREVIEWS.light} split={STOCK_PREVIEWS.dark} />
      {:else if mode.id === 'light'}
        <ThemePreview colors={STOCK_PREVIEWS.light} />
      {:else if mode.id === 'dark'}
        <ThemePreview colors={STOCK_PREVIEWS.dark} />
      {:else}
        <ThemePreview colors={omarchyColors()} />
      {/if}
      <span class="card-label">
        {mode.label}
        {#if checked}<svg class="check" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>{/if}
      </span>
    </button>
  {/each}
</div>

{#if !isAndroid}
  <div class="setting-row themes-head" data-setting-target="general-themes">
    <div><div class="label">Themes</div><div class="desc">Pick a palette, or create and save your own. Saved themes stay on this device.</div></div>
  </div>
  <div class="gallery" role="group" aria-label="Themes">
    {#each CUSTOM_THEME_PRESETS as preset}
      {@const selected = activePreset(preset.theme)}
      <div class="theme-card" class:active={selected}>
        <button class="theme-select card" aria-disabled={busy} aria-pressed={selected} onclick={() => choosePreset(preset.theme)}>
          <ThemePreview colors={previewColors(preset.theme)} />
          <span class="card-label">
            <span class="name">{preset.label}</span>
            {#if selected}<svg class="check" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>{/if}
          </span>
        </button>
        <button class="theme-edit" aria-disabled={busy} aria-label={`Customize ${preset.label}`} title="Customize" onclick={() => customizePreset(preset.label, preset.theme)}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.7 2.5 2.5M2.5 13.5l.6-2.7 7.7-7.7 2.1 2.1-7.7 7.7-2.7.6Z"/></svg>
        </button>
      </div>
    {/each}
    {#each appStore.savedThemes as theme (theme.id)}
      {@const selected = activeSaved?.id === theme.id}
      <div class="theme-card" class:active={selected} class:editing={themeEditor.editingId === theme.id}>
        <button class="theme-select card" aria-disabled={busy} aria-pressed={selected} onclick={() => chooseSaved(theme)}>
          <ThemePreview colors={previewColors(theme.palette)} accent={theme.accent} />
          <span class="card-label">
            <span class="name">{theme.name}</span>
            {#if selected}<svg class="check" viewBox="0 0 12 12" aria-hidden="true"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>{/if}
          </span>
        </button>
        <button class="theme-edit" aria-disabled={busy} aria-label={`Edit ${theme.name}`} title="Edit" onclick={() => !busy && guardThemeEditor(() => openThemeEditor({ theme }))}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.7 2.5 2.5M2.5 13.5l.6-2.7 7.7-7.7 2.1 2.1-7.7 7.7-2.7.6Z"/></svg>
        </button>
      </div>
    {/each}
    <button class="create-card" aria-disabled={busy} onclick={createTheme}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"/></svg>
      Create theme
    </button>
  </div>
{/if}
{#if error}
  <p class="appearance-error" role="alert">{error}</p>
{/if}

<style>
  .themes-head { margin-top: 4px; }
  .mode-cards,
  .gallery {
    display: grid;
    gap: 12px;
    padding: 2px 0 18px;
  }
  /* Schemes always span the full width in equal columns. */
  .mode-cards { grid-template-columns: repeat(var(--mode-count, 3), minmax(0, 1fr)); }
  /* About 3 themes wide on desktop, 2 on medium, 1 on narrow windows. */
  .gallery { grid-template-columns: repeat(auto-fill, minmax(min(100%, 200px), 1fr)); }
  .card {
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 10px;
    color: var(--ink-soft);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    font-family: var(--sans);
    gap: 7px;
    padding: 8px 8px 10px;
    text-align: left;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), box-shadow var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    width: 100%;
  }
  .card[aria-disabled='true'], .theme-edit[aria-disabled='true'], .create-card[aria-disabled='true'] { cursor: default; opacity: 0.6; }
  .card:hover { border-color: var(--line-strong); color: var(--ink); }
  .card:focus-visible,
  .theme-edit:focus-visible,
  .create-card:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .card.active,
  .theme-card.active .card { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); color: var(--ink); }
  .theme-card.editing .card { border-style: dashed; border-color: var(--accent); }
  .card-label {
    align-items: center;
    display: flex;
    font-size: 12.5px;
    font-weight: 500;
    gap: 6px;
    justify-content: space-between;
    min-width: 0;
    padding: 0 3px;
  }
  .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .check { fill: none; flex: none; height: 12px; stroke: var(--accent); stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.8; width: 12px; }
  .theme-card { position: relative; }
  .theme-edit {
    align-items: center;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--ink-mute);
    cursor: pointer;
    display: flex;
    height: 24px;
    justify-content: center;
    opacity: 0;
    position: absolute;
    right: 14px;
    top: 14px;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    width: 24px;
  }
  .theme-card:hover .theme-edit,
  .theme-edit:focus-visible { opacity: 1; }
  .theme-edit:hover { color: var(--ink); }
  /* Touch screens have no hover, so keep the edit control visible there. */
  @media (hover: none) { .theme-edit { opacity: 1; } }
  .theme-edit svg,
  .create-card svg { fill: none; height: 12px; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; width: 12px; }
  .create-card {
    align-items: center;
    background: transparent;
    border: 1px dashed var(--line-strong);
    border-radius: 10px;
    color: var(--ink-mute);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    gap: 6px;
    justify-content: center;
    min-height: 128px;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .create-card svg { height: 16px; width: 16px; }
  .create-card:hover { border-color: var(--ink-mute); color: var(--ink); }
  .appearance-error { color: var(--danger); font-size: 12px; line-height: 1.45; margin: -6px 0 12px; }
  @media (prefers-reduced-motion: reduce) {
    .card, .theme-edit, .create-card { transition-duration: 1ms; }
  }
</style>
