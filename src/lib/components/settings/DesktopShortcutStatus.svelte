<script lang="ts">
  import { onMount } from 'svelte';
  import { desktopShortcut, loadDesktopShortcuts, type ShortcutStatus } from '../../shortcutStatus.svelte';
  import { isLinux } from '../../platform';
  let { id }: { id: ShortcutStatus['id'] } = $props();
  const status = $derived(desktopShortcut(id));
  onMount(() => { void loadDesktopShortcuts(); });
</script>

{#if status?.note}
  <p class="desc" role={status.active === null ? 'alert' : 'status'}>{id === 'dictation' && status.active === null && isLinux && status.note.startsWith('No available shortcut.')
    ? 'The desktop shortcut could not be registered. Verenu requires working Hyprland integration; Xfce/X11 shortcut capture is not supported. On Hyprland, check the portal and conflicting desktop bindings.'
    : status.note}</p>
{/if}
