<script lang="ts">
  import { appStore } from '../../stores';
  import type { AppearanceMode } from '../../settings';
  import { findActiveSavedTheme, CUSTOM_THEME_PRESETS, sameThemeColors } from '../../customTheme';
  import { setAppearanceMode, withAppearanceLock } from '../../appearanceActions';
  import { guardThemeEditor, themeEditor } from '../../themeEditor.svelte';
  import { directionFromOrder, SETTINGS_SECTION_ORDER } from '../../motion';

  const modes: { id: AppearanceMode; label: string }[] = [
    { id: 'system', label: 'System' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' },
  ];

  let error = $state('');
  let group: HTMLElement | null = $state(null);
  const busy = $derived(appStore.appearanceSaving || themeEditor.saving);
  const activeIndex = $derived(modes.findIndex((mode) => mode.id === appStore.appearanceMode));

  const themeName = $derived.by(() => {
    if (appStore.appearanceMode === 'omarchy') return 'Omarchy';
    if (appStore.appearanceMode !== 'custom') return null;
    return findActiveSavedTheme(appStore.savedThemes, appStore.customTheme, appStore.activeThemeId)?.name
      ?? CUSTOM_THEME_PRESETS.find((preset) => sameThemeColors(preset.theme, appStore.customTheme))?.label
      ?? 'a custom theme';
  });

  function choose(mode: AppearanceMode) {
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

  function handleKeydown(event: KeyboardEvent, index: number) {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = (index + step + modes.length) % modes.length;
    group?.querySelectorAll<HTMLElement>('[role="radio"]')[next]?.focus();
    choose(modes[next].id);
  }

  function openAppearance() {
    appStore.settingsAnimDir = directionFromOrder(appStore.settingsSection, 'appearance', SETTINGS_SECTION_ORDER);
    appStore.settingsSection = 'appearance';
  }
</script>

<div class="setting-row quick-appearance" data-setting-target="general-appearance">
  <div>
    <div class="label">Appearance</div>
    <div class="desc">
      {#if themeName}Using {themeName}. Pick a mode to switch back.{:else}Accent colors, themes, and fonts live in{/if}
      <button class="inline-link" onclick={openAppearance}>{themeName ? 'Change theme' : 'Appearance'}</button>{#if !themeName}.{/if}
    </div>
  </div>
  <div
    class="quick-seg"
    class:none={activeIndex < 0}
    role="radiogroup"
    aria-label="Appearance"
    style:--seg-index={Math.max(0, activeIndex)}
    bind:this={group}
  >
    <span class="quick-thumb" aria-hidden="true"></span>
    {#each modes as mode, index (mode.id)}
      {@const checked = appStore.appearanceMode === mode.id}
      <button
        class="quick-btn appearance-option"
        class:on={checked}
        role="radio"
        aria-checked={checked}
        aria-disabled={busy}
        tabindex={checked || (activeIndex < 0 && index === 0) ? 0 : -1}
        onclick={() => choose(mode.id)}
        onkeydown={(event) => handleKeydown(event, index)}
      >{mode.label}</button>
    {/each}
  </div>
</div>
{#if error}
  <p class="quick-error" role="alert">{error}</p>
{/if}

<style>
  .inline-link {
    background: none;
    border: 0;
    border-radius: 3px;
    color: var(--ink-soft);
    cursor: pointer;
    font: inherit;
    padding: 0;
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: 2px;
    transition: color var(--ui-duration-fast) var(--ui-ease-out), text-decoration-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .inline-link:hover { color: var(--ink); text-decoration-color: currentColor; }
  .inline-link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .quick-seg {
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: 9px;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    padding: 2px;
    position: relative;
  }
  .quick-thumb {
    background: var(--bg-elev);
    border-radius: 7px;
    bottom: 2px;
    box-shadow: 0 0 0 1px var(--line), 0 1px 2px rgba(0, 0, 0, 0.08);
    left: 2px;
    position: absolute;
    top: 2px;
    transform: translateX(calc(var(--seg-index) * 100%));
    transition: transform var(--ui-duration-base) var(--ui-ease-out), opacity var(--ui-duration-fast) var(--ui-ease-out);
    width: calc((100% - 4px) / 3);
  }
  .quick-seg.none .quick-thumb { opacity: 0; }
  .quick-btn {
    background: transparent;
    border: 0;
    border-radius: 7px;
    color: var(--ink-soft);
    cursor: pointer;
    font-family: var(--sans);
    font-size: 12.5px;
    font-weight: 500;
    height: 28px;
    min-width: 64px;
    padding: 0 12px;
    position: relative;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .quick-btn:hover { color: var(--ink-strong); }
  .quick-btn.on { color: var(--ink); }
  .quick-btn[aria-disabled='true'] { cursor: default; }
  .quick-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .quick-error { color: var(--danger); font-size: 12px; line-height: 1.45; margin: 8px 2px 0; }
  @media (prefers-reduced-motion: reduce) {
    .quick-thumb, .quick-btn, .inline-link { transition-duration: 1ms; }
  }
</style>
