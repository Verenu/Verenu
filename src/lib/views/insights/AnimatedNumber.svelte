<script lang="ts">
  import { untrack } from 'svelte';
  import { tweened } from 'svelte/motion';
  import { expoOut } from 'svelte/easing';
  import { motionMs } from '../../motion';

  let {
    value,
    format = (n: number) => Math.round(n).toLocaleString(),
  }: { value: number; format?: (n: number) => string } = $props();

  const display = tweened(untrack(() => value), { duration: motionMs(700), easing: expoOut });

  $effect(() => {
    display.set(value);
  });
</script>

<span class="animated-num">{format($display)}</span>
