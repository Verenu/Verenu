<script lang="ts">
  import { modalFocusTrap } from '../modalFocus';
  import { portal } from '../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../motion';

  let { busy, ondeny, onapprove }: {
    busy: boolean;
    ondeny: () => void;
    onapprove: () => void;
  } = $props();
  let denyButton = $state<HTMLButtonElement | null>(null);

  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || busy) return;
    event.preventDefault();
    ondeny();
  }
</script>

<svelte:window onkeydown={onKeydown} />
<button type="button" class="ui-modal-backdrop agent-pair-backdrop" aria-label="Deny agent access"
  use:portal disabled={busy} onclick={ondeny} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
<div class="ui-modal-card ui-modal-card--portalled agent-pair-dialog" use:portal
  use:modalFocusTrap={{ active: true, initialFocus: () => denyButton, restoreFocus: () => null }}
  role="dialog" aria-modal="true" aria-labelledby="agent-pair-title" aria-describedby="agent-pair-body" tabindex="-1"
  in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
  out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}>
  <div class="ui-modal-head"><h2 id="agent-pair-title" class="ui-modal-title">Allow agent access?</h2></div>
  <div class="ui-modal-body"><p id="agent-pair-body" class="ui-modal-copy">An agent on this computer wants access to Verenu. Approved agents can read and change your settings, Contexts, and retained dictation history. Other local agents can share this access until Verenu quits.</p></div>
  <div class="ui-modal-foot"><div class="ui-modal-actions">
    <button bind:this={denyButton} type="button" class="btn-ghost" disabled={busy} onclick={ondeny}>Deny</button>
    <button type="button" class="btn-primary" disabled={busy} onclick={onapprove}>Allow until Verenu quits</button>
  </div></div>
</div>

<style>
  .agent-pair-backdrop { border: 0; cursor: default; padding: 0; z-index: 160; }
  .agent-pair-dialog { z-index: 161; }
</style>
