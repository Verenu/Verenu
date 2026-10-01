<script lang="ts">
  // Review a capture and save it as a sub-app: an app plus a window-title
  // rule. It goes into the sub-app list and is assigned to a context group
  // from that group's page.
  import { invoke } from '../tauri';
  import { formatIpcError, type ContextSubApp, type TitleMatchMode } from '../stores';
  import { contextsStore } from '../contextsStore.svelte';
  import { modalFocusTrap } from '../modalFocus';
  import { modalBackdrop, modalCard, MOTION_MS, MOTION_PX, motionMs, motionPx } from '../motion';
  import { scale } from 'svelte/transition';
  import { backOut } from 'svelte/easing';
  import CompactSelect from './CompactSelect.svelte';
  import AppIcon from './AppIcon.svelte';
  import {
    MATCH_MODES,
    SUB_APP_LABEL_LIMIT,
    SUB_APP_PATTERN_LIMIT,
    suggestLabel,
    titleRuleMatches,
    type SubAppCapture,
  } from '../subApps';

  let {
    sheet,
    onClose,
  }: {
    sheet: { mode: 'capture'; capture: SubAppCapture };
    onClose: () => void;
  } = $props();

  // The sheet mounts fresh for each capture, so reading it once is intended.
  // svelte-ignore state_referenced_locally
  const capture = sheet.capture;
  const executable = capture.executable;
  const appName = capture.app_name || executable;

  let pattern = $state(capture.proposed_pattern);
  let label = $state(suggestLabel(capture.proposed_pattern));
  let matchMode = $state<TitleMatchMode>('contains');
  let saving = $state(false);
  let error = $state('');
  let labelInput = $state<HTMLInputElement | null>(null);

  const matchesCapture = $derived(titleRuleMatches(pattern, matchMode, capture.window_title));
  const canSave = $derived(!saving && pattern.trim().length > 0 && label.trim().length > 0);
  const modeOptions = MATCH_MODES.map((mode) => ({ value: mode.id, label: mode.label }));

  async function save() {
    if (!canSave) return;
    saving = true;
    error = '';
    try {
      const saved = await invoke<ContextSubApp>('create_sub_app', {
        executable,
        appName,
        label: label.trim(),
        icon: null,
        titlePattern: pattern.trim(),
        matchMode,
      });
      contextsStore.subApps = [...contextsStore.subApps.filter((s) => s.id !== saved.id), saved];
      onClose();
    } catch (err) {
      error = formatIpcError(err);
    } finally {
      saving = false;
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    // An open menu inside the sheet closes first (Dropdown's convention).
    if (event.key === 'Escape' && !document.querySelector('.sub-app-sheet [aria-expanded="true"]')) onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<button type="button" class="ui-modal-backdrop sub-app-backdrop" aria-label="Close dialog" tabindex="-1" onclick={onClose} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
<div
  class="ui-modal-card sub-app-sheet"
  role="dialog"
  aria-modal="true"
  aria-labelledby="sub-app-sheet-title"
  tabindex="-1"
  use:modalFocusTrap={{ active: true, initialFocus: () => labelInput }}
  in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
  out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
>
  <div class="ui-modal-head">
    <h2 id="sub-app-sheet-title" class="ui-modal-title">New sub-app</h2>
    <div class="sub-app-app">
      <AppIcon exe={executable} label={appName} size={18} />
      <span>{appName}</span>
    </div>
  </div>

  <div class="ui-modal-body">
    <label class="sub-app-field" for="sub-app-label">Name</label>
    <input
      id="sub-app-label"
      class="ui-input"
      type="text"
      maxlength={SUB_APP_LABEL_LIMIT}
      placeholder="e.g. Acme Slack"
      bind:value={label}
      bind:this={labelInput}
      autocomplete="off"
      onkeydown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void save(); } }}
    />

    <span class="sub-app-field">Window title</span>
    <div class="sub-app-rule">
      <CompactSelect
        value={matchMode}
        options={modeOptions}
        label="Title match"
        onchange={(mode) => (matchMode = mode)}
      />
      <div class="sub-app-pattern">
        <input
          class="ui-input ui-input--dense"
          type="text"
          maxlength={SUB_APP_PATTERN_LIMIT}
          aria-label="Window title text"
          aria-describedby="sub-app-match"
          bind:value={pattern}
          autocomplete="off"
          spellcheck="false"
        />
        <span id="sub-app-match" class="sub-app-match" class:is-match={matchesCapture} role="status">
          {#key matchesCapture}
            <svg
              in:scale={{ start: 0.4, duration: motionMs(MOTION_MS.base), easing: backOut }}
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.6"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-label={matchesCapture ? 'Matches this window' : "Doesn't match this window"}
            >
              {#if matchesCapture}<path d="M20 6 9 17l-5-5"/>{:else}<path d="M18 6 6 18M6 6l12 12"/>{/if}
            </svg>
          {/key}
        </span>
      </div>
    </div>

    {#if error}<p class="save-error" role="alert">{error}</p>{/if}
  </div>

  <div class="ui-modal-foot ui-modal-actions">
    <button type="button" class="btn-ghost" onclick={onClose}>Cancel</button>
    <button type="button" class="btn-primary" onclick={save} disabled={!canSave}>
      Create
    </button>
  </div>
</div>

<style>
  .sub-app-backdrop {
    appearance: none;
    border: 0;
    padding: 0;
  }

  .sub-app-sheet {
    --ui-modal-w: 460px;
    display: flex;
    flex-direction: column;
  }

  .sub-app-app {
    align-items: center;
    color: var(--ink-mute);
    display: flex;
    font-size: 12.5px;
    gap: 8px;
    margin-top: 6px;
  }

  .sub-app-field {
    color: var(--ink-soft);
    display: block;
    font-size: 12px;
    font-weight: 500;
    margin: 14px 0 6px;
  }

  #sub-app-label {
    box-sizing: border-box;
    width: 100%;
  }

  .sub-app-rule {
    align-items: center;
    display: flex;
    gap: 8px;
  }

  .sub-app-pattern {
    flex: 1;
    min-width: 0;
    position: relative;
  }

  .sub-app-pattern input {
    box-sizing: border-box;
    padding-right: 30px;
    width: 100%;
  }

  .sub-app-match {
    color: var(--danger);
    display: grid;
    pointer-events: none;
    place-items: center;
    position: absolute;
    right: 10px;
    top: 50%;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
    translate: 0 -50%;
  }

  .sub-app-match.is-match {
    color: var(--success);
  }

  .sub-app-match svg {
    grid-area: 1 / 1;
  }

  .save-error {
    color: var(--danger);
    font-size: 12px;
    margin: 12px 0 0;
  }
</style>
