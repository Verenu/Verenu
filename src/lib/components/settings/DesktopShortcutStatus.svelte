<script lang="ts">
  import { onMount } from 'svelte';
  import { desktopShortcut, loadDesktopShortcuts, type ShortcutStatus } from '../../shortcutStatus.svelte';
  import { isLinux } from '../../platform';
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
  <div class="shortcut-conflict" role="status">
    <p class="desc"><strong>{status.conflict.trigger}</strong> is already assigned to {status.conflict.description || 'a desktop shortcut'}. The desktop shortcut takes priority{ status.conflict.while_holding ? ' while you hold the dictation hotkey' : '' }.</p>
    <p class="desc">Choose another key above{ status.conflict.while_holding ? ', change your dictation hotkey,' : '' } or change the desktop shortcut. You can also give an agent the details below.</p>
    <button type="button" class="btn-ghost btn-compact" onclick={() => { showPrompt = !showPrompt; }}>{showPrompt ? 'Hide agent prompt' : 'Show agent prompt'}</button>
    {#if showPrompt}
      <p class="desc">Review this machine-specific prompt before sharing it.</p>
      <textarea aria-label="Agent prompt" readonly value={prompt} rows="5"></textarea>
      <button type="button" class="btn-ghost btn-compact" onclick={copyPrompt}>Copy prompt</button>
      <span class="desc" aria-live="polite">{copyMessage}</span>
    {/if}
  </div>
{:else if status?.note}
  <p class="desc" role={status.active === null ? 'alert' : 'status'}>{id === 'dictation' && status.active === null && isLinux && status.note.startsWith('No available shortcut.')
    ? 'The desktop shortcut could not be registered. Verenu requires working Hyprland integration; Xfce/X11 shortcut capture is not supported. On Hyprland, check the portal and conflicting desktop bindings.'
    : status.note}</p>
{/if}

<style>
  .shortcut-conflict { margin: 8px 0 12px; }
  .shortcut-conflict p { margin: 6px 0; }
  .shortcut-conflict textarea { display: block; box-sizing: border-box; width: 100%; margin: 8px 0; padding: 8px; color: inherit; background: transparent; border: 1px solid var(--border-color, #555); border-radius: 6px; resize: vertical; }
</style>
