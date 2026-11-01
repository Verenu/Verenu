<script lang="ts">
  /**
   * A value readout whose digits roll to their new position instead of jumping.
   * Characters that are not digits (a decimal point, a unit) stay put.
   */
  let {
    value,
    decimals = 0,
    suffix = '',
  }: {
    value: number;
    decimals?: number;
    suffix?: string;
  } = $props();

  const DIGITS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
  const text = $derived(`${value.toFixed(decimals)}${suffix}`);
  // Index from the right so a digit keeps its column when the number gains or loses one.
  const cells = $derived(
    Array.from(text, (char, index) => ({ char: char === ' ' ? '\u00a0' : char, key: text.length - index, digit: /\d/.test(char) ? Number(char) : null })),
  );
</script>

<span class="rolling-number">
  <span class="rolling-sr">{text}</span>
  <span class="rolling-visual" aria-hidden="true">
    {#each cells as cell (cell.key)}
      {#if cell.digit === null}
        <span class="rolling-static">{cell.char}</span>
      {:else}
        <span class="rolling-digit">
          <span class="rolling-strip" style:--digit={cell.digit}>
            {#each DIGITS as digit (digit)}<span>{digit}</span>{/each}
          </span>
        </span>
      {/if}
    {/each}
  </span>
</span>

<style>
  .rolling-number { position: relative; display: inline-flex; font-variant-numeric: tabular-nums; }

  .rolling-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }

  .rolling-visual { display: inline-flex; line-height: 1.3; }
  .rolling-static { display: inline-block; height: 1.3em; }

  .rolling-digit {
    display: inline-block;
    height: 1.3em;
    overflow: hidden;
  }

  .rolling-strip {
    display: flex;
    flex-direction: column;
    transform: translate3d(0, calc(var(--digit) * -1.3em), 0);
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }

  .rolling-strip > span { display: block; height: 1.3em; }

  @media (prefers-reduced-motion: reduce) {
    .rolling-strip { transition: none; }
  }
</style>
