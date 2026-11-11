<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { invoke } from '../../tauri';
  import { appStore, formatIpcError, type ContextSubApp } from '../../stores';
  import { contextsStore, loadContexts } from '../../contextsStore.svelte';
  import { saveSetting } from '../../settings';
  import { motionMs, MOTION_MS } from '../../motion';
  import SubAppIcon from '../SubAppIcon.svelte';
  import { isMac } from '../../platform';
  import { desktopShortcut, loadDesktopShortcuts } from '../../shortcutStatus.svelte';
  import DesktopShortcutStatus from './DesktopShortcutStatus.svelte';
  import {
    DEFAULT_SUB_APP_CAPTURE_HOTKEY,
    chordFromKeyboardEvent,
    subAppCaptureKeys,
  } from '../../subApps';

  let recording = $state(false);
  let shortcutError = $state('');
  let removingId = $state<number | null>(null);
  let listError = $state('');

  const chord = $derived(appStore.subAppCaptureHotkey || DEFAULT_SUB_APP_CAPTURE_HOTKEY);
  const activeChord = $derived(desktopShortcut('capture')?.active ?? chord);
  const isDefault = $derived(chord === DEFAULT_SUB_APP_CAPTURE_HOTKEY);
  const subApps = $derived(
    [...contextsStore.subApps].sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' })),
  );
  const PHRASE = { contains: 'contains', starts_with: 'starts with', equals: 'is' } as const;

  onMount(() => {
    void loadContexts();
    void loadDesktopShortcuts();
  });

  function contextName(subApp: ContextSubApp): string | null {
    if (subApp.context_id === null) return null;
    return contextsStore.contexts.find((context) => context.id === subApp.context_id)?.name ?? null;
  }

  async function saveChord(next: string) {
    const previous = appStore.subAppCaptureHotkey;
    appStore.subAppCaptureHotkey = next;
    shortcutError = '';
    try {
      await saveSetting('sub_app_capture_hotkey', next);
    } catch (err) {
      appStore.subAppCaptureHotkey = previous;
      shortcutError = formatIpcError(err, 'Could not save the sub-app capture shortcut');
    }
  }

  function handleRecordKeydown(event: KeyboardEvent) {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') {
      recording = false;
      return;
    }
    const next = chordFromKeyboardEvent(event);
    if (!next) return;
    recording = false;
    void saveChord(next);
  }

  async function remove(subApp: ContextSubApp) {
    removingId = subApp.id;
    listError = '';
    try {
      await invoke('delete_sub_app', { id: subApp.id });
      contextsStore.subApps = contextsStore.subApps.filter((item) => item.id !== subApp.id);
    } catch (err) {
      listError = formatIpcError(err, 'Could not remove this sub-app');
    } finally {
      removingId = null;
    }
  }
</script>

<svelte:window onkeydown={handleRecordKeydown} />

<h2 class="settings-h">Sub-apps</h2>
<p class="settings-lede">A sub-app is one window inside an app, such as a Discord server or a VS Code project, that gets its own Context.</p>

<h3 class="settings-subhead first">Shortcut</h3>

