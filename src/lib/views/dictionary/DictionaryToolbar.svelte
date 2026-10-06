<script lang="ts">
  import SharedListToolbar from '../SharedListToolbar.svelte';
  import { sortLabels, type SortKey } from './helpers';

  let { search = $bindable(), sort = $bindable(), count, onAdd }: {
    search: string; sort: SortKey; count: number; onAdd: () => void;
  } = $props();
</script>

<SharedListToolbar bind:search bind:sort {sortLabels} {onAdd}
  placeholder={`Search ${count} ${count === 1 ? 'term' : 'terms'}…`}
  searchLabel="Search dictionary" actionLabel="Add term" />
