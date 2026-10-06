<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, listen } from '../tauri';
  import { createListenerScope } from '../listenerScope';
  import type { DumpWindowKind } from '../agentAccessibilityDump';

  let { windowKind, extras = {} }: {
    windowKind: DumpWindowKind;
    extras?: Record<string, unknown>;
  } = $props();
  let enabled = $state(false);
  let Runtime = $state<typeof import('./AgentAccessibilityDumpRuntime.svelte').default | null>(null);

  onMount(() => {
    let active = true;
    let changed = false;
    const listeners = createListenerScope();
    void listeners.track(listen<boolean>('verenu:ruin-accessibility-changed', ({ payload }) => {
      changed = true;
      if (active) enabled = Boolean(payload);
    })).catch((error) => console.error('Failed to listen for ruin-accessibility changes:', error));
    void invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' })
      .then((value) => { if (active && !changed) enabled = value === true; })
      .catch((error) => console.error('Failed to load ruin_accessibility:', error));
    return () => { active = false; listeners.dispose(); };
  });

  $effect(() => {
    if (!enabled || Runtime) return;
    let active = true;
    void import('./AgentAccessibilityDumpRuntime.svelte')
      .then((module) => { if (active) Runtime = module.default; })
      .catch((error) => console.error('Failed to load accessibility dump:', error));
    return () => { active = false; };
  });
</script>

{#if enabled && Runtime}
  <Runtime {windowKind} {extras} />
{/if}
