<script lang="ts">
  import MacPermissions from '../../components/MacPermissions.svelte';
  import type { ProviderId } from '../../settings';

  let { provider, allCoreGranted = $bindable(false) }: { provider: ProviderId; allCoreGranted?: boolean } = $props();
</script>

<div class="step perm-step">
  <MacPermissions variant="setup" {provider} bind:allGranted={allCoreGranted} />
</div>

<style>
  .perm-step { max-width: 680px; }
</style>
