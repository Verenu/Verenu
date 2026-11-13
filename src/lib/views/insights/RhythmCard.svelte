<script lang="ts">
  import { fmtHour, fmtNumber } from './helpers';
  import ChartTooltip from './ChartTooltip.svelte';
  import { activeDayCount, peakWeekday, weekdayStats, weekendShare, fmtPercent, WEEKDAY_LONG, WEEKDAY_ORDER, WEEKDAY_SHORT } from './analytics';
  import type { InsightsDay } from './types';

  let { hourly = [], daily = [] }: { hourly?: number[]; daily?: InsightsDay[] } = $props();

  const hours = $derived(
    Array.from({ length: 24 }, (_, i) => {
      const n = Array.isArray(hourly) ? Number(hourly[i]) : 0;
      return Number.isFinite(n) && n > 0 ? n : 0;
    }),
  );
  const hourMax = $derived(Math.max(0, ...hours));
  const hourTotal = $derived(hours.reduce((sum, n) => sum + n, 0));
  const peakHour = $derived(hourMax > 0 ? hours.indexOf(hourMax) : -1);

  const weekdays = $derived(weekdayStats(daily));
  const ordered = $derived(WEEKDAY_ORDER.map((d) => weekdays[d]));
  const weekdayMax = $derived(Math.max(0, ...weekdays.map((w) => w.average)));
  const peakDay = $derived(peakWeekday(weekdays));
  const weekend = $derived(weekendShare(weekdays));

  // A pattern from one or two days is noise; hold back the labels until there is a week's worth of signal.
  const enoughDays = $derived(activeDayCount(daily) >= 3);

  /** Morning 5–12, afternoon 12–17, evening 17–22, night otherwise. */
  const dayPart = $derived.by(() => {
    if (peakHour < 0 || !enoughDays) return null;
    if (peakHour >= 5 && peakHour < 12) return 'a morning talker';
    if (peakHour >= 12 && peakHour < 17) return 'an afternoon talker';
    if (peakHour >= 17 && peakHour < 22) return 'an evening talker';
    return 'a night owl';
  });

  let hoverHour = $state<number | null>(null);
  let hoverDay = $state<number | null>(null);
  let tip = $state<{ x: number; y: number; host: 'hour' | 'day' } | null>(null);

  function place(event: PointerEvent, host: 'hour' | 'day') {
    const col = event.currentTarget as HTMLElement;
    const chart = col.closest('.rhythm-chart') as HTMLElement | null;
    const bar = col.querySelector('.bar') as HTMLElement | null;
    if (!chart || !bar) return;
    const chartRect = chart.getBoundingClientRect();
    const barRect = bar.getBoundingClientRect();
    tip = { x: barRect.left - chartRect.left + barRect.width / 2, y: barRect.top - chartRect.top, host };
  }

  const scaled = (value: number, max: number) => (max > 0 ? Math.max(0.02, value / max) : 0.02);
</script>

