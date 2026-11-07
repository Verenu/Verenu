<script lang="ts">
  import { onMount } from 'svelte';
  import { formatIpcError } from '../errors';
  import SetupShell from '../setup/SetupShell.svelte';
  import { getInstallId } from './consent';
  import { consentStore, loadConsent } from './consentStore.svelte';

  /** How often to ask the backend whether the server has confirmed the deletion. */
  const POLL_MS = 10_000;

  let installId = $state('');
  let idError = $state('');
  let copied = $state(false);
  let checking = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  async function check() {
    if (checking) return;
    checking = true;
    try { await loadConsent(); } finally { checking = false; }
  }

  async function copyId() {
    try {
      await navigator.clipboard.writeText(installId);
      copied = true;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = false), 1800);
    } catch { copied = false; }
  }

  onMount(() => {
    getInstallId().then((id) => (installId = id), (err) => (idError = formatIpcError(err)));
    const timer = setInterval(() => void loadConsent(), POLL_MS);
    return () => { clearInterval(timer); clearTimeout(copiedTimer); };
  });
</script>

<SetupShell step={0} totalSteps={0} onDotClick={() => {}}>
  {#snippet right()}
    <button class="btn-ghost ui-focus-ring" onclick={check} disabled={checking}>{checking ? 'Checking…' : 'Check again'}</button>
  {/snippet}

  <div class="step pending-step" data-testid="deletion-pending-screen" role="status" aria-live="polite">
    <span class="pending-chip" data-testid="deletion-pending">Deletion pending</span>
    <h2 class="pending-title" tabindex="-1">Deleting your data</h2>
    <p class="pending-lede">
      Verenu removed your history, settings, dictionary, and saved API keys from this device. It is now waiting for the server to confirm it deleted the record it holds for this install.
    </p>
    <div class="pending-card">
      <p>Nothing else is available until the server confirms. Verenu checks again every minute and each time it starts, so you can close it and come back.</p>
      <div class="pending-id">
        <span class="pending-heading">Install ID</span>
        <code data-testid="install-id">{installId || '…'}</code>
        <button class="btn-ghost btn-compact ui-focus-ring" onclick={copyId} disabled={!installId}><span aria-live="polite">{copied ? 'Copied' : 'Copy'}</span></button>
      </div>
      {#if idError}<p class="pending-error" role="alert">{idError}</p>{/if}
      {#if consentStore.error}<p class="pending-error" role="alert">{consentStore.error}</p>{/if}
    </div>
  </div>
</SetupShell>

<style>
  .pending-step { max-width: 620px; display: flex; flex-direction: column; gap: 14px; }
  .pending-chip { align-self: flex-start; padding: 3px 10px; border: 1px solid var(--line-strong); border-radius: 999px; color: var(--ink-soft); font-size: 12px; font-weight: 600; }
  .pending-title { margin: 0; font-size: 20px; font-weight: 600; color: var(--ink-strong); outline: none; }
  .pending-lede { margin: 0; color: var(--ink-mute); font-size: 13px; line-height: 1.45; }
  .pending-card { display: flex; flex-direction: column; gap: 12px; padding: 17px; border: 1px solid var(--line); border-radius: var(--r-lg); background: var(--paper-2); color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
  .pending-card p { margin: 0; }
  .pending-id { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; }
  .pending-heading { color: var(--ink-soft); font-weight: 600; }
  .pending-id code { padding: 3px 8px; border: 1px solid var(--line); border-radius: 6px; background: var(--paper); color: var(--ink-soft); font-size: 12px; word-break: break-all; user-select: all; }
  .pending-error { color: var(--accent); }
</style>