<div class="setting-row" data-setting-target="sub-apps-shortcut">
  <div>
    <div class="label">Capture shortcut</div>
    <div class="desc">Focus a window and press this to save it as a sub-app.</div>
  </div>
  <div class="shortcut-actions">
    {#if !isDefault && !recording}
      <button type="button" class="btn-ghost btn-compact" onclick={() => void saveChord(DEFAULT_SUB_APP_CAPTURE_HOTKEY)}>Reset</button>
    {/if}
    <button
      type="button"
      class="badge key-badge shortcut-btn"
      class:recording
      aria-label={recording ? 'Press the new shortcut, or Escape to cancel' : `Change capture shortcut, currently ${desktopShortcut('capture')?.active === null ? 'unavailable' : subAppCaptureKeys(activeChord).join(' + ')}`}
      onclick={() => { recording = !recording; shortcutError = ''; }}
    >
      {#key recording}
        <span in:fade={{ duration: motionMs(MOTION_MS.fast) }}>
          {recording ? 'Press keys…' : desktopShortcut('capture')?.active === null ? 'Unavailable' : subAppCaptureKeys(activeChord).join(' + ')}
        </span>
      {/key}
    </button>
  </div>
</div>
{#if shortcutError}<p class="section-error" role="alert">{shortcutError}</p>{/if}
<DesktopShortcutStatus id="capture" />
{#if recording}
  <p class="shortcut-hint">Hold {isMac ? '⌘, ⌥, or ⌃' : 'Ctrl, Alt, or Super'} with a letter, number, or F1–F12. Escape cancels.</p>
{/if}

<h3 class="settings-subhead">Your sub-apps</h3>
{#if subApps.length === 0}
  <div class="empty">
    <span class="empty-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="14" rx="2.5"/><path d="M3 9h18M8 21h8"/></svg>
    </span>
    <p class="empty-title">No sub-apps yet</p>
    <p class="empty-text">
      {#if desktopShortcut('capture')?.active === null}
        Choose an available capture shortcut above.
      {:else}
        Focus a window and press
        {#each subAppCaptureKeys(activeChord) as key, index}{#if index > 0}<span class="plus">+</span>{/if}<kbd>{key}</kbd>{/each}
      {/if}
    </p>
  </div>
{:else}
  <ul class="sub-app-list">
    {#each subApps as subApp (subApp.id)}
      {@const context = contextName(subApp)}
      <li class="sub-app-row">
        <span class="sub-app-icon"><SubAppIcon {subApp} label={subApp.app_name ?? subApp.executable} size={18} /></span>
        <div class="sub-app-text">
          <div class="sub-app-name">{subApp.label}</div>
          <div class="sub-app-meta">
            {subApp.app_name ?? subApp.executable} · title {PHRASE[subApp.match_mode]} “{subApp.title_pattern}”
          </div>
          <div class="sub-app-context" class:is-unassigned={!context}>{context ? `In ${context}` : 'Not in a context group'}</div>
        </div>
        <button
          type="button"
          class="btn-ghost btn-compact"
          onclick={() => void remove(subApp)}
          disabled={removingId === subApp.id}
          aria-label={`Remove ${subApp.label}`}
        >Remove</button>
      </li>
    {/each}
  </ul>
{/if}
{#if listError}<p class="section-error" role="alert">{listError}</p>{/if}

<style>
  .shortcut-actions {
    align-items: center;
    display: flex;
    gap: 8px;
  }

  .shortcut-btn {
    cursor: pointer;
    transition: background var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out);
    white-space: nowrap;
  }

  .shortcut-btn:hover {
    background: var(--control-hover);
  }

  .shortcut-btn.recording {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }

  .shortcut-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .shortcut-hint,
  .section-error {
    font-size: 12px;
    line-height: 1.5;
    margin: 8px 2px 0;
  }

  .shortcut-hint { color: var(--ink-mute); }

  .empty {
    border: 1px dashed var(--line);
    border-radius: var(--r-lg, 14px);
    display: grid;
    gap: 6px;
    justify-items: center;
    padding: 32px 16px;
    text-align: center;
  }

  .empty-icon {
    background: var(--paper-2);
    border-radius: 12px;
    color: var(--ink-mute);
    display: grid;
    height: 40px;
    margin-bottom: 4px;
    place-items: center;
    width: 40px;
  }

  .empty-title { color: var(--ink); font-size: 13.5px; font-weight: 600; margin: 0; }
  .empty-text { color: var(--ink-mute); font-size: 12px; line-height: 1.8; margin: 0; }
  .empty .plus { margin: 0 3px; }

  .section-error {
    color: var(--danger);
  }

  .empty kbd {
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    color: var(--ink-mute);
    font-family: var(--mono);
    font-size: 10.5px;
    padding: 2px 5px;
  }

  .sub-app-list {
    background: var(--settings-card-bg, var(--bg-elev));
    border: 1px solid var(--line);
    border-radius: var(--r-lg, 14px);
    display: flex;
    flex-direction: column;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .sub-app-row {
    align-items: center;
    border-bottom: 1px solid var(--line);
    display: grid;
    gap: 12px;
    grid-template-columns: auto minmax(0, 1fr) auto;
    padding: 12px 16px;
  }

  .sub-app-row:last-child {
    border-bottom: 0;
  }

  .sub-app-icon {
    display: grid;
    place-items: center;
    width: 22px;
  }

  .sub-app-name {
    color: var(--ink);
    font-size: 13px;
    font-weight: 500;
  }

  .sub-app-meta {
    color: var(--ink-mute);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub-app-context {
    color: var(--ink-soft);
    font-size: 11.5px;
    margin-top: 1px;
  }

  .sub-app-context.is-unassigned {
    color: var(--ink-faint);
  }
</style>