<section class="card rhythm" aria-labelledby="rhythm-h">
  <header class="card-head">
    <div>
      <h2 class="card-h" id="rhythm-h">Your rhythm</h2>
      <p class="card-sub">When in the day and week you dictate</p>
    </div>
  </header>

  <div class="rhythm-grid">
    <div class="rhythm-block">
      <p class="block-label">By hour</p>
      <div
        class="rhythm-chart hours"
        role="img"
        aria-label={peakHour >= 0
          ? `Words by hour of day. Peak at ${fmtHour(peakHour)} with ${fmtNumber(hourMax)} words.`
          : 'No hourly activity recorded yet.'}
      >
        {#each hours as words, hour}
          <div
            class="col"
            role="presentation"
            onpointerenter={(e) => { hoverHour = hour; place(e, 'hour'); }}
            onpointerleave={() => { hoverHour = null; tip = null; }}
          >
            <span
              class="bar"
              class:peak={hour === peakHour}
              class:hovered={hoverHour === hour}
              class:dim={hoverHour !== null && hoverHour !== hour}
              style:transform="scaleY({scaled(words, hourMax)})"
              style:--i={hour}
            ></span>
          </div>
        {/each}
        {#if tip?.host === 'hour' && hoverHour !== null}
          <ChartTooltip x={tip.x} y={tip.y} visible={true}>
            <strong>{fmtNumber(hours[hoverHour])}</strong> words
            <div class="tooltip-dim">{fmtHour(hoverHour)}–{fmtHour((hoverHour + 1) % 24)}</div>
          </ChartTooltip>
        {/if}
      </div>
      <div class="axis" aria-hidden="true">
        <span>12a</span><span>6a</span><span>12p</span><span>6p</span><span>11p</span>
      </div>
      <p class="foot">
        {#if peakHour >= 0}
          Busiest around <strong>{fmtHour(peakHour)}</strong>, with {fmtPercent(hourMax / Math.max(1, hourTotal))} of your words.{#if dayPart}{' '}You're {dayPart}.{/if}
        {:else}
          Hourly patterns appear once you've dictated a few times.
        {/if}
      </p>
    </div>

    <div class="rhythm-block">
      <p class="block-label">By weekday <span class="block-hint">average words</span></p>
      <div
        class="rhythm-chart days"
        role="img"
        aria-label={peakDay
          ? `Average words by weekday. ${WEEKDAY_LONG[peakDay.weekday]} is busiest with ${fmtNumber(peakDay.average)} words on average.`
          : 'No weekday activity recorded yet.'}
      >
        {#each ordered as slot, i (slot.weekday)}
          <div
            class="col"
            role="presentation"
            onpointerenter={(e) => { hoverDay = slot.weekday; place(e, 'day'); }}
            onpointerleave={() => { hoverDay = null; tip = null; }}
          >
            <span
              class="bar"
              class:peak={peakDay?.weekday === slot.weekday}
              class:weekend={slot.weekday === 0 || slot.weekday === 6}
              class:hovered={hoverDay === slot.weekday}
              class:dim={hoverDay !== null && hoverDay !== slot.weekday}
              style:transform="scaleY({scaled(slot.average, weekdayMax)})"
              style:--i={i * 2}
            ></span>
          </div>
        {/each}
        {#if tip?.host === 'day' && hoverDay !== null}
          <ChartTooltip x={tip.x} y={tip.y} visible={true}>
            <strong>{fmtNumber(weekdays[hoverDay].average)}</strong> words on average
            <div class="tooltip-dim">{WEEKDAY_LONG[hoverDay]} · {weekdays[hoverDay].days} in range</div>
          </ChartTooltip>
        {/if}
      </div>
      <div class="axis days-axis" aria-hidden="true">
        {#each ordered as slot (slot.weekday)}<span>{WEEKDAY_SHORT[slot.weekday].slice(0, 1)}</span>{/each}
      </div>
      <p class="foot">
        {#if peakDay && enoughDays}
          <strong>{WEEKDAY_LONG[peakDay.weekday]}s</strong> are busiest.{#if weekend !== null}{' '}Weekends carry {fmtPercent(weekend)} of your words.{/if}
        {:else}
          Weekday patterns appear after a few active days.
        {/if}
      </p>
    </div>
  </div>
</section>

<style>
  .rhythm-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.65fr) minmax(0, 1fr);
    gap: 28px;
  }

  .rhythm-block { min-width: 0; display: flex; flex-direction: column; }

  .block-label {
    margin: 0 0 10px;
    font-size: 11px;
    font-weight: 500;
    color: var(--ink-soft);
  }
  .block-hint { color: var(--ink-faint); font-weight: 400; margin-left: 4px; }

  .rhythm-chart {
    position: relative;
    display: grid;
    align-items: end;
    height: 108px;
    gap: 3px;
  }
  .rhythm-chart.hours { grid-template-columns: repeat(24, minmax(0, 1fr)); }
  .rhythm-chart.days { grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 8px; }

  .col {
    position: relative;
    height: 100%;
    min-width: 0;
    display: flex;
    align-items: flex-end;
  }

  .bar {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 34%, var(--paper-3));
    transform-origin: bottom;
    transition:
      transform 520ms var(--ui-ease-out),
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      opacity var(--ui-duration-fast) var(--ui-ease-out);
    animation: bar-grow 640ms var(--ui-ease-out) both;
    animation-delay: calc(var(--i, 0) * 18ms + 120ms);
  }
  .days .bar { border-radius: 5px; }
  .bar.weekend { background: color-mix(in srgb, var(--accent) 22%, var(--paper-3)); }
  .bar.peak,
  .bar.hovered { background: var(--accent); }
  .bar.dim { opacity: 0.55; }

  @keyframes bar-grow {
    from { scale: 1 0; }
    to { scale: 1 1; }
  }

  .axis {
    display: flex;
    justify-content: space-between;
    margin-top: 7px;
    font-size: 10px;
    color: var(--ink-mute);
    font-variant-numeric: tabular-nums;
  }
  .days-axis {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 8px;
    text-align: center;
  }

  .foot {
    margin: 12px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--ink-mute);
  }
  .foot strong { color: var(--ink); font-weight: 500; }

  @container insights (max-width: 620px) {
    .rhythm-grid { grid-template-columns: minmax(0, 1fr); gap: 22px; }
  }

  @container insights (max-width: 420px) {
    .rhythm-chart.hours { gap: 2px; }
    .bar { border-radius: 2px; }
  }

  @media (prefers-reduced-motion: reduce) {
    .bar { animation: none; transition-duration: 1ms; }
  }
</style>
