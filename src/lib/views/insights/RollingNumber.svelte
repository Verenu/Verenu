<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { reducedMotionEnabled } from '../../motion';
  import { ROLL_TRAVEL, RollSpring } from './rollingSpring';

  let {
    value,
    format = (n: number) => Math.round(n).toLocaleString(),
  }: { value: number; format?: (n: number) => string } = $props();

  const uid = `rn-${Math.random().toString(36).slice(2)}`;
  const text = $derived(format(value));

  /* Real motion blur is velocity x exposure, and a frame is the exposure. The
     blur is vertical-only (stdDeviation "0 n"): an isotropic blur smears a
     digit sideways into its neighbours and reads as glow rather than movement,
     which is the whole reason this is an SVG filter and not filter: blur(). */
  const BLUR_PER_PX_PER_S = 0.022;
  const MAX_BLUR = 7;

  let view = $state<string[]>([]);

  let cellEls: (HTMLSpanElement | null)[] = [];
  let liveEls: (HTMLSpanElement | null)[] = [];
  let ghostEls: (HTMLSpanElement | null)[] = [];
  let blurEls: (SVGFEGaussianBlurElement | null)[] = [];

  // Deliberately not $state: these change every frame and drive the DOM
  // directly, so routing them through reactivity would only add churn.
  let chars: string[] = [];
  // Seeded from the mount value only; every later value arrives via the
  // effect below, which is the point — silence the reactivity lint.
  let prevValue = untrack(() => value);
  let fontPx = 16;
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
    live.style.transform = `translateY(${(t * travel).toFixed(4)}em)`;
    live.style.opacity = `${clamp(1 - Math.abs(t) * 1.35, 0, 1)}`;
    ghost.style.transform = `translateY(${((t - 1) * travel).toFixed(4)}em)`;
    ghost.style.opacity = `${clamp(Math.abs(t) * 1.5 - 0.15, 0, 1)}`;
    const blur = blurEls[i];
    if (blur) {
      const speedPx = Math.abs(spring.vel) * ROLL_TRAVEL * fontPx;
      blur.setAttribute(
        'stdDeviation',
        `0 ${Math.min(MAX_BLUR, speedPx * BLUR_PER_PX_PER_S).toFixed(2)}`
      );
    }
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
    // Dropping the filter when idle keeps settled text on normal subpixel
    // antialiasing; a live filter forces every digit onto its own layer.
    if (cell) cell.style.filter = '';
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
    // Compare by place value, not by string index, so 842 -> 1,749 rolls the
    // ones digit into the ones digit rather than shifting every column along.
    const shift = next.length - chars.length;
    const rollDir: 1 | -1 = value >= prevValue ? 1 : -1;
    prevValue = value;
    if (shift !== 0) {
      for (let i = 0; i < springs.length; i++) settle(i);
      springs.length = 0;
    }
    const firstCell = cellEls[0];
    if (firstCell) fontPx = parseFloat(getComputedStyle(firstCell).fontSize) || 16;

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
      cell.style.filter = `url(#${uid}-${i})`;
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

<!-- The filter defs are a sibling, and the cells sit on one line with no gaps
     between them: any whitespace in here becomes a text node, which would make
     the readout copy and read aloud as "1 , 7 4 9" instead of "1,749". -->
<svg class="rn-defs" aria-hidden="true" focusable="false">
  <defs>
    {#each view as _, i (i)}
      <!-- The region must be generous: anything the blur or the travel pushes
           outside it is clipped, which is what draws a hard box round it. -->
      <filter
        id="{uid}-{i}"
        x="-80%"
        y="-200%"
        width="260%"
        height="500%"
        color-interpolation-filters="sRGB"
      >
        <feGaussianBlur bind:this={blurEls[i]} in="SourceGraphic" stdDeviation="0 0" />
      </filter>
    {/each}
  </defs>
</svg>
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

  /* Nothing clips and nothing forces a line-height: the settled digit is plain
     in-flow text, so a digit that just rolled sits on exactly the same baseline
     as one that never moved. (inline-block + overflow:hidden does not — it
     aligns by its bottom margin edge, which is what made them settle uneven.) */
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

  .rn-defs {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
  }
</style>
