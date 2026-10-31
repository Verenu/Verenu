<script lang="ts">
  import { appStore } from '../../stores';
  import { emit } from '../../tauri';
  import AppMappingsEditor from '../AppMappingsEditor.svelte';
  import Callout from '../Callout.svelte';
</script>

{#if !appStore.cleanupEnabled}
  <div class="cleanup-off-banner">
    <Callout
      severity="info"
      title="Cleanup is turned off"
      action={{ label: 'Turn cleanup on', onclick: () => emit('open-flow:open-settings-section', 'general') }}
    >App mappings have no effect right now. The tone and per-app overrides here only apply during the cleanup step. Your mappings are kept, just not used.</Callout>
  </div>
{/if}

<div class="mappings-host" data-setting-target="apps-mappings" class:mappings-disabled={!appStore.cleanupEnabled} aria-disabled={!appStore.cleanupEnabled} inert={!appStore.cleanupEnabled}>
  <AppMappingsEditor />
</div>

<style>
  .mappings-host {
    --mappings-measure: none;
  }

  .mappings-disabled {
    opacity: 0.45;
    pointer-events: none;
    user-select: none;
  }

  .cleanup-off-banner {
    margin-bottom: 20px;
  }
</style>
