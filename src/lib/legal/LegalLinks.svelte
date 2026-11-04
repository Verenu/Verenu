<script lang="ts">
  import { PRIVACY_URL, PRIVACY_VERSION, TERMS_URL, TERMS_VERSION } from './consent';
  import { openLegalLink } from './openLegalLink';

  function open(event: MouseEvent, url: string) {
    event.preventDefault();
    void openLegalLink(url);
  }
</script>

<ul class="legal-links">
  <li>
    <a class="legal-link ui-focus-ring" href={TERMS_URL} onclick={(e) => open(e, TERMS_URL)} target="_blank" rel="noopener noreferrer">Terms of Service</a>
    <span class="legal-version">version {TERMS_VERSION}</span>
  </li>
  <li>
    <a class="legal-link ui-focus-ring" href={PRIVACY_URL} onclick={(e) => open(e, PRIVACY_URL)} target="_blank" rel="noopener noreferrer">Privacy Policy</a>
    <span class="legal-version">version {PRIVACY_VERSION}</span>
  </li>
</ul>

<style>
  .legal-links { display: flex; flex-wrap: wrap; gap: 8px 22px; margin: 0; padding: 0; list-style: none; }
  .legal-links li { display: flex; align-items: baseline; gap: 8px; }
  .legal-link { color: var(--accent); font-size: 13px; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; border-radius: 3px; }
  .legal-version { color: var(--ink-mute); font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
