<script lang="ts">
  import Callout from '../components/Callout.svelte';
  import { consentBlocked, requireConsent } from './consentStore.svelte';

  /**
   * Shown under a feature that needs accepted terms. Renders nothing once accepted.
   * "Review and accept" opens the accept dialog; `thenEnable` runs after Accept.
   * `banner` renders a page-level notice instead of a line under one control.
   */
  let { feature, thenEnable, banner = false }: { feature: string; thenEnable?: () => void | Promise<void>; banner?: boolean } = $props();

  const gated = $derived(consentBlocked());
  const review = () => requireConsent(thenEnable ?? (() => {}));
</script>

{#if gated && banner}
  <Callout
    class="consent-banner"
    severity="info"
    data-testid="consent-hint"
    action={{ label: 'Review and accept', onclick: review }}
  >{feature} is off because you have not accepted the Terms and Privacy Policy.</Callout>
{:else if gated}
  <p class="consent-hint" data-testid="consent-hint">
    {feature} is off because you have not accepted the Terms and Privacy Policy.
    <button class="consent-hint-link ui-focus-ring" type="button" onclick={review}>Review and accept</button>
  </p>
{/if}

<style>
  .consent-hint { margin: 6px 0 0; color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
  .consent-hint-link { background: none; border: none; padding: 0; color: var(--accent); font: inherit; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; cursor: pointer; border-radius: 3px; }
  :global(.consent-banner) { margin: 0 0 20px; }
</style>
