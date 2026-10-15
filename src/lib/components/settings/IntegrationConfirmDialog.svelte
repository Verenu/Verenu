<script lang="ts">
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../../motion';

  let {
    title,
    body,
    confirmLabel,
    busy = false,
    oncancel,
    onconfirm,
    restoreFocus,
  }: {
    title: string;
    body: string;
    confirmLabel: string;
    busy?: boolean;
    oncancel: () => void;
    onconfirm: () => void;
    restoreFocus: () => HTMLElement | null;
  } = $props();

  // One confirmation is open at a time, so fixed ids keep test selectors stable.
  let cancelButton = $state<HTMLButtonElement | null>(null);

  function onWindowKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || busy) return;
    event.preventDefault();
    event.stopPropagation();
    oncancel();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<button
  type="button"
  class="ui-modal-backdrop integration-confirm-backdrop"
  aria-label="Close dialog"
  use:portal
  disabled={busy}
  onclick={oncancel}
  in:modalBackdrop={{ duration: 180 }}
  out:modalBackdrop={{ duration: 160 }}
></button>

<div
  class="ui-modal-card ui-modal-card--portalled integration-confirm"
  use:portal
  use:modalFocusTrap={{ active: true, initialFocus: () => cancelButton, restoreFocus }}
  role="dialog"
  aria-modal="true"
  aria-labelledby="integration-confirm-title"
  aria-describedby="integration-confirm-body"
  tabindex="-1"
  in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
  out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
>
  <div class="ui-modal-head">
    <h2 id="integration-confirm-title" class="ui-modal-title">{title}</h2>
  </div>
  <div class="ui-modal-body">
    <p id="integration-confirm-body" class="ui-modal-copy">{body}</p>
  </div>
  <div class="ui-modal-foot">
    <div class="ui-modal-actions">
      <button bind:this={cancelButton} type="button" class="btn-ghost" disabled={busy} onclick={oncancel}>Cancel</button>
      <button type="button" class="btn-danger" disabled={busy} onclick={onconfirm}>{confirmLabel}</button>
    </div>
  </div>
</div>

<style>
  /* A native button backs the backdrop so it can be dismissed from the keyboard. */
  /* The portal moves these to the body, outside Settings' z-index 60 layer, so they
     sit at the same level as the sync dialogs. The shared shell stays at 50/51. */
  .integration-confirm-backdrop { border: 0; cursor: default; padding: 0; z-index: 70; }
  .integration-confirm { z-index: 71; }
</style>
