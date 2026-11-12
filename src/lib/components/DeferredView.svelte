<script lang="ts">
  import { untrack } from 'svelte';
  import type { LazyComponent } from '../lazyComponent.svelte';
  import Callout from './Callout.svelte';

  let { view }: { view: LazyComponent } = $props();
  $effect(() => {
    const current = view;
    untrack(() => { void current.load(); });
  });
</script>

{#if view.component}
  <view.component />
{:else if view.error}
  <div class="deferred-error">
    <Callout severity="error" action={{ label: 'Reload app', onclick: () => location.reload() }}>Couldn't load this view.</Callout>
  </div>
{:else}
  <!-- Views load in a few frames. Announce it without putting a line of text into the layout. -->
  <p class="deferred-loading" role="status">Loading…</p>
{/if}

<style>
  .deferred-error {
    width: min(100%, 480px);
    margin: 0 auto;
    padding: var(--page-pad-y) var(--page-pad-x);
  }

  .deferred-loading {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
