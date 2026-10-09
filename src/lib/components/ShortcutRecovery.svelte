<script lang="ts">
  import { desktopShortcut } from '../shortcutStatus.svelte';
  import { isAndroid, isLinux } from '../platform';
  import { openSetupSettings } from '../settingsNavigation';

  const status = $derived(desktopShortcut('dictation'));
  const unavailable = $derived(!isAndroid && status?.active === null);
  const detail = $derived(status?.note?.startsWith('No available shortcut.') ? null : status?.note);
</script>

{#if unavailable}
  <div class="shortcut-recovery" role="status">
    <strong>Dictation shortcut unavailable</strong>
    <p>{isLinux
      ? 'Global dictation shortcuts require working Hyprland desktop integration. Xfce/X11 shortcut capture is not supported.'
      : 'The desktop has not registered a dictation shortcut. Check its configuration and permissions.'}</p>
    {#if detail}<p>{detail}</p>{/if}
    <button class="btn-ghost btn-compact" onclick={() => openSetupSettings('general')}>Open shortcut settings</button>
  </div>
{/if}

<style>
  .shortcut-recovery { padding: 12px 16px; border: 1px solid var(--warning-line); border-radius: var(--r-md); background: var(--warning-bg); margin-bottom: 16px; }
  strong { font-size: 13px; color: var(--ink); }
  p { margin: 6px 0 10px; font-size: 12.5px; line-height: 1.5; color: var(--ink-soft); }
</style>
