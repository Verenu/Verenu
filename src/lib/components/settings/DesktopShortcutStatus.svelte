<script lang="ts">
  import { onMount } from 'svelte';
  import { desktopShortcut, loadDesktopShortcuts, type ShortcutStatus } from '../../shortcutStatus.svelte';
  import { isLinux } from '../../platform';
  import Callout from '../Callout.svelte';
  let { id }: { id: ShortcutStatus['id'] } = $props();
  const status = $derived(desktopShortcut(id));
  let showPrompt = $state(false);
  let copyMessage = $state('');
  const prompt = $derived(status?.conflict ? `Help me resolve a Linux shortcut conflict on my ${status.conflict.desktop} desktop. Verenu's ${id === 'handsfree' ? 'switch to hands-free' : 'cancel dictation'} shortcut is ${status.requested}. The conflicting combination is ${status.conflict.trigger}${status.conflict.description ? `, described by the desktop as "${status.conflict.description}"` : ''}. Inspect my current desktop shortcut configuration, explain which binding owns this combination, and help me choose a different desktop binding or Verenu shortcut. Preserve unrelated shortcuts and ask before changing my desktop configuration.` : '');
  async function copyPrompt() {
    try { await navigator.clipboard.writeText(prompt); copyMessage = 'Prompt copied.'; }
    catch { copyMessage = 'Could not copy. Select the prompt below to copy it.'; showPrompt = true; }
  }
  onMount(() => { void loadDesktopShortcuts(); });
</script>

{#if status?.conflict}
  <div class="shortcut-conflict">
    <Callout
      severity="warning"
      compact
      title={`${status.conflict.trigger} is already assigned to ${status.conflict.description || 'a desktop shortcut'}`}
      action={{ label: showPrompt ? 'Hide agent prompt' : 'Show agent prompt', onclick: () => { showPrompt = !showPrompt; } }}
    >The desktop shortcut takes priority{ status.conflict.while_holding ? ' while you hold the dictation hotkey' : '' }. Choose another key{ status.conflict.while_holding ? ', change your dictation hotkey,' : '' } or change the desktop shortcut. An agent can help with the prompt.</Callout>
    {#if showPrompt}
      <p class="desc">Review this machine-specific prompt before sharing it.</p>
      <textarea aria-label="Agent prompt" readonly value={prompt} rows="5"></textarea>
      <button type="button" class="btn-ghost btn-compact" onclick={copyPrompt}>Copy prompt</button>
      <span class="desc" aria-live="polite">{copyMessage}</span>
    {/if}
  </div>
{:else if status?.note}
  <div class="shortcut-note">
    <Callout severity={status.active === null ? 'error' : 'info'} compact>{id === 'dictation' && status.active === null && isLinux && status.note.startsWith('No available shortcut.')
      ? 'The desktop shortcut could not be registered. Verenu requires working Hyprland integration; Xfce/X11 shortcut capture is not supported. On Hyprland, check the portal and conflicting desktop bindings.'
      : status.note}</Callout>
  </div>
{/if}

<style>
  .shortcut-conflict, .shortcut-note { margin-top: 10px; }
  .shortcut-conflict p { margin: 8px 0 6px; }
  .shortcut-conflict textarea { display: block; box-sizing: border-box; width: 100%; margin: 0 0 8px; padding: 8px 10px; color: var(--ink-soft); background: transparent; border: 1px solid var(--line-strong); border-radius: var(--r-sm); font: 11.5px/1.5 var(--mono); resize: vertical; }
  .shortcut-conflict textarea:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
