import { describe, expect, it } from 'vitest';
import {
  activeDayCount,
  contextShares,
  costPerThousandWords,
  dailyAverage,
  fmtHoursMinutes,
  fmtPercent,
  peakWeekday,
  speedupOverTyping,
  timeSavedMs,
  weekdayStats,
  weekendShare,
} from './analytics';

const day = (d: string, words: number) => ({ day: d, words, transcriptions: words ? 1 : 0, speaking_ms: 0 });

describe('timeSavedMs', () => {
  it('compares speaking time with typing at 40 wpm', () => {
    // 400 words typed at 40 wpm = 10 minutes; spoken in 3 minutes saves 7.
    expect(timeSavedMs(400, 3 * 60_000)).toBe(7 * 60_000);
  });
  it('never goes negative and ignores bad input', () => {
    expect(timeSavedMs(10, 60 * 60_000)).toBe(0);
    expect(timeSavedMs(0, 0)).toBe(0);
    expect(timeSavedMs(Number.NaN, 0)).toBe(0);
    expect(timeSavedMs(400, Number.NaN)).toBe(10 * 60_000);
  });
});

describe('speedupOverTyping', () => {
  it('is null without a measured pace', () => {
    expect(speedupOverTyping(0)).toBeNull();
    expect(speedupOverTyping(120)).toBe(3);
  });
});

describe('weekday rhythm', () => {
  // 2026-11-02 is a Monday.
  const daily = [
    day('2026-11-02', 100), day('2026-11-03', 0), day('2026-11-04', 300),
    day('2026-11-05', 0), day('2026-11-06', 0), day('2026-11-07', 50), day('2026-11-08', 50),
    day('2026-11-09', 300),
  ];
  const stats = weekdayStats(daily);

  it('averages each weekday across its calendar days, idle days included', () => {
    expect(stats[1]).toEqual({ weekday: 1, total: 400, days: 2, average: 200 });
    expect(stats[3].average).toBe(300);
    expect(stats[2].average).toBe(0);
  });
  it('picks the busiest weekday and the weekend share', () => {
    expect(peakWeekday(stats)?.weekday).toBe(3);
    expect(weekendShare(stats)).toBeCloseTo(100 / 800);
  });
  it('handles empty and malformed series', () => {
    const empty = weekdayStats([day('not-a-day', 5)]);
    expect(empty.every((slot) => slot.days === 0)).toBe(true);
    expect(peakWeekday(empty)).toBeNull();
    expect(weekendShare(empty)).toBeNull();
  });
  it('counts active days and the per-day mean', () => {
    expect(activeDayCount(daily)).toBe(5);
    expect(dailyAverage(daily)).toBe(100);
    expect(dailyAverage([])).toBe(0);
  });
});

describe('contextShares', () => {
  it('adds an unattributed remainder so shares sum to the range total', () => {
    const rows = contextShares([{ context_id: 2, words: 30, transcriptions: 3 }, { context_id: 1, words: 60, transcriptions: 6 }], 100);
    expect(rows.map((r) => [r.contextId, r.words])).toEqual([[1, 60], [2, 30], [null, 10]]);
    expect(rows.reduce((sum, r) => sum + r.share, 0)).toBeCloseTo(1);
  });
  it('folds contexts beyond the limit into the remainder', () => {
    const usage = Array.from({ length: 7 }, (_, i) => ({ context_id: i + 1, words: 10, transcriptions: 1 }));
    const rows = contextShares(usage, 70, 5);
    expect(rows).toHaveLength(6);
    expect(rows[5]).toMatchObject({ contextId: null, words: 20, transcriptions: 2 });
  });
  it('tolerates a total that lags the listed words and missing data', () => {
    expect(contextShares([{ context_id: 1, words: 50, transcriptions: 1 }], 10)[0].share).toBe(1);
    expect(contextShares(undefined, 0)).toEqual([]);
    expect(contextShares([{ context_id: 1, words: 0, transcriptions: 2 }], 0)).toEqual([]);
  });
});

describe('formatting', () => {
  it('formats headline durations', () => {
    expect(fmtHoursMinutes(0)).toEqual({ value: '0', unit: 'min' });
    expect(fmtHoursMinutes(45 * 60_000)).toEqual({ value: '45', unit: 'min' });
    expect(fmtHoursMinutes(62 * 60_000)).toEqual({ value: '1', unit: 'hr' });
    expect(fmtHoursMinutes(3.24 * 3_600_000)).toEqual({ value: '3.2', unit: 'hrs' });
    expect(fmtHoursMinutes(1234 * 3_600_000)).toEqual({ value: (1234).toLocaleString(), unit: 'hrs' });
  });
  it('keeps tiny and near-total percentages honest', () => {
    expect(fmtPercent(0.004)).toBe('<1%');
    expect(fmtPercent(0.996)).toBe('>99%');
    expect(fmtPercent(1)).toBe('100%');
    expect(fmtPercent(0)).toBe('0%');
  });
  it('prices per thousand words', () => {
    expect(costPerThousandWords(0.5, 10_000)).toBeCloseTo(0.05);
    expect(costPerThousandWords(null, 10)).toBeNull();
    expect(costPerThousandWords(1, 0)).toBeNull();
  });
});
