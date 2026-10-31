<script lang="ts">
  import { desktopShortcut } from '../shortcutStatus.svelte';
  import { isAndroid, isLinux } from '../platform';
  import { openSetupSettings } from '../settingsNavigation';
  import Callout from './Callout.svelte';

  const status = $derived(desktopShortcut('dictation'));
  const unavailable = $derived(!isAndroid && status?.active === null);
  const detail = $derived(status?.note?.startsWith('No available shortcut.') ? null : status?.note);
</script>

{#if unavailable}
  <div class="shortcut-recovery">
    <Callout
      severity="warning"
      title="Dictation shortcut unavailable"
      action={{ label: 'Open shortcut settings', onclick: () => openSetupSettings('general') }}
    >
      <p>{isLinux
        ? 'Global dictation shortcuts require working Hyprland desktop integration. Xfce/X11 shortcut capture is not supported.'
        : 'The desktop has not registered a dictation shortcut. Check its configuration and permissions.'}</p>
      {#if detail}<p>{detail}</p>{/if}
    </Callout>
  </div>
{/if}

<style>
  .shortcut-recovery { margin-bottom: 16px; }
</style>
