<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauriRuntime } from '../tauri';
  import AgentPairDialog from './AgentPairDialog.svelte';

  let request = $state<{ code: string; id: string } | null>(null);
  let busy = $state(false);

  async function refresh() {
    if (!isTauriRuntime() || busy) return;
    try { request = await invoke<{ code: string; id: string } | null>('agent_pending_request'); }
    catch { request = null; }
  }

  async function respond(approve: boolean, scopes: string[] = []) {
    if (!request || busy) return;
    busy = true;
    try { await invoke('agent_respond', { id: request.id, approve, scopes }); }
    finally { request = null; busy = false; }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 1000);
    return () => clearInterval(timer);
  });
</script>

{#if request}
  <AgentPairDialog
    code={request.code}
    {busy}
    ondeny={() => void respond(false)}
    onapprove={(scopes) => void respond(true, scopes)}
  />
{/if}
