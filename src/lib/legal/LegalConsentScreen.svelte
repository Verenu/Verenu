<script lang="ts">
  import SetupShell from '../setup/SetupShell.svelte';
  import Callout from '../components/Callout.svelte';
  import LegalLinks from './LegalLinks.svelte';
  import { acceptConsent, consentStore, skipConsent } from './consentStore.svelte';
  import { currentVersions } from './consentStore.svelte';

  /** `first` is the onboarding step. `update` is the re-prompt after a version change. */
  let { mode = 'first' }: { mode?: 'first' | 'update' } = $props();

  const update = $derived(mode === 'update');
  const versions = $derived(currentVersions());
</script>

<SetupShell step={0} totalSteps={0} onDotClick={() => {}}>
  {#snippet left()}
    <button class="btn-skip ui-focus-ring" onclick={skipConsent} disabled={consentStore.busy}>Skip for now</button>
  {/snippet}
  {#snippet right()}
    <button class="btn-primary btn-lg ui-focus-ring" onclick={acceptConsent} disabled={consentStore.busy}>
      {consentStore.busy ? 'Saving…' : 'Accept and continue'}
    </button>
  {/snippet}

  <div class="step legal-step" data-testid="legal-consent-screen" data-mode={mode}>
    <div class="legal-intro">
      <h2 class="legal-title" tabindex="-1">
        {update ? 'Our terms and privacy policy changed' : 'Before you start'}
      </h2>
      <p class="legal-lede">
        {update
          ? `We updated the Terms of Service and Privacy Policy (Terms ${versions.termsVersion}, Privacy ${versions.privacyVersion}). Please review them and choose whether to accept.`
          : 'Review the Terms of Service and Privacy Policy, then choose whether to accept.'}
      </p>
    </div>

    <div class="legal-card">
      <div class="legal-block">
        <span class="legal-heading">If you accept</span>
        <span>Verenu records your acceptance of the Terms (version {versions.termsVersion}) and Privacy Policy (version {versions.privacyVersion}), tied to a random install ID. Product analytics and online features turn on.</span>
      </div>
      <div class="legal-block">
        <span class="legal-heading">What we never collect</span>
        <span>Dictated audio, transcripts, cleaned text, prompts, clipboard contents, or credentials.</span>
      </div>
      <div class="legal-block">
        <span class="legal-heading">If you skip</span>
        <span>Dictation still works. Analytics and online features stay off. You can turn them on later in Settings → Privacy.</span>
      </div>
      <LegalLinks />
    </div>

    {#if consentStore.notice}<Callout severity="warning" compact data-testid="consent-notice">{consentStore.notice}</Callout>{/if}
    {#if consentStore.error}
      <Callout severity="error" compact>Could not save your choice. {consentStore.error}</Callout>
    {/if}
  </div>
</SetupShell>

<style>
  .legal-step { max-width: 620px; display: flex; flex-direction: column; gap: 18px; }
  .legal-title { margin: 0 0 4px; font-size: 20px; font-weight: 600; color: var(--ink-strong); outline: none; }
  .legal-lede { margin: 0; color: var(--ink-mute); font-size: 13px; line-height: 1.45; }
  .legal-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 17px;
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    background: var(--paper-2);
    color: var(--ink-mute);
    font-size: 12px;
    line-height: 1.45;
  }
  .legal-block { display: grid; gap: 2px; }
  .legal-heading { color: var(--ink-soft); font-weight: 600; }
  @media (max-width: 520px) { .legal-card { padding: 14px; } }
</style>
