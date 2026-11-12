<script lang="ts">
  /** What a provider can do, in words. An icon alone did not say which task it stood for. */
  let { transcription = false, cleanup = false, iconOnly = false }: { transcription?: boolean; cleanup?: boolean; iconOnly?: boolean } = $props();
</script>

{#if transcription || cleanup}
  <span class="caps" class:icon-only={iconOnly}>
    {#if transcription}
      <span class="cap" title={iconOnly ? 'Transcription' : undefined} role={iconOnly ? 'img' : undefined} aria-label={iconOnly ? 'Transcription' : undefined}>
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><rect x="5.5" y="1.5" width="5" height="8" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3 7.5a5 5 0 0 0 10 0M8 12.5V15" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
        {#if !iconOnly}<span>Transcription</span>{/if}
      </span>
    {/if}
    {#if cleanup}
      <span class="cap" title={iconOnly ? 'Clean-up' : undefined} role={iconOnly ? 'img' : undefined} aria-label={iconOnly ? 'Clean-up' : undefined}>
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M6.5 2.5l1.1 2.9 2.9 1.1-2.9 1.1-1.1 2.9-1.1-2.9-2.9-1.1 2.9-1.1zM12 9.5l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/></svg>
        {#if !iconOnly}<span>Clean-up</span>{/if}
      </span>
    {/if}
  </span>
{/if}

<style>
  .caps { display: inline-flex; align-items: center; gap: 4px; flex: none; }
  .cap { display: inline-flex; align-items: center; gap: 5px; height: 22px; padding: 0 8px 0 7px; border-radius: 999px; background: var(--paper-2); color: var(--ink-mute); font-family: var(--sans); font-size: 11px; font-weight: 500; line-height: 1; white-space: nowrap; }
  .icon-only .cap { width: 22px; padding: 0; justify-content: center; border-radius: 7px; }
</style>
