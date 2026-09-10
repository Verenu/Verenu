<script lang="ts">
  import { untrack } from 'svelte';
  import { tweened } from 'svelte/motion';
  import { cubicOut } from 'svelte/easing';
  import { motionMs, reducedMotionEnabled } from '../../motion';

  let { value }: { value: number } = $props();

  const DIGITS = Array.from({ length: 10 }, (_, index) => index);
  const duration = motionMs(500);
  const normalize = (next: number) => Math.max(0, Math.round(Number.isFinite(next) ? next : 0));
  const initialValue = reducedMotionEnabled() ? normalize(untrack(() => value)) : 0;
  const display = tweened(initialValue, { duration, easing: cubicOut });
  let previousValue = initialValue;
  let animating = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const next = normalize(value);
    if (next === previousValue) return;
    previousValue = next;
    animating = true;
    if (timer) clearTimeout(timer);
    display.set(next);
    timer = setTimeout(() => {
      animating = false;
      timer = undefined;
    }, duration);
    return () => {
      if (timer) clearTimeout(timer);
      timer = undefined;
    };
  });

  const formatted = $derived(normalize($display).toLocaleString());
  const accessibleValue = $derived(normalize(value).toLocaleString());
</script>

<span class="rolling-number" class:animating aria-hidden="true">
  {#each formatted.split('') as character, index (index)}
    {#if /\d/.test(character)}
      <span class="digit-window">
        <span class="digit-track" style:transform={`translateY(-${Number(character) * 10}%)`}>
          {#each DIGITS as digit}
            <span class="digit-face" data-digit={digit}></span>
          {/each}
        </span>
      </span>
    {:else}
      <span class="separator">{character}</span>
    {/if}
  {/each}
</span>
<span class="sr-only">{accessibleValue}</span>

<style>
  .rolling-number {
    display: inline-flex;
    align-items: baseline;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .digit-window {
    display: inline-block;
    height: 1em;
    overflow: hidden;
    position: relative;
    vertical-align: bottom;
    width: 0.62em;
    mask-image: linear-gradient(to bottom, transparent 0%, black 14%, black 86%, transparent 100%);
    -webkit-mask-image: linear-gradient(to bottom, transparent 0%, black 14%, black 86%, transparent 100%);
  }

  .digit-track {
    display: block;
    height: 1000%;
    will-change: transform, filter;
  }

  .digit-face {
    align-items: center;
    display: flex;
    height: 10%;
    justify-content: center;
  }

  .digit-face::before {
    content: attr(data-digit);
  }

  .separator {
    display: inline-block;
    width: 0.25em;
  }

  .rolling-number.animating .digit-track {
    filter: blur(0.8px);
  }

  .rolling-number.animating .digit-window {
    filter: blur(0.18px);
  }

  @media (prefers-reduced-motion: reduce) {
    .digit-track {
      will-change: auto;
    }

    .rolling-number.animating .digit-track {
      filter: none;
    }

    .rolling-number.animating .digit-window {
      filter: none;
    }
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
