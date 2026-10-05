<script lang="ts">
  import { onMount } from 'svelte';
  import { desktopShortcut, loadDesktopShortcuts, type ShortcutStatus } from '../../shortcutStatus.svelte';
  let { id }: { id: ShortcutStatus['id'] } = $props();
  const status = $derived(desktopShortcut(id));
  onMount(() => { void loadDesktopShortcuts(); });
</script>

{#if status?.note}
  <p class="desc" role={status.active === null ? 'alert' : 'status'}>{status.note}</p>
{/if}
