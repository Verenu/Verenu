<script lang="ts">
  import type { Snippet } from 'svelte';
  import { consentBlocked, requireConsent } from './consentStore.svelte';

  /**
   * Wraps a settings area whose actions need accepted terms. While blocked, any
   * element marked `data-needs-consent` looks disabled, and tapping it opens the
   * accept dialog instead. After Accept the tapped element is clicked again, so
   * the user gets the feature they asked for. The wrapper adds no layout box.
   */
  let { children }: { children?: Snippet } = $props();
  const blocked = $derived(consentBlocked());

  let root = $state<HTMLElement | null>(null);

  // Mirror the visual dimming for assistive tech, including controls added later.
  $effect(() => {
    if (!root) return;
    const mark = () => {
      for (const el of root!.querySelectorAll<HTMLElement>('[data-needs-consent]')) {
        if (blocked) el.setAttribute('aria-disabled', 'true');
        else el.removeAttribute('aria-disabled');
      }
    };
    mark();
    const observer = new MutationObserver(mark);
    observer.observe(root, { childList: true, subtree: true });
    return () => observer.disconnect();
  });

  function intercept(event: MouseEvent) {
    if (!blocked) return;
    const target = (event.target as Element | null)?.closest<HTMLElement>('[data-needs-consent]');
    if (!target) return;
    event.preventDefault();
    event.stopPropagation();
    requireConsent(() => target.click());
  }
</script>

<div class="consent-area" bind:this={root} data-blocked={blocked || undefined} onclickcapture={intercept}>{@render children?.()}</div>

<style>
  .consent-area { display: contents; }
  :global(.consent-area[data-blocked] [data-needs-consent]) { opacity: 0.5; }
</style>
