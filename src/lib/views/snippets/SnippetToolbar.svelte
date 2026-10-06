<script lang="ts">
  import SharedListToolbar from '../SharedListToolbar.svelte';
  import { sortLabels, type SortKey } from './helpers';

  let { search = $bindable(), sort = $bindable(), onNew }: {
    search: string; sort: SortKey; onNew: () => void;
  } = $props();
</script>

<SharedListToolbar bind:search bind:sort {sortLabels} onAdd={onNew}
  placeholder="Search snippets…" searchLabel="Search snippets" actionLabel="New snippet" />
