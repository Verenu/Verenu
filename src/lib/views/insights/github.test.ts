import { describe, expect, it } from 'vitest';
import { alignCommits, commitPath, type GithubSnapshot } from './github';

const snapshot: GithubSnapshot = {
  username: 'octocat', fetched_at: 1, start_day: '2026-10-01', end_day: '2026-10-03',
  utc_offset: 0, complete: true, warning: null,
  daily: [{ day: '2026-10-01', commits: 0 }, { day: '2026-10-02', commits: 8 }],
};
const daily = ['2026-09-30', '2026-10-01', '2026-10-02', '2026-10-03'].map(day => ({ day, words: 100, transcriptions: 1, speaking_ms: 1000 }));

describe('GitHub overlay coverage', () => {
  it('aligns by date and keeps missing/outside dates distinct from zero', () => {
    expect(alignCommits(daily, snapshot)).toEqual([null, 0, 8, null]);
    expect(alignCommits(daily, null)).toEqual([null, null, null, null]);
  });
  it('does not claim zero commits for incomplete searches', () => {
    expect(alignCommits(daily, { ...snapshot, complete: false })).toEqual([null, null, 8, null]);
  });
  it('breaks the green line across unknown dates', () => {
    expect(commitPath([0, 4, null, 3], i => i * 10, c => 100 - c)).toBe('M 0 100 L 10 96 M 30 97');
  });
});
