<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { reducedMotionEnabled } from '../../motion';
  import { ROLL_TRAVEL, RollSpring } from './rollingSpring';

  let {
    value,
    format = (n: number) => Math.round(n).toLocaleString(),
  }: { value: number; format?: (n: number) => string } = $props();

  const text = $derived(format(value));

  const STRETCH_PER_SPEED = 0.012;
  const MAX_STRETCH = 0.22;

  let view = $state<string[]>([]);

  let cellEls: (HTMLSpanElement | null)[] = [];
  let liveEls: (HTMLSpanElement | null)[] = [];
  let ghostEls: (HTMLSpanElement | null)[] = [];

  let chars: string[] = [];
  let prevValue = untrack(() => value);
  const springs: RollSpring[] = [];

  let raf = 0;
  let lastFrame = 0;
  let destroyed = false;

  const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

  function paint(i: number) {
    const live = liveEls[i];
    const ghost = ghostEls[i];
    if (!live || !ghost) return;
    const spring = springs[i];
    if (!spring) return;
    const t = spring.pos;
    const travel = ROLL_TRAVEL * spring.dir;
    const stretch = 1 + Math.min(MAX_STRETCH, Math.abs(spring.vel) * STRETCH_PER_SPEED);
    live.style.transform = `translateY(${(t * travel).toFixed(4)}em) scaleY(${stretch.toFixed(3)})`;
    live.style.opacity = `${clamp(1 - Math.abs(t) * 1.35, 0, 1)}`;
    ghost.style.transform = `translateY(${((t - 1) * travel).toFixed(4)}em) scaleY(${stretch.toFixed(3)})`;
    ghost.style.opacity = `${clamp(Math.abs(t) * 1.5 - 0.15, 0, 1)}`;
  }

  function settle(i: number) {
    springs[i]?.reset();
    const live = liveEls[i];
    const ghost = ghostEls[i];
    const cell = cellEls[i];
    if (live) live.removeAttribute('style');
    if (ghost) {
      ghost.textContent = '';
      ghost.removeAttribute('style');
    }
    if (cell) cell.classList.remove('rolling');
  }

  function frame(now: number) {
    const dt = lastFrame ? Math.min(0.032, (now - lastFrame) / 1000) : 1 / 60;
    lastFrame = now;
    let active = false;
    for (let i = 0; i < springs.length; i++) {
      const spring = springs[i];
      if (!spring?.active) continue;
      if (!spring.step(dt)) {
        settle(i);
        continue;
      }
      paint(i);
      active = true;
    }
    raf = active ? requestAnimationFrame(frame) : 0;
    if (!active) lastFrame = 0;
  }

  function applyChange(next: string[]) {
    const reduced = reducedMotionEnabled();
    const shift = next.length - chars.length;
    const rollDir: 1 | -1 = value >= prevValue ? 1 : -1;
    prevValue = value;
    if (shift !== 0) {
      for (let i = 0; i < springs.length; i++) settle(i);
      springs.length = 0;
    }
    let started = false;
    for (let i = 0; i < next.length; i++) {
      const was = chars[i - shift];
      if (reduced || was === undefined || was === next[i]) continue;
      const live = liveEls[i];
      const ghost = ghostEls[i];
      const cell = cellEls[i];
      if (!live || !ghost || !cell) continue;
      ghost.textContent = was;
      (springs[i] ??= new RollSpring()).start(rollDir);
      cell.classList.add('rolling');
      paint(i);
      started = true;
    }
    chars = next;
    if (started && raf === 0) {
      lastFrame = 0;
      raf = requestAnimationFrame(frame);
    }
  }

  $effect(() => {
    const next = text.split('');
    view = next;
    // tick() lands after Svelte has written the new digits but before the
    // browser paints, so a roll's opening frame is applied in the same paint as
    // the character swap — otherwise the new digit flashes in place first.
    tick().then(() => {
      if (!destroyed) applyChange(next);
    });
  });

  onDestroy(() => {
    destroyed = true;
    if (raf) cancelAnimationFrame(raf);
  });
</script>

<span class="rolling-number"
  >{#each view as ch, i (i)}<span class="cell" bind:this={cellEls[i]}><span
      class="live"
      bind:this={liveEls[i]}>{ch}</span><span
      class="ghost"
      bind:this={ghostEls[i]}
      aria-hidden="true"
    ></span></span>{/each}</span
>

<style>
  .rolling-number {
    display: inline-flex;
    align-items: baseline;
    font-variant-numeric: tabular-nums;
  }

  .cell {
    position: relative;
    display: inline-block;
  }

  .live {
    display: inline-block;
  }

  .ghost {
    position: absolute;
    inset: 0;
    opacity: 0;
    pointer-events: none;
  }

  .cell:global(.rolling) .live,
  .cell:global(.rolling) .ghost {
    will-change: transform, opacity;
  }
</style>
