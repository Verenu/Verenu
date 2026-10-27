<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauriRuntime } from '../tauri';
  import IntegrationConfirmDialog from './settings/IntegrationConfirmDialog.svelte';

  let request = $state<{ code: string; id: string } | null>(null);
  let busy = $state(false);

  async function refresh() {
    if (!isTauriRuntime() || busy) return;
    try { request = await invoke<{ code: string; id: string } | null>('agent_pending_request'); }
    catch { request = null; }
  }

  async function respond(approve: boolean) {
    if (!request || busy) return;
    busy = true;
    try { await invoke('agent_respond', { id: request.id, approve }); }
    finally { request = null; busy = false; }
  }

  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 1000);
    return () => clearInterval(timer);
  });
</script>

{#if request}
  <IntegrationConfirmDialog
    title="Allow agent access?"
    body="An agent is requesting access. Check that its code matches: {request.code}. An approved agent can read and change your Verenu data and settings, including transcripts, replace or delete saved API keys, and manage sync. Access expires after one hour or when Verenu closes."
    confirmLabel="Allow access"
    priority={true}
    {busy}
    oncancel={() => void respond(false)}
    onconfirm={() => void respond(true)}
    restoreFocus={() => null}
  />
{/if}
