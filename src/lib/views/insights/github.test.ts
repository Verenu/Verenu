import { describe, expect, it } from 'vitest';
import {
  activityDescription,
  activityLabel,
  activityPath,
  alignActivity,
  type GithubSnapshot,
} from './github';

const snapshot: GithubSnapshot = {
  source: 'contributions',
  username: 'fixture-user', fetched_at: 1, start_day: '2026-10-01', end_day: '2026-10-03',
  utc_offset: 0, complete: true, warning: null,
  daily: [
    { day: '2026-10-01', commits: 0 },
    { day: '2026-10-02', commits: 8 },
  ],
};
const daily = ['2026-09-30', '2026-10-01', '2026-10-02', '2026-10-03'].map(day => ({ day, words: 100, transcriptions: 1, speaking_ms: 1000 }));

describe('GitHub activity coverage', () => {
  it('aligns known calendar counts by date and keeps missing dates distinct from zero', () => {
    expect(alignActivity(daily, snapshot)).toEqual([null, 0, 8, null]);
    expect(alignActivity(daily, null)).toEqual([null, null, null, null]);
  });

  it('keeps days without indexed results unknown for partial public search', () => {
    const partial: GithubSnapshot = {
      ...snapshot,
      source: 'public_commits',
      complete: false,
      daily: [
        { day: '2026-10-01', commits: null },
        { day: '2026-10-02', commits: 8 },
      ],
    };
    expect(alignActivity(daily, partial)).toEqual([null, null, 8, null]);
    expect(activityDescription(8, partial)).toBe('8 public commits (partial results)');
    expect(activityDescription(null, partial)).toBe('Public commit count unknown');
  });

  it('uses contribution wording and the public-search fallback label', () => {
    expect(activityLabel(snapshot)).toBe('Contributions');
    expect(activityDescription(1, snapshot)).toBe('1 contribution');
    const fallback = { ...snapshot, source: 'public_commits' as const };
    expect(activityLabel(fallback)).toBe('Public commits');
    expect(activityDescription(1, fallback)).toBe('1 public commit');
  });

  it('breaks the smooth line across unknown dates', () => {
    const path = activityPath([0, 4, null, 3, 5], i => i * 10, count => 100 - count, { top: 0, bottom: 100 });
    expect(path.match(/M /g)).toHaveLength(2);
    expect(path).not.toContain('20 ');
  });

  it('does not draw a stray point for a single known day', () => {
    expect(activityPath([null, 4, null], i => i * 10, count => 100 - count, { top: 0, bottom: 100 })).toBe('');
  });
});
