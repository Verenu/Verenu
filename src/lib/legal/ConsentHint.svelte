<script lang="ts">
  import { consentStore } from './consentStore.svelte';
  import { isConsentGranted } from './consent';

  /** Shown under a feature that needs accepted terms. Renders nothing once accepted. */
  let { feature, onaccept }: { feature: string; onaccept?: () => void } = $props();

  const gated = $derived(consentStore.loaded && consentStore.state !== null && !isConsentGranted(consentStore.state));
</script>

{#if gated}
  <p class="consent-hint" data-testid="consent-hint">
    {feature} is off because you have not accepted the Terms and Privacy Policy.
    {#if onaccept}
      <button class="consent-hint-link ui-focus-ring" type="button" onclick={onaccept}>Review and accept</button>
    {/if}
  </p>
{/if}

<style>
  .consent-hint { margin: 6px 0 0; color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
  .consent-hint-link { background: none; border: none; padding: 0; color: var(--accent); font: inherit; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; cursor: pointer; border-radius: 3px; }
</style>
