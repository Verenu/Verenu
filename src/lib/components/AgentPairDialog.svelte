<script lang="ts">
  import { modalFocusTrap } from '../modalFocus';
  import { portal } from '../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../motion';
  import Toggle from './Toggle.svelte';

  let { busy, code, ondeny, onapprove }: {
    busy: boolean;
    code: string;
    ondeny: () => void;
    onapprove: (scopes: string[]) => void;
  } = $props();
  let denyButton = $state<HTMLButtonElement | null>(null);
  let selected = $state<string[]>(['metadata']);
  const permissions = [
    { id: 'metadata', label: 'Read settings, Contexts, and library' },
    { id: 'history_text', label: 'Read dictation text' },
    { id: 'settings_write', label: 'Change settings' },
    { id: 'context_write', label: 'Change Contexts' },
    { id: 'library_write', label: 'Change vocabulary and snippets' },
    { id: 'destructive', label: 'Delete entries and other data' },
    { id: 'diagnostics', label: 'Read diagnostics' },
    { id: 'provider_calls', label: 'Test prompts with providers' },
    { id: 'credentials', label: 'Replace or delete credentials' },
    { id: 'integrations', label: 'Manage sync and integrations' },
  ];

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
  <div class="ui-modal-body"><p id="agent-pair-body" class="ui-modal-copy">Check that the agent's code matches: {code}. Choose what this agent can access. Access expires after one hour or when Verenu quits. Read access includes history metadata.</p>
    <div class="agent-permissions">
      {#each permissions as permission (permission.id)}
        <div class="agent-permission"><span>{permission.label}</span><Toggle checked={selected.includes(permission.id)} label={permission.label} disabled={busy} onchange={(enabled) => { selected = enabled ? [...selected, permission.id] : selected.filter((id) => id !== permission.id); }} /></div>
      {/each}
    </div>
    <p class="ui-modal-copy">Deleting also requires permission to change the affected area. Provider tests may incur charges.</p>
  </div>
  <div class="ui-modal-foot"><div class="ui-modal-actions">
    <button bind:this={denyButton} type="button" class="btn-ghost" disabled={busy} onclick={ondeny}>Deny</button>
    <button type="button" class="btn-primary" disabled={busy} onclick={() => onapprove(selected)}>Allow access</button>
  </div></div>
</div>

<style>
  .agent-pair-backdrop { border: 0; cursor: default; padding: 0; z-index: 160; }
  .agent-pair-dialog { z-index: 161; }
  .agent-permissions { display: grid; gap: 0.65rem; margin: 1rem 0; }
  .agent-permission { display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
</style>
