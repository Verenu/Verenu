<script lang="ts">
  import { privacyUrl, termsUrl } from './consent';
  import { currentVersions } from './consentStore.svelte';
  import { openLegalLink } from './openLegalLink';

  const versions = $derived(currentVersions());
  const termsHref = $derived(termsUrl(versions.termsVersion));
  const privacyHref = $derived(privacyUrl(versions.privacyVersion));

  function open(event: MouseEvent, url: string) {
    event.preventDefault();
    void openLegalLink(url);
  }
</script>

<ul class="legal-links">
  <li>
    <a class="legal-link ui-focus-ring" href={termsHref} onclick={(e) => open(e, termsHref)} target="_blank" rel="noopener noreferrer">Terms of Service</a>
    <span class="legal-version">version {versions.termsVersion}</span>
  </li>
  <li>
    <a class="legal-link ui-focus-ring" href={privacyHref} onclick={(e) => open(e, privacyHref)} target="_blank" rel="noopener noreferrer">Privacy Policy</a>
    <span class="legal-version">version {versions.privacyVersion}</span>
  </li>
</ul>

<style>
  .legal-links { display: flex; flex-wrap: wrap; gap: 8px 22px; margin: 0; padding: 0; list-style: none; }
  .legal-links li { display: flex; align-items: baseline; gap: 8px; }
  .legal-link { color: var(--accent); font-size: 13px; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; border-radius: 3px; }
  .legal-version { color: var(--ink-mute); font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
