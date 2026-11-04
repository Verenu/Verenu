<script lang="ts">
  import type { Snippet } from 'svelte';
  import { consentBlocked, requireConsent } from './consentStore.svelte';

  /**
   * Wraps a settings area whose actions need accepted terms. While blocked, any
   * element marked `data-needs-consent` looks disabled (the owner also sets aria-disabled with consentBlocked()), and tapping it opens the
   * accept dialog instead. After Accept the tapped element is clicked again, so
   * the user gets the feature they asked for. The wrapper adds no layout box.
   */
  let { children }: { children?: Snippet } = $props();
  const blocked = $derived(consentBlocked());

  function intercept(event: MouseEvent) {
    if (!blocked) return;
    const target = (event.target as Element | null)?.closest<HTMLElement>('[data-needs-consent]');
    if (!target) return;
    event.preventDefault();
    event.stopPropagation();
    requireConsent(() => target.click());
  }
</script>

<div class="consent-area" data-blocked={blocked || undefined} onclickcapture={intercept}>{@render children?.()}</div>

<style>
  .consent-area { display: contents; }
  :global(.consent-area[data-blocked] [data-needs-consent]) { opacity: 0.5; }
</style>
