<script lang="ts">
  import { untrack } from 'svelte';
  import type { LazyComponent } from '../lazyComponent.svelte';

  let { view }: { view: LazyComponent } = $props();
  $effect(() => {
    const current = view;
    untrack(() => { void current.load(); });
  });
</script>

{#if view.component}
  <view.component />
{:else if view.error}
  <p role="alert">Couldn't load this view.</p>
  <button class="btn-ghost" onclick={() => location.reload()}>Reload app</button>
{:else}
  <p role="status">Loading…</p>
{/if}
