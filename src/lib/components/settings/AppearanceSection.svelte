<script lang="ts">
  import { flip } from 'svelte/animate';
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { isAndroid, isLinux, isMac } from '../../platform';
  import { appStore } from '../../stores';
  import { MOTION_MS, motionMs } from '../../motion';
  import { saveSetting, type AppearanceMode, type InterfaceFont } from '../../settings';
  import {
    CUSTOM_THEME_PRESETS,
    STOCK_PREVIEWS,
    basicThemePalette,
    findActiveSavedTheme,
    isDarkColor,
    previewColors,
    sameThemeColors,
    type CustomTheme,
    type SavedTheme,
  } from '../../customTheme';
  import { applyPalette, persistAccentColor, setAppearanceMode, withAppearanceLock } from '../../appearanceActions';
  import { guardThemeEditor, openThemeEditor, themeEditor } from '../../themeEditor.svelte';
  import { DEFAULT_INTERFACE_FONT, INTERFACE_FONT_OPTIONS, applyInterfaceFont } from '../../interfaceFont';
  import AccentSwatches from './AccentSwatches.svelte';
  import ThemePreview from './ThemePreview.svelte';

  type Filter = 'all' | 'dark' | 'light';
  type GalleryItem = {
    key: string;
    name: string;
    palette: CustomTheme;
    accent: string | null;
    saved: SavedTheme | null;
  };

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
    light: 'Always light',
    dark: 'Always dark',
    omarchy: 'Match Omarchy',
  };

  const filters: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'dark', label: 'Dark' },
    { id: 'light', label: 'Light' },
  ];

  let error = $state('');
  let filter = $state<Filter>('all');
  let fontSaving = $state(false);
  let resetting = $state(false);
  let modeGroup: HTMLElement | null = $state(null);
  let fontGroup: HTMLElement | null = $state(null);
  let filterGroup: HTMLElement | null = $state(null);
  const busy = $derived(appStore.appearanceSaving || themeEditor.saving);

  const activeSaved = $derived(
    appStore.appearanceMode === 'custom'
      ? findActiveSavedTheme(appStore.savedThemes, appStore.customTheme, appStore.activeThemeId)
      : null,
  );
  const activePresetLabel = $derived(
    appStore.appearanceMode === 'custom' && !activeSaved
      ? CUSTOM_THEME_PRESETS.find((preset) => sameThemeColors(preset.theme, appStore.customTheme))?.label ?? null
      : null,
  );
  const themeActive = $derived(appStore.appearanceMode === 'custom');
  const activeThemeName = $derived(activeSaved?.name ?? activePresetLabel ?? 'Custom colors');

  const lookName = $derived(
    themeActive ? activeThemeName : modes.find((mode) => mode.id === appStore.appearanceMode)?.label ?? 'System',
  );
  const lookDetail = $derived.by(() => {
    const font = INTERFACE_FONT_OPTIONS.find((option) => option.id === appStore.interfaceFont)?.label ?? 'Inter Tight';
    const accent = appStore.accentColor ?? 'Default accent';
    if (themeActive) return [activeSaved ? 'Your theme' : activePresetLabel ? 'Built-in theme' : 'Custom theme', accent, font];
    const base = appStore.appearanceMode === 'system' ? systemDescription : modeDescriptions[appStore.appearanceMode] ?? '';
    return [base, accent, font];
  });
  const isDefaultLook = $derived(
    appStore.appearanceMode === 'system' && appStore.accentColor === null && appStore.interfaceFont === DEFAULT_INTERFACE_FONT,
  );

  function matchesFilter(palette: CustomTheme): boolean {
    if (filter === 'all') return true;
    return isDarkColor(palette.background) === (filter === 'dark');
  }

  const galleryItems = $derived<GalleryItem[]>([
    ...appStore.savedThemes
      .filter((theme) => matchesFilter(theme.palette))
      .map((theme) => ({ key: theme.id, name: theme.name, palette: theme.palette, accent: theme.accent, saved: theme })),
    ...CUSTOM_THEME_PRESETS
      .filter((preset) => matchesFilter(preset.theme))
      .map((preset) => ({ key: `preset:${preset.label}`, name: preset.label, palette: preset.theme, accent: null, saved: null })),
  ]);
  function itemSelected(item: GalleryItem): boolean {
    if (!themeActive) return false;
    return item.saved ? activeSaved?.id === item.saved.id : !activeSaved && activePresetLabel === item.name;
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

  function chooseItem(item: GalleryItem) {
    if (busy) return;
    guardThemeEditor(async () => {
      error = '';
      const options = item.saved ? { accent: item.saved.accent, themeId: item.saved.id } : { themeId: null };
      if ((await withAppearanceLock(() => applyPalette(item.palette, options))) === false) {
        error = 'Could not save the theme. Your previous look was restored.';
      }
    });
  }

  function editItem(item: GalleryItem) {
    if (busy) return;
    const saved = item.saved;
    guardThemeEditor(() =>
      saved ? openThemeEditor({ theme: saved }) : openThemeEditor({ seed: item.palette, name: `${item.name} copy` }),
    );
  }

  function createTheme() {
    if (busy) return;
    guardThemeEditor(() => {
      const dark = document.documentElement.dataset.theme === 'dark';
      const seed = themeActive && appStore.customTheme ? appStore.customTheme : basicThemePalette(dark);
      openThemeEditor({ seed, name: 'My theme', accent: themeActive ? appStore.accentColor : null });
    });
  }

  function chooseAccent(color: string | null) {
    if (busy) return;
    guardThemeEditor(async () => {
      error = '';
      const saved = await withAppearanceLock(() => persistAccentColor(color));
      if (saved === false) error = 'Could not save the accent color. Your previous color was restored.';
    });
  }

  async function chooseFont(font: InterfaceFont, fromReset = false) {
    if (fontSaving || (resetting && !fromReset) || font === appStore.interfaceFont) return;
    const previous = appStore.interfaceFont;
    fontSaving = true;
    error = '';
    appStore.interfaceFontChangeVersion++;
    appStore.interfaceFont = font;
    applyInterfaceFont(document.documentElement, font);
    try {
      await saveSetting('interface_font', font);
      // A refresh that started before the save committed may have read the old value; the saved choice wins.
      appStore.interfaceFontChangeVersion++;
      appStore.interfaceFont = font;
    } catch (err) {
      console.error('save interface_font failed:', err);
      appStore.interfaceFont = previous;
      applyInterfaceFont(document.documentElement, previous);
      error = 'Could not save the font. Your previous font was restored.';
    } finally {
      fontSaving = false;
    }
  }

  async function resetLook() {
    if (busy || fontSaving || resetting || isDefaultLook) return;
    guardThemeEditor(async () => {
      error = '';
      resetting = true;
      try {
        if (appStore.appearanceMode !== 'system') {
          const ok = await withAppearanceLock(async () => {
            try { await setAppearanceMode('system'); return true; } catch { return false; }
          });
          if (ok === false) { error = 'Could not reset the appearance. Try again.'; return; }
        }
        if (appStore.accentColor !== null) {
          const ok = await withAppearanceLock(() => persistAccentColor(null));
          if (ok === false) { error = 'Could not reset the accent color. Try again.'; return; }
        }
        if (appStore.interfaceFont !== DEFAULT_INTERFACE_FONT) await chooseFont(DEFAULT_INTERFACE_FONT, true);
      } finally {
        resetting = false;
      }
    });
  }

  /** Arrow keys move through a radio group and select, matching native radio behavior. */
  function radioKeydown<T>(event: KeyboardEvent, group: HTMLElement | null, items: T[], index: number, select: (item: T) => void) {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = (index + step + items.length) % items.length;
    group?.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus();
    select(items[next]);
  }

  const tabbableMode = $derived(modes.some((m) => m.id === appStore.appearanceMode) ? appStore.appearanceMode : modes[0].id);
  const filterIndex = $derived(filters.findIndex((f) => f.id === filter));
  const fontIndex = $derived(Math.max(0, INTERFACE_FONT_OPTIONS.findIndex((f) => f.id === appStore.interfaceFont)));
  const flipParams = $derived({ duration: motionMs(MOTION_MS.panel), easing: cubicOut });
</script>

{#snippet check()}
  <span class="card-check" aria-hidden="true">
    <svg viewBox="0 0 12 12"><path d="m2.5 6.2 2.1 2.1 4.9-4.8"/></svg>
  </span>
{/snippet}


<h2 class="settings-h">Appearance</h2>
<p class="settings-lede">Choose how Verenu looks. Changes apply right away.</p>

<section class="appearance-hero" aria-label="Current look">
  <div class="live-window" aria-hidden="true">
    <div class="lw-side">
      <span class="lw-brand"><i></i><b></b></span>
      <span class="lw-nav active"><i></i><b></b></span>
      <span class="lw-nav"><i></i><b></b></span>
      <span class="lw-nav"><i></i><b class="short"></b></span>
    </div>
    <div class="lw-main">
      <span class="lw-title">Good morning</span>
      <span class="lw-sub">Your dictation, cleaned up.</span>
      <div class="lw-card">
        <div class="lw-row"><b></b><span class="lw-toggle on"><i></i></span></div>
        <div class="lw-row"><b class="short"></b><span class="lw-toggle"><i></i></span></div>
      </div>
      <div class="lw-actions">
        <span class="lw-btn primary">Save</span>
        <span class="lw-btn">Cancel</span>
      </div>
      <span class="lw-pill"><i></i><i></i><i></i><i></i><i></i></span>
    </div>
  </div>
  <div class="hero-copy">
    <span class="hero-eyebrow">Current look</span>
    {#key lookName}
      <span class="hero-name" in:fade={{ duration: motionMs(MOTION_MS.base) }}>{lookName}</span>
    {/key}
    <p class="hero-meta">{lookDetail.join(' · ')}</p>
    <div class="hero-actions">
      {#if !isAndroid}
        <button class="btn-ghost btn-compact" aria-disabled={busy} onclick={createTheme}>
          {themeActive && !activeSaved ? 'Save as theme' : 'New theme'}
        </button>
      {/if}
      <button class="btn-ghost btn-compact" disabled={isDefaultLook} aria-disabled={busy || fontSaving || resetting} onclick={resetLook}>Reset</button>
    </div>
  </div>
</section>

<h3 class="settings-subhead">Mode</h3>
<div
  class="mode-grid"
  style:--modes={modes.length}
  role="radiogroup"
  aria-label="Appearance"
  data-setting-target="appearance-mode"
  bind:this={modeGroup}
>
  {#each modes as mode, index (mode.id)}
    {@const checked = appStore.appearanceMode === mode.id}
    <button
      class="look-card mode-card appearance-option"
      class:selected={checked}
      role="radio"
      aria-label={mode.label}
      aria-describedby={`appearance-${mode.id}-desc`}
      aria-checked={checked}
      aria-disabled={busy}
      tabindex={tabbableMode === mode.id ? 0 : -1}
      onclick={() => chooseMode(mode.id)}
      onkeydown={(event) => radioKeydown(event, modeGroup, modes, index, (m) => void chooseMode(m.id))}
    >
      <span class="card-preview">
        {#if mode.id === 'system'}
          <ThemePreview colors={STOCK_PREVIEWS.light} split={STOCK_PREVIEWS.dark} accent={appStore.accentColor} compact />
        {:else if mode.id === 'light'}
          <ThemePreview colors={STOCK_PREVIEWS.light} accent={appStore.accentColor} compact />
        {:else if mode.id === 'dark'}
          <ThemePreview colors={STOCK_PREVIEWS.dark} accent={appStore.accentColor} compact />
        {:else}
          <ThemePreview colors={omarchyColors()} accent={appStore.accentColor} compact />
        {/if}
      </span>
      <span class="card-meta">
        <span class="card-name">{mode.label}</span>
        <span class="card-desc" id={`appearance-${mode.id}-desc`}>{mode.id === 'system' ? systemDescription : modeDescriptions[mode.id]}</span>
      </span>
      {@render check()}
    </button>
  {/each}
</div>
{#if themeActive}
  <p class="mode-note" in:fade={{ duration: motionMs(MOTION_MS.base) }}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6.25"/><path d="M8 7.25v3.5M8 5.2v.01"/></svg>
    <span><strong>{activeThemeName}</strong> is in use. Choose a mode to go back to the standard colors.</span>
  </p>
{/if}

<h3 class="settings-subhead">Accent color</h3>
<div class="ap-card" data-setting-target="appearance-accent-card">
  <p class="ap-card-desc">Used for buttons, highlights, toggles, and focus rings.{#if activeSaved?.accent} Choosing a color here replaces this theme's accent until you pick the theme again.{/if}</p>
  <AccentSwatches value={appStore.accentColor} disabled={busy} onchange={chooseAccent} />
</div>

{#if !isAndroid}
  <div class="themes-head" data-setting-target="appearance-themes">
    <div>
      <h3 class="settings-subhead themes-title">Themes</h3>
      <p class="themes-desc">Full palettes that replace the light and dark colors. Your themes stay on this device.</p>
    </div>
    <div class="seg" role="radiogroup" aria-label="Filter themes" style:--seg-count={filters.length} style:--seg-index={filterIndex} bind:this={filterGroup}>
      <span class="seg-thumb" aria-hidden="true"></span>
      {#each filters as option, index (option.id)}
        <button
          class="seg-btn"
          class:on={filter === option.id}
          role="radio"
          aria-checked={filter === option.id}
          aria-label={`${option.label} themes`}
          tabindex={filter === option.id ? 0 : -1}
          onclick={() => (filter = option.id)}
          onkeydown={(event) => radioKeydown(event, filterGroup, filters, index, (f) => (filter = f.id))}
        >{option.label}</button>
      {/each}
    </div>
  </div>

  <div class="theme-gallery" role="group" aria-label="Themes">
    <button class="look-card create-card" aria-label="Create theme" aria-disabled={busy} onclick={createTheme}>
      <span class="create-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16"><path d="M8 3.5v9M3.5 8h9"/></svg>
      </span>
      <span class="create-copy">
        <span class="card-name">Create theme</span>
        <span class="card-desc">{themeActive ? 'Start from the current colors' : 'Pick your own colors'}</span>
      </span>
    </button>
    {#each galleryItems as item (item.key)}
      {@const selected = itemSelected(item)}
      {@const editing = !!item.saved && themeEditor.editingId === item.saved.id}
      <div
        class="look-card theme-card"
        class:selected
        class:editing
        animate:flip={flipParams}
        in:scale={{ start: 0.94, duration: motionMs(MOTION_MS.base), easing: cubicOut }}
        out:fade={{ duration: motionMs(MOTION_MS.fast) }}
      >
        <button class="card-hit" aria-label={item.name} aria-pressed={selected} aria-disabled={busy} onclick={() => chooseItem(item)}>
          <span class="card-preview"><ThemePreview colors={previewColors(item.palette)} accent={item.accent} compact /></span>
          <span class="card-meta">
            <span class="card-name">{item.name}</span>
            <span class="card-tone">{item.saved ? 'Your theme' : isDarkColor(item.palette.background) ? 'Dark' : 'Light'}</span>
          </span>
          {@render check()}
        </button>
        <button
          class="card-edit"
          aria-disabled={busy}
          aria-label={item.saved ? `Edit ${item.name}` : `Customize ${item.name}`}
          title={item.saved ? 'Edit' : 'Customize a copy'}
          onclick={() => editItem(item)}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.7 2.5 2.5M2.5 13.5l.6-2.7 7.7-7.7 2.1 2.1-7.7 7.7-2.7.6Z"/></svg>
        </button>
      </div>
    {/each}
  </div>
{/if}

<h3 class="settings-subhead font-title">Interface font</h3>
<p class="font-desc">Applies to the main window. The dictation pill keeps its own compact type.</p>
<div
  class="font-grid"
  role="radiogroup"
  aria-label="Interface font"
  data-setting-target="appearance-font"
  style:--seg-index={fontIndex}
  bind:this={fontGroup}
>
  {#each INTERFACE_FONT_OPTIONS as option, index (option.id)}
    {@const checked = appStore.interfaceFont === option.id}
    <button
      class="look-card font-card font-{option.id}"
      class:selected={checked}
      role="radio"
      aria-checked={checked}
      aria-label={option.label}
      aria-describedby={`font-${option.id}-desc`}
      aria-disabled={fontSaving || resetting}
      tabindex={checked ? 0 : -1}
      onclick={() => chooseFont(option.id)}
      onkeydown={(event) => radioKeydown(event, fontGroup, INTERFACE_FONT_OPTIONS, index, (f) => void chooseFont(f.id))}
    >
      <span class="font-sample" aria-hidden="true">Aa</span>
      <span class="card-meta">
        <span class="card-name">{option.label}</span>
        <span class="card-desc" id={`font-${option.id}-desc`}>{option.description}</span>
      </span>
      {@render check()}
    </button>
  {/each}
</div>

{#if error}
  <p class="appearance-error" role="alert">{error}</p>
{/if}

<style>
  /* ---------- Hero: a live miniature of the app drawn from the active tokens ---------- */
  .appearance-hero {
    align-items: stretch;
    background: var(--settings-card-bg, var(--bg-elev));
    border: 1px solid var(--line);
    border-radius: var(--r-lg, 16px);
    display: grid;
    gap: 20px;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
    margin-bottom: 4px;
    padding: 14px;
  }
  .live-window {
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 10px;
    display: flex;
    min-height: 172px;
    overflow: hidden;
    position: relative;
  }
  .live-window, .lw-side, .lw-card, .lw-btn, .lw-toggle, .lw-pill, .lw-nav.active {
    transition: background-color var(--ui-duration-base) var(--ui-ease-out), border-color var(--ui-duration-base) var(--ui-ease-out), color var(--ui-duration-base) var(--ui-ease-out);
  }
  .lw-side {
    background: var(--sidebar-bg);
    border-right: 1px solid var(--line-soft, var(--line));
    display: flex;
    flex: 0 0 30%;
    flex-direction: column;
    gap: 6px;
    padding: 12px 9px;
  }
  .lw-brand, .lw-nav { align-items: center; border-radius: 5px; display: flex; gap: 6px; height: 16px; padding: 0 5px; }
  .lw-brand { margin-bottom: 4px; }
  .lw-brand i { background: var(--ink); border-radius: 3px; height: 9px; width: 9px; }
  .lw-brand b { background: var(--ink); border-radius: 2px; height: 5px; opacity: 0.8; width: 44%; }
  .lw-nav i { border: 1.5px solid var(--ink-mute); border-radius: 2px; height: 7px; width: 7px; }
  .lw-nav b { background: var(--ink-mute); border-radius: 2px; height: 4px; opacity: 0.6; width: 58%; }
  .lw-nav b.short { width: 40%; }
  .lw-nav.active { background: var(--control-hover); }
  .lw-nav.active i { border-color: var(--accent); }
  .lw-nav.active b { background: var(--ink); opacity: 0.85; }
  .lw-main { display: flex; flex: 1; flex-direction: column; gap: 3px; min-width: 0; padding: 12px 12px 10px; position: relative; }
  .lw-title { color: var(--ink); font-family: var(--sans); font-size: 12px; font-weight: 600; letter-spacing: -0.01em; line-height: 1.2; white-space: nowrap; }
  .lw-sub { color: var(--ink-mute); font-family: var(--sans); font-size: 9.5px; line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lw-card { background: var(--bg-elev); border: 1px solid var(--line); border-radius: 7px; display: flex; flex-direction: column; margin-top: 7px; }
  .lw-row { align-items: center; display: flex; gap: 8px; justify-content: space-between; padding: 7px 8px; }
  .lw-row + .lw-row { border-top: 1px solid var(--line); }
  .lw-row b { background: var(--ink-soft); border-radius: 2px; height: 4px; opacity: 0.55; width: 52%; }
  .lw-row b.short { width: 36%; }
  .lw-toggle { background: var(--line-strong); border-radius: 999px; flex: none; height: 10px; position: relative; width: 18px; }
  .lw-toggle i { background: var(--bg-elev); border-radius: 50%; height: 8px; left: 1px; position: absolute; top: 1px; transition: transform var(--ui-duration-base) var(--ui-ease-out); width: 8px; }
  .lw-toggle.on { background: var(--accent); }
  .lw-toggle.on i { background: var(--on-accent); transform: translateX(8px); }
  .lw-actions { display: flex; gap: 5px; margin-top: 9px; }
  .lw-btn { border: 1px solid var(--line); border-radius: 5px; color: var(--ink-soft); font-family: var(--sans); font-size: 8.5px; font-weight: 500; line-height: 1; padding: 4px 8px; }
  .lw-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .lw-pill {
    align-items: center;
    background: var(--pill-bg);
    border: 1px solid var(--pill-line);
    border-radius: 999px;
    bottom: 9px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.18);
    display: flex;
    gap: 2px;
    height: 16px;
    padding: 0 7px;
    position: absolute;
    right: 10px;
  }
  .lw-pill i { animation: lw-wave 1.4s ease-in-out infinite; background: var(--pill-bar); border-radius: 1px; height: 7px; transform-origin: center; width: 2px; }
  .lw-pill i:nth-child(2) { animation-delay: -0.2s; }
  .lw-pill i:nth-child(3) { animation-delay: -0.45s; }
  .lw-pill i:nth-child(4) { animation-delay: -0.7s; }
  .lw-pill i:nth-child(5) { animation-delay: -0.95s; }
  @keyframes lw-wave {
    0%, 100% { transform: scaleY(0.35); }
    50% { transform: scaleY(1); }
  }
  .hero-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; padding: 4px 6px 4px 0; }
  .hero-eyebrow { color: var(--ink-mute); font-size: 11.5px; font-weight: 500; }
  .hero-name { color: var(--ink); font-size: 20px; font-weight: 600; letter-spacing: -0.02em; line-height: 1.25; margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hero-meta { color: var(--ink-mute); font-size: 12px; line-height: 1.5; margin: 6px 0 0; }
  .hero-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 14px; }
  .hero-actions [aria-disabled='true'] { cursor: default; opacity: 0.6; }

  /* ---------- Shared selectable card ---------- */
  .look-card {
    --card-bg: var(--settings-card-bg, var(--bg-elev));
    background: var(--card-bg);
    border: 1px solid var(--line);
    border-radius: var(--r-md, 12px);
    color: var(--ink-soft);
    font-family: var(--sans);
    position: relative;
    text-align: left;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out), background-color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out);
  }
  /* The selection ring is a separate layer so it can fade and settle without animating layout. */
  .look-card::after {
    border: 2px solid var(--accent);
    border-radius: inherit;
    content: '';
    inset: -1px;
    opacity: 0;
    pointer-events: none;
    position: absolute;
    transform: scale(1.015);
    transition: opacity var(--ui-duration-base) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .look-card.selected::after { opacity: 1; transform: scale(1); }
  .look-card.selected { color: var(--ink); }
  .look-card:hover:not(.selected) { border-color: var(--line-strong); }
  button.look-card { cursor: pointer; }
  button.look-card:active:not([aria-disabled='true']) { transform: scale(0.985); }
  button.look-card[aria-disabled='true'] { cursor: default; }
  button.look-card:focus-visible, .card-hit:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .card-preview {
    border-radius: 8px;
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--ink) 10%, transparent);
    display: block;
    overflow: hidden;
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .look-card:hover .card-preview { transform: translateY(-1px); }
  .card-meta { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .card-name { color: var(--ink); font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-desc, .card-tone { color: var(--ink-mute); font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-check {
    align-items: center;
    background: var(--accent);
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--card-bg);
    display: flex;
    height: 18px;
    justify-content: center;
    opacity: 0;
    pointer-events: none;
    position: absolute;
    right: 8px;
    top: 8px;
    transform: scale(0.5);
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-base) cubic-bezier(0.34, 1.56, 0.64, 1);
    width: 18px;
    z-index: 1;
  }
  .card-check svg { fill: none; height: 10px; stroke: var(--on-accent); stroke-linecap: round; stroke-linejoin: round; stroke-width: 2; width: 10px; }
  .selected .card-check { opacity: 1; transform: scale(1); }

  /* ---------- Mode ---------- */
  .mode-grid { display: grid; gap: 10px; grid-template-columns: repeat(var(--modes, 3), minmax(0, 1fr)); }
  .mode-card { display: flex; flex-direction: column; gap: 10px; padding: 8px 8px 10px; }
  .mode-card .card-meta { padding: 0 4px; }
  .mode-card :global(.preview) { aspect-ratio: 2.3 / 1; }
  .mode-note { align-items: flex-start; color: var(--ink-mute); display: flex; font-size: 12px; gap: 7px; line-height: 1.45; margin: 10px 2px 0; }
  .mode-note svg { fill: none; flex: none; height: 14px; margin-top: 1px; stroke: currentColor; stroke-linecap: round; stroke-width: 1.4; width: 14px; }
  .mode-note strong { color: var(--ink-soft); font-weight: 500; }

  /* ---------- Accent ---------- */
  .ap-card {
    background: var(--settings-card-bg, var(--bg-elev));
    border: 1px solid var(--line);
    border-radius: var(--r-lg, 16px);
    padding: 14px 16px 16px;
  }
  .ap-card-desc { color: var(--ink-mute); font-size: 12px; line-height: 1.45; margin: 0 0 12px; }

  /* ---------- Themes ---------- */
  .themes-head { align-items: flex-end; display: flex; gap: 16px; justify-content: space-between; margin: 28px 0 10px; }
  .themes-head .themes-title { margin: 0 0 2px; }
  .themes-desc { color: var(--ink-mute); font-size: 12px; line-height: 1.45; margin: 0; padding-left: 2px; }
  .seg {
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: 9px;
    display: grid;
    flex: none;
    grid-template-columns: repeat(var(--seg-count), minmax(0, 1fr));
    padding: 2px;
    position: relative;
  }
  .seg-thumb {
    background: var(--bg-elev);
    border-radius: 7px;
    box-shadow: 0 0 0 1px var(--line), 0 1px 2px rgba(0, 0, 0, 0.08);
    bottom: 2px;
    left: 2px;
    position: absolute;
    top: 2px;
    transform: translateX(calc(var(--seg-index) * 100%));
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
    width: calc((100% - 4px) / var(--seg-count));
  }
  .seg-btn {
    background: transparent;
    border: 0;
    border-radius: 7px;
    color: var(--ink-soft);
    cursor: pointer;
    font-family: var(--sans);
    font-size: 12px;
    font-weight: 500;
    height: 26px;
    min-width: 52px;
    padding: 0 10px;
    position: relative;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .seg-btn:hover { color: var(--ink-strong); }
  .seg-btn.on { color: var(--ink); }
  .seg-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .theme-gallery { display: grid; gap: 10px; grid-template-columns: repeat(auto-fill, minmax(148px, 1fr)); }
  .theme-card { display: flex; }
  .theme-card.editing { border-color: var(--line-strong); }
  .card-hit {
    background: transparent;
    border: 0;
    border-radius: inherit;
    color: inherit;
    cursor: pointer;
    display: flex;
    flex: 1;
    flex-direction: column;
    font-family: var(--sans);
    gap: 9px;
    min-width: 0;
    padding: 7px 7px 9px;
    text-align: left;
  }
  .card-hit[aria-disabled='true'] { cursor: default; }
  .theme-card .card-meta { padding: 0 34px 0 3px; }
  .card-edit {
    align-items: center;
    background: transparent;
    border: 0;
    border-radius: 7px;
    bottom: 7px;
    color: var(--ink-mute);
    cursor: pointer;
    display: flex;
    height: 28px;
    justify-content: center;
    opacity: 0;
    position: absolute;
    right: 6px;
    transform: translateY(2px);
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out), background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    width: 28px;
  }
  .theme-card:hover .card-edit, .theme-card:focus-within .card-edit, .theme-card.selected .card-edit, .theme-card.editing .card-edit { opacity: 1; transform: none; }
  .card-edit:hover { background: var(--control-hover); color: var(--ink); }
  .card-edit:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; }
  .card-edit[aria-disabled='true'] { cursor: default; opacity: 0.4; }
  .card-edit svg { fill: none; height: 13px; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.5; width: 13px; }
  @media (hover: none) {
    .card-edit { opacity: 1; transform: none; }
  }
  .create-card {
    align-items: center;
    background: transparent;
    border-style: dashed;
    border-color: var(--line-strong);
    display: flex;
    flex-direction: column;
    gap: 10px;
    justify-content: center;
    min-height: 120px;
    padding: 12px;
    text-align: center;
  }
  .create-card:hover:not([aria-disabled='true']) { background: var(--control-hover); border-color: var(--ink-faint); }
  .create-card .card-name, .create-card .card-desc { white-space: normal; }
  .create-icon {
    align-items: center;
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: 50%;
    color: var(--ink-soft);
    display: flex;
    height: 30px;
    justify-content: center;
    transition: transform var(--ui-duration-base) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    width: 30px;
  }
  .create-card:hover .create-icon { color: var(--ink); transform: rotate(90deg); }
  .create-icon svg { fill: none; height: 14px; stroke: currentColor; stroke-linecap: round; stroke-width: 1.6; width: 14px; }
  .create-copy { display: flex; flex-direction: column; gap: 1px; }
  .gallery-empty { color: var(--ink-mute); font-size: 12px; margin: 10px 2px 0; }

  /* ---------- Font ---------- */
  h3.font-title { margin-bottom: 2px; }
  .font-desc { color: var(--ink-mute); font-size: 12px; line-height: 1.45; margin: 0 0 10px; padding-left: 2px; }
  .font-grid { display: grid; gap: 10px; grid-template-columns: repeat(3, minmax(0, 1fr)); }
  .font-card { align-items: center; display: flex; gap: 12px; padding: 12px 14px; }
  .font-sample {
    align-items: center;
    background: var(--paper-2);
    border-radius: 8px;
    color: var(--ink);
    display: flex;
    flex: none;
    font-size: 18px;
    font-weight: 500;
    height: 40px;
    justify-content: center;
    letter-spacing: -0.01em;
    width: 44px;
  }
  .font-default .font-sample { font-family: 'Inter Tight Variable', 'Inter Tight', ui-sans-serif, sans-serif; }
  .font-system .font-sample { font-family: system-ui, -apple-system, 'Segoe UI', 'Noto Sans', Ubuntu, Cantarell, sans-serif; }
  .font-mono .font-sample { font-family: var(--mono); font-size: 16px; letter-spacing: -0.03em; }
  .font-card .card-check { top: 50%; margin-top: -9px; right: 12px; }
  .font-card .card-meta { padding-right: 22px; }

  .appearance-error { color: var(--danger); font-size: 12px; line-height: 1.45; margin: 12px 2px 0; }

  @container settings-panel (max-width: 560px) {
    .appearance-hero { grid-template-columns: minmax(0, 1fr); gap: 14px; }
    .hero-copy { padding: 0 4px 2px; }
    .font-grid { grid-template-columns: minmax(0, 1fr); }
    .themes-head { align-items: flex-start; flex-direction: column; gap: 10px; }
  }
  @container settings-panel (max-width: 520px) {
    .mode-grid { grid-template-columns: repeat(min(var(--modes, 3), 3), minmax(0, 1fr)); gap: 8px; }
    .mode-card .card-desc { display: none; }
    .mode-card :global(.preview) { aspect-ratio: 1.6 / 1; }
    .theme-gallery { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .look-card, .look-card::after, .card-preview, .card-check, .card-edit, .seg-thumb, .create-icon, .lw-toggle i,
    .live-window, .lw-side, .lw-card, .lw-btn, .lw-toggle, .lw-pill, .lw-nav.active { transition-duration: 1ms; }
    .lw-pill i { animation: none; transform: scaleY(0.7); }
    .look-card:hover .card-preview, .create-card:hover .create-icon { transform: none; }
  }
</style>
