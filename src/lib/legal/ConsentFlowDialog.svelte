<script lang="ts">
  import { modalFocusTrap } from '../modalFocus';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../motion';
  import Callout from '../components/Callout.svelte';
  import LegalLinks from './LegalLinks.svelte';
  import { acceptFromFlow, consentFlow, consentStore, dismissConsentFlow } from './consentStore.svelte';

  let acceptButton = $state<HTMLButtonElement | null>(null);
</script>

{#if consentFlow.open}
  <button class="ui-modal-backdrop consent-flow-layer" aria-label="Close dialog" tabindex="-1" onclick={dismissConsentFlow} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="ui-modal-card consent-flow-layer"
    data-testid="consent-flow"
    use:modalFocusTrap={{ active: consentFlow.open, initialFocus: () => acceptButton }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="consent-flow-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === 'Escape') dismissConsentFlow(); }}
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="ui-modal-head"><h2 id="consent-flow-title" class="ui-modal-title">Accept the Terms and Privacy Policy</h2></div>
    <div class="ui-modal-body">
      <p class="ui-modal-copy">
        This feature needs your acceptance first. If you accept, analytics and online features turn on. Dictated audio, transcripts, prompts, clipboard contents, and credentials are never collected. If you dismiss this, the feature stays off.
      </p>
      <div class="flow-links"><LegalLinks /></div>
      {#if consentStore.error}<Callout severity="error" compact>Could not save your choice. {consentStore.error}</Callout>{/if}
    </div>
    <div class="ui-modal-foot">
      <div class="ui-modal-actions">
        <button class="btn-ghost" onclick={dismissConsentFlow} disabled={consentStore.busy}>Not now</button>
        <button bind:this={acceptButton} class="btn-primary" onclick={acceptFromFlow} disabled={consentStore.busy}>
          {consentStore.busy ? 'Saving…' : 'Accept and continue'}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* Above the setup overlay (z-index 100) so it works during onboarding. */
  .consent-flow-layer { z-index: 300; }
  .ui-modal-card.consent-flow-layer { z-index: 301; }
  .flow-links { margin-top: 12px; }
</style>
