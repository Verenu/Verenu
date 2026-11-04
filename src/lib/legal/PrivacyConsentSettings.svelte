<script lang="ts">
  import { onMount } from 'svelte';
  import { formatIpcError } from '../errors';
  import { modalFocusTrap } from '../modalFocus';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../motion';
  import LegalLinks from './LegalLinks.svelte';
  import { CONSENT_STATUS_LABEL, getInstallId } from './consent';
  import { acceptConsent, consentStore, loadConsent, requestDataDeletion } from './consentStore.svelte';

  let installId = $state('');
  let installIdError = $state('');
  let copied = $state(false);
  let copyError = $state(false);
  let confirmOpen = $state(false);
  let cancelButton = $state<HTMLButtonElement | null>(null);
  let deleteButton = $state<HTMLButtonElement | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  const status = $derived(consentStore.state?.status ?? null);
  const pending = $derived(status === 'deletion_pending');
  const canAccept = $derived(status === 'skipped' || status === 'unanswered' || status === 'update_required');
  const acceptedOn = $derived(
    consentStore.state?.acceptedAt
      ? new Date(consentStore.state.acceptedAt).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
      : null,
  );

  const statusDetail = $derived.by(() => {
    if (status !== 'accepted') return '.';
    const when = acceptedOn ? ` on ${acceptedOn}` : '';
    return `${when}${consentStore.state?.syncPending ? ', will sync when you are online' : ''}.`;
  });

  onMount(() => {
    void loadConsent();
    getInstallId().then(
      (id) => (installId = id),
      (err) => (installIdError = formatIpcError(err)),
    );
    return () => clearTimeout(copiedTimer);
  });

  async function copyInstallId() {
    copyError = false;
    try {
      await navigator.clipboard.writeText(installId);
      copied = true;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = false), 1800);
    } catch {
      copied = false;
      copyError = true;
    }
  }

  async function confirmDelete() {
    const ok = await requestDataDeletion();
    if (ok) confirmOpen = false;
  }
</script>

<h3 class="settings-subhead first">Terms and data</h3>

<div class="setting-row" data-setting-target="privacy-consent-status">
  <div>
    <div class="label">Terms and Privacy Policy</div>
    <div class="desc">
      <span data-testid="consent-status">{status ? CONSENT_STATUS_LABEL[status] : 'Loading…'}</span>{statusDetail}
      {#if status === 'skipped'}Analytics and online features are off until you accept.{/if}
    </div>
    <div class="consent-links"><LegalLinks /></div>
    {#if consentStore.error}<div class="desc consent-error" role="alert">{consentStore.error}</div>{/if}
  </div>
  {#if canAccept}
    <button class="btn-primary" onclick={acceptConsent} disabled={consentStore.busy}>Accept terms</button>
  {/if}
</div>

<div class="setting-row" data-setting-target="privacy-install-id">
  <div>
    <div class="label">Install ID</div>
    <div class="desc">A random ID for this installation. Share it with support if you need help. It cannot be used to access or delete your data.</div>
    {#if installIdError}<div class="desc consent-error" role="alert">{installIdError}</div>{/if}
    <code class="install-id" data-testid="install-id">{installId || '…'}</code>
    {#if copyError}<div class="desc consent-error" role="alert">Could not copy. Select the ID and copy it manually.</div>{/if}
  </div>
  <button class="btn-ghost" onclick={copyInstallId} disabled={!installId}>
    <span aria-live="polite">{copied ? 'Copied' : 'Copy'}</span>
  </button>
</div>

<div class="setting-row" data-setting-target="privacy-delete-data">
  <div>
    <div class="label">Delete my data</div>
    <div class="desc">
      {#if pending}
        <span data-testid="deletion-pending">Deletion pending.</span> We have asked the server to delete the data tied to this install. This stays here until the server confirms.
      {:else}
        Deletes the data Verenu holds on its servers for this install, including your recorded acceptance. Local history and settings on this device are not affected.
      {/if}
    </div>
  </div>
  <button
    bind:this={deleteButton}
    class="btn-danger"
    onclick={() => (confirmOpen = true)}
    disabled={pending || consentStore.busy}
  >{pending ? 'Deletion pending' : 'Delete my data'}</button>
</div>

{#if confirmOpen}
  <button class="ui-modal-backdrop" aria-label="Close dialog" tabindex="-1" onclick={() => (confirmOpen = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="ui-modal-card"
    use:modalFocusTrap={{ active: confirmOpen, initialFocus: () => cancelButton, restoreFocus: () => deleteButton }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="delete-data-title"
    tabindex="-1"
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="ui-modal-head"><h2 id="delete-data-title" class="ui-modal-title">Delete your data?</h2></div>
    <div class="ui-modal-body">
      <p class="ui-modal-copy">
        This asks Verenu to delete the server-side data tied to this install, including your recorded acceptance. It cannot be undone. Analytics and online features turn off until you accept again.
      </p>
      {#if consentStore.error}<p class="ui-modal-copy consent-error" role="alert">{consentStore.error}</p>{/if}
    </div>
    <div class="ui-modal-foot">
      <div class="ui-modal-actions">
        <button bind:this={cancelButton} class="btn-ghost" onclick={() => (confirmOpen = false)}>Cancel</button>
        <button class="btn-danger btn-compact" onclick={confirmDelete} disabled={consentStore.busy}>
          {consentStore.busy ? 'Deleting…' : 'Delete my data'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .consent-links { margin-top: 8px; }
  .install-id { display: inline-block; margin-top: 8px; padding: 3px 8px; border: 1px solid var(--line); border-radius: var(--r-sm, 6px); background: var(--paper-2); color: var(--ink-soft); font-size: 12px; user-select: all; word-break: break-all; }
  .consent-error { color: var(--accent); }
</style>
