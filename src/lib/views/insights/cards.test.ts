import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import RhythmCard from './RhythmCard.svelte';
import KpiTiles from './KpiTiles.svelte';
import ContextBreakdown from './ContextBreakdown.svelte';
import CleanupCard from './CleanupCard.svelte';
import { EMPTY_INSIGHTS, type InsightsPayload } from './types';

const day = (d: string, words: number) => ({ day: d, words, transcriptions: words ? 1 : 0, speaking_ms: words * 400 });
// 2026-11-02 is a Monday.
const week = ['2026-11-02', '2026-11-03', '2026-11-04', '2026-11-05', '2026-11-06', '2026-11-07', '2026-11-08'];

describe('RhythmCard', () => {
  it('withholds personality labels when only one day has activity', () => {
    const hourly = new Array(24).fill(0);
    hourly[9] = 12;
    const { body } = render(RhythmCard, { props: { hourly, daily: week.map((d, i) => day(d, i === 6 ? 12 : 0)) } });
    expect(body).toContain('Busiest around');
    expect(body).not.toContain('talker');
    expect(body).not.toContain('are busiest');
  });

  it('names the busiest weekday and time of day with enough history', () => {
    const hourly = new Array(24).fill(0);
    hourly[21] = 500;
    const { body } = render(RhythmCard, { props: { hourly, daily: week.map((d, i) => day(d, [100, 400, 50, 50, 50, 0, 0][i])) } });
    expect(body).toContain('an evening talker');
    expect(body).toContain('Tuesdays</strong> are busiest');
    expect(body).toContain('Weekends carry 0%');
  });

  it('survives a short or malformed hourly array', () => {
    const { body } = render(RhythmCard, { props: { hourly: [Number.NaN, -5], daily: [] } });
    expect(body).toContain('Hourly patterns appear');
    expect((body.match(/class="col[\s"]/g) ?? []).length).toBe(31);
  });
});

describe('KpiTiles', () => {
  function payload(patch: Partial<InsightsPayload['totals']>): InsightsPayload {
    return { ...EMPTY_INSIGHTS, daily: week.map((d) => day(d, 2)), totals: { ...EMPTY_INSIGHTS.totals, ...patch } };
  }

  it('does not round a sliver of a book up to 1%', () => {
    const { body } = render(KpiTiles, { props: { data: payload({ words_in_range: 12, total_words: 12 }), rangeDays: 7 } });
    expect(body).toContain('Under 1% of a full-length book');
  });

  it('shows growth past 1,000% as a factor and hides the comparison for all time', () => {
    const up = render(KpiTiles, { props: { data: payload({ words_in_range: 4000, words_prev_range: 100 }), rangeDays: 30 } }).body;
    expect(up).toContain('40×');
    const allTime = render(KpiTiles, { props: { data: payload({ words_in_range: 4000, words_prev_range: 100 }), rangeDays: 0 } }).body;
    expect(allTime).not.toContain('class="delta');
  });
});

describe('ContextBreakdown', () => {
  const contexts = [
    { id: 1, name: 'Everywhere', icon: null, color: null, is_everywhere: true },
    { id: 2, name: 'Coding', icon: 'code', color: '#88f' },
  ];

  it('lists known contexts as filters and unattributed words as a static Other row', () => {
    const { body } = render(ContextBreakdown, {
      props: {
        usage: [{ context_id: 2, words: 60, transcriptions: 2 }, { context_id: 9, words: 10, transcriptions: 1 }],
        totalWords: 100,
        contexts,
        onpick: () => {},
      },
    });
    expect(body).toContain('60% of your words went to Coding');
    expect(body).toContain('Deleted context');
    expect(body).toContain('Other');
    expect((body.match(/<button/g) ?? []).length).toBe(1);
  });
});

describe('CleanupCard', () => {
  it('marks partial history counts as minimums and handles no cleanup', () => {
    const partial = render(CleanupCard, { props: { cleanup: { ...EMPTY_INSIGHTS.cleanup, raw_words: 100, clean_words: 90, vocabulary_replacements: 3, activity_incomplete: true, activity_tracked_dictations: 4 } } }).body;
    expect(partial).toContain('≥');
    expect(partial).toContain('trimmed');
    const none = render(CleanupCard, { props: { cleanup: EMPTY_INSIGHTS.cleanup } }).body;
    expect(none).toContain('No cleaned dictations in this range yet.');
  });

  it('separates expired or partial history text from the range total', () => {
    const expired = render(CleanupCard, { props: { cleanup: { ...EMPTY_INSIGHTS.cleanup, text_dictations: 0 }, totalDictations: 5 } }).body;
    expect(expired).toContain('no longer in history');
    expect(expired).not.toContain('No cleaned dictations');
    // One dictation of fifty missing still qualifies the comparison.
    const partial = render(CleanupCard, { props: { cleanup: { ...EMPTY_INSIGHTS.cleanup, raw_words: 520, clean_words: 498, text_dictations: 49 }, totalDictations: 50 } }).body;
    expect(partial).toContain('Based on dictations still saved in history.');
    const full = render(CleanupCard, { props: { cleanup: { ...EMPTY_INSIGHTS.cleanup, raw_words: 520, clean_words: 498, text_dictations: 50 }, totalDictations: 50 } }).body;
    expect(full).not.toContain('Based on dictations still saved');
    const older = render(CleanupCard, { props: { cleanup: { ...EMPTY_INSIGHTS.cleanup, text_dictations: undefined }, totalDictations: 5 } }).body;
    expect(older).toContain('No cleaned dictations in this range yet.');
  });
});
