<script lang="ts">
  import { untrack } from 'svelte';
  import { tweened } from 'svelte/motion';
  import { expoOut } from 'svelte/easing';
  import { motionMs, reducedMotionEnabled } from '../../motion';

  let {
    value,
    from,
    duration = 700,
    format = (n: number) => Math.round(n).toLocaleString(),
  }: {
    value: number;
    /** Starting display value on mount, for a count-up entrance. Defaults to `value`. */
    from?: number;
    duration?: number;
    format?: (n: number) => string;
  } = $props();

  const start = untrack(() => (from === undefined || reducedMotionEnabled() ? value : from));
  const display = tweened(start, { duration: motionMs(untrack(() => duration)), easing: expoOut });

  $effect(() => {
    display.set(Number.isFinite(value) ? value : 0);
  });
</script>

<span class="animated-num">{format($display)}</span>
