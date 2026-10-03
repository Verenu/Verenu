<script lang="ts">
  import { invoke } from '../../tauri';
  import MacPermissions from '../MacPermissions.svelte';
  import AndroidPermissionsStep from '../../setup/steps/AndroidPermissionsStep.svelte';
  import { isAndroid } from '../../platform';
  import type { ProviderId } from '../../settings';

  // Surface the Keychain row for whichever provider's key is configured.
  let provider = $state<ProviderId | null>(null);

  invoke<string | null>('get_setting', { key: 'transcription_provider' })
    .then((p) => { if (p) provider = p as ProviderId; })
    .catch(() => {});
</script>

<h2 class="settings-h">Permissions</h2>
{#if isAndroid}
  <p class="panel-note">
    Review what Verenu can do on this device. Reinstalling or updating the app can
    switch the Accessibility Service off, which hides the dictation pill.
  </p>

  <div data-setting-target="permissions">
    <AndroidPermissionsStep />
  </div>
{:else}
  <p class="panel-note">
    Verenu needs these macOS permissions to capture your voice and type into other
    apps. Anything not granted will stop dictation from working everywhere.
  </p>

  <div data-setting-target="permissions">
    <MacPermissions variant="settings" {provider} />
  </div>
{/if}

<style>
  h2 { margin-bottom: 6px; }
</style>
