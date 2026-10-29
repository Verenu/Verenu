<script lang="ts">
  import type { CleanupIntensity, ToneId } from '../../settings';
  import { fade } from 'svelte/transition';
  import { motionMs } from '../../motion';
  import { cleanupCards, toneCards, writingStylePreview } from '../setupData';

  let {
    intensity = $bindable(),
    tone = $bindable(),
    onIntensitySelect,
    cleanupModelMissing,
    onReviewModels,
  }: { intensity: CleanupIntensity; tone: ToneId; onIntensitySelect?: () => void; cleanupModelMissing: boolean; onReviewModels: () => void } = $props();

  const cleanupOff = $derived(intensity === 'none');
  const toneInactive = $derived(cleanupOff || intensity === 'rules');
  const preview = $derived(writingStylePreview(intensity, tone));
</script>

<div class="step writing-style-step">
  <div class="style-group">
    <p class="group-label">Cleanup intensity</p>
    {#if intensity === 'rules'}<p class="group-note">Your previous Basic setting is still active. Choose a level to replace it; Basic cannot be selected again.</p>{/if}
    {#if cleanupModelMissing && intensity !== 'none' && intensity !== 'rules'}
      <p class="group-note" role="status">Cleanup is off with your transcription-only model choice. <button type="button" class="cleanup-model-link" onclick={onReviewModels}>Choose a preset with a cleanup model</button> to use this level.</p>
    {/if}
    <div class="style-grid cleanup-grid">
      {#each cleanupCards as c}
        <button
          class="pick-card style-card"
          class:selected={intensity === c.id}
          aria-pressed={intensity === c.id}
          onclick={() => { intensity = c.id; onIntensitySelect?.(); }}
        >
          <div class="card-top">
            <span class="card-name">{c.name}</span>
            <div class="pick-radio" class:checked={intensity === c.id}></div>
          </div>
          <p class="card-desc">{c.desc}</p>
        </button>
      {/each}
    </div>
  </div>

  <div class="style-group" class:disabled={toneInactive}>
    <div class="group-head">
      <p class="group-label">Tone</p>
      {#if toneInactive}
        <span class="group-note">{cleanupOff ? 'Off skips cleanup' : 'Basic skips AI tone'}</span>
      {/if}
    </div>
    <div class="style-grid tone-grid" inert={toneInactive}>
      {#each toneCards as t}
        <button
          class="pick-card style-card"
          class:selected={tone === t.id && !toneInactive}
          aria-pressed={tone === t.id && !toneInactive}
          onclick={() => { tone = t.id; }}
        >
          <div class="card-top">
            <span class="card-name">{t.name}</span>
            <div class="pick-radio" class:checked={tone === t.id && !toneInactive}></div>
          </div>
          <p class="card-desc">{t.desc}</p>
        </button>
      {/each}
    </div>
  </div>

  <div class="preview-box">
    <div class="preview-stack">
      {#key cleanupOff ? 'off' : `${intensity}-${tone}`}
        <div class="preview-slot" in:fade={{ duration: motionMs(180), delay: motionMs(70) }} out:fade={{ duration: motionMs(70) }}>
          {#if cleanupOff}
            <p class="preview-off">Cleanup is off — whatever you say is injected exactly as transcribed.</p>
          {:else}
            <div class="preview-row">
              <span class="preview-sample"><span class="preview-label">Before</span><span class="preview-before">"{preview.before}"</span></span>
              <span class="preview-arrow" aria-hidden="true">→</span>
              <span class="preview-sample"><span class="preview-label">After</span><span class="preview-after">"{preview.after}"</span></span>
            </div>
          {/if}
        </div>
      {/key}
    </div>
  </div>
</div>

<style>
  .writing-style-step { gap: 16px; }

  .style-group { display: flex; flex-direction: column; gap: 8px; }

  .style-group { transition: opacity 0.22s ease; }
  .style-group.disabled { opacity: 0.4; }

  .group-head { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; }

  .group-note { font-size: 11px; color: var(--ink-faint); font-style: italic; }
  .cleanup-model-link { color: var(--ink-strong); text-decoration: underline; cursor: pointer; }

  .style-grid {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 8px;
  }

  .style-grid > :global(*) { grid-column: span 3; }
  .tone-grid > :global(*) { grid-column: span 4; }

  .cleanup-grid > :global(*) { grid-column: span 6; }

  .style-card { min-height: 72px; justify-content: flex-start; }

  .card-top { display: flex; justify-content: space-between; align-items: center; gap: 6px; }
  .card-name { font-size: 12.5px; font-weight: 500; color: var(--ink-strong); }
  .card-desc { font-size: 11px; color: var(--ink-mute); margin: 0; line-height: 1.35; }

  .preview-box {
    background: var(--paper-2);
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    padding: 11px 14px;
    display: flex;
    align-items: stretch;
    min-height: 62px;
    box-sizing: border-box;
  }

  .preview-stack { flex: 1; min-width: 0; display: grid; }
  .preview-stack > :global(*) { grid-column: 1; grid-row: 1; align-self: center; }

  .preview-row { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); align-items: center; gap: 12px; min-width: 0; }

  .preview-sample { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .preview-label { font-size: 10px; font-weight: 500; text-transform: none; letter-spacing: 0; color: var(--ink-faint); }

  .preview-before { font-size: 12px; font-style: italic; color: var(--ink-mute); line-height: 1.4; }

  .preview-arrow { font-size: 12px; color: var(--ink-faint); flex-shrink: 0; }

  .preview-after { font-size: 12.5px; font-weight: 500; color: var(--accent-ink); line-height: 1.4; }

  .preview-off { margin: 0; font-size: 12.5px; color: var(--ink-mute); line-height: 1.4; }

  @media (max-width: 720px) {
    .style-grid > :global(*) { grid-column: span 6; }
    .tone-grid > :global(*) { grid-column: span 6; }
    .preview-row { grid-template-columns: 1fr; gap: 5px; }
    .preview-arrow { display: none; }
  }

  @media (max-height: 660px) {
    .writing-style-step { gap: 12px; }
    .style-group { gap: 6px; }
    .writing-style-step .style-grid { gap: 6px; }
    .writing-style-step .style-card { min-height: 0; padding: 8px 10px; gap: 2px; }
    .writing-style-step .card-desc { line-height: 1.3; }
    .writing-style-step .preview-box { min-height: 52px; padding: 8px 12px; }
  }
</style>
