<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import { consentBlocked, requireConsent } from './consentStore.svelte';

  /**
   * A button for a feature that needs accepted terms. While blocked it looks and
   * reads as disabled but still takes a tap, which opens the accept dialog and
   * runs `onclick` only after Accept.
   */
  let {
    onclick,
    class: className = '',
    children,
    ...rest
  }: { onclick: () => void | Promise<void>; class?: string; children?: Snippet } & Omit<HTMLButtonAttributes, 'onclick'> = $props();

  const blocked = $derived(consentBlocked());
</script>

<button
  type="button"
  {...rest}
  class="{className} ui-focus-ring"
  class:consent-gated={blocked}
  aria-disabled={blocked || undefined}
  onclick={() => requireConsent(onclick)}
>{@render children?.()}</button>

<style>
  .consent-gated { opacity: 0.5; }
</style>
