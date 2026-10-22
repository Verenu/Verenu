<script lang="ts">
  import canonicalMark from '../../../../src-tauri/icons/verenu-mark.svg?raw';

  const mark = canonicalMark.replace('<svg ', '<svg class="verenu-mark" ');
</script>

{@html mark}

<style>
  :global(.verenu-mark) {
    display: block;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
</style>
