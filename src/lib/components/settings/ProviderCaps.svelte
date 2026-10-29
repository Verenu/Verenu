<script lang="ts">
  let { transcription = false, cleanup = false }: { transcription?: boolean; cleanup?: boolean } = $props();
</script>

{#if transcription || cleanup}
  <span class="caps">
    {#if transcription}
      <span class="cap" title="Transcription" role="img" aria-label="Transcription">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><rect x="5.5" y="1.5" width="5" height="8" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M3 7.5a5 5 0 0 0 10 0M8 12.5V15" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
      </span>
    {/if}
    {#if cleanup}
      <span class="cap" title="Cleanup" role="img" aria-label="Cleanup">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M6.5 2.5l1.1 2.9 2.9 1.1-2.9 1.1-1.1 2.9-1.1-2.9-2.9-1.1 2.9-1.1zM12 9.5l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/></svg>
      </span>
    {/if}
  </span>
{/if}

<style>
  .caps { display: inline-flex; align-items: center; gap: 4px; flex: none; }
  .cap { display: inline-grid; place-items: center; width: 22px; height: 22px; border-radius: 7px; background: var(--paper-2); color: var(--ink-mute); }
</style>
