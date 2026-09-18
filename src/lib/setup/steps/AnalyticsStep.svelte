<script lang="ts">
  import Toggle from '../../components/Toggle.svelte';

  let {
    enabled,
    onchange,
    error = false,
  }: {
    enabled: boolean;
    onchange: (value: boolean) => void;
    error?: boolean;
  } = $props();
</script>

<div class="step analytics-step">
  <div class="analytics-intro">
    <div class="analytics-mark" aria-hidden="true">↗</div>
    <div>
      <h3>Help improve Verenu</h3>
      <p>Verenu uses product analytics to find rough edges and make dictation more reliable.</p>
    </div>
  </div>

  <div class="analytics-card">
    <div class="analytics-row">
      <div>
        <div class="analytics-label">Share privacy-preserving product analytics</div>
        <p class="analytics-desc">Enabled by default. You can turn this off now or anytime in Settings → Privacy.</p>
      </div>
      <Toggle
        checked={enabled}
        onchange={onchange}
        label="Share privacy-preserving product analytics"
        error={error}
      />
    </div>

    <div class="analytics-details">
      <div>
        <span class="detail-title">What we collect</span>
        <span>Coarse feature usage, reliability and failure categories, settings adoption, pipeline timings, and basic app/OS metadata.</span>
      </div>
      <div>
        <span class="detail-title">What we never collect</span>
        <span>Dictated audio, transcripts, cleaned text, prompts, clipboard contents, window or app contents, URLs, accessibility text, credentials, microphone names, or raw error messages.</span>
      </div>
      <div>
        <span class="detail-title">How identity works</span>
        <span>Temporary random pseudonymous IDs are used for one app process and are not tied to an account or device ID.</span>
      </div>
    </div>
  </div>
</div>

<style>
  .analytics-step { max-width: 620px; }

  .analytics-intro { display: flex; align-items: flex-start; gap: 13px; }
  .analytics-mark {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 18px;
    font-weight: 600;
  }
  .analytics-intro h3 { margin: 0 0 4px; font-size: 17px; font-weight: 600; color: var(--ink-strong); }
  .analytics-intro p { margin: 0; color: var(--ink-mute); font-size: 13px; line-height: 1.45; }

  .analytics-card {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 17px;
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    background: var(--paper-2);
  }
  .analytics-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 18px; }
  .analytics-label { color: var(--ink-strong); font-size: 13px; font-weight: 600; line-height: 1.35; }
  .analytics-desc { margin: 4px 0 0; color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
  .analytics-row :global(.toggle) { margin-top: 2px; }

  .analytics-details { display: grid; gap: 11px; padding-top: 15px; border-top: 1px solid var(--line); color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
  .analytics-details > div { display: grid; gap: 2px; }
  .detail-title { color: var(--ink-soft); font-weight: 600; }

  @media (max-width: 520px) {
    .analytics-row { gap: 10px; }
    .analytics-card { padding: 14px; }
  }
</style>
