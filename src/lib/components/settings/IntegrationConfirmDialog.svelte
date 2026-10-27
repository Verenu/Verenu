<script lang="ts">
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../../motion';

  let {
    title,
    body,
    confirmLabel,
    busy = false,
    priority = false,
    oncancel,
    onconfirm,
    restoreFocus,
  }: {
    title: string;
    body: string;
    confirmLabel: string;
    busy?: boolean;
    priority?: boolean;
    oncancel: () => void;
    onconfirm: () => void;
    restoreFocus: () => HTMLElement | null;
  } = $props();

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
  class:priority
  aria-label="Close dialog"
  use:portal
  disabled={busy}
  onclick={oncancel}
  in:modalBackdrop={{ duration: 180 }}
  out:modalBackdrop={{ duration: 160 }}
></button>

<div
  class="ui-modal-card ui-modal-card--portalled integration-confirm"
  class:priority
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
  .integration-confirm-backdrop { border: 0; cursor: default; padding: 0; z-index: 70; }
  .integration-confirm { z-index: 71; }
  .integration-confirm-backdrop.priority { z-index: 110; }
  .integration-confirm.priority { z-index: 111; }
</style>
