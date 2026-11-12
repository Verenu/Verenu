import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import DailyChart from './DailyChart.svelte';
import type { GithubSnapshot } from './github';

describe('GitHub chart legend', () => {
  it.each(['contributions', 'public_commits'] as const)('keeps the %s comparison without a scale popover', (source) => {
    const github: GithubSnapshot = {
      source, username: 'fixture-user', fetched_at: 1,
      start_day: '2026-10-01', end_day: '2026-10-01', utc_offset: 0,
      complete: true, warning: null, daily: [{ day: '2026-10-01', commits: 3 }],
    };
    const { body } = render(DailyChart, {
      props: {
        daily: [{ day: '2026-10-01', words: 200, transcriptions: 1, speaking_ms: 1000 }],
        rangeLabel: 'Today', github,
      },
    });
    expect(body).toContain(source === 'contributions' ? 'Contributions' : 'Public commits');
    expect(body).toContain('Bars use independent scales to compare trends.');
    expect(body).not.toContain('Chart scale details');
    expect(body).not.toContain('<details');
  });

  it('pairs word and known GitHub bars on short ranges and keeps lines on longer ranges', () => {
    const daily = Array.from({ length: 15 }, (_, i) => ({
      day: `2026-10-${String(i + 1).padStart(2, '0')}`, words: 100 + i, transcriptions: 1, speaking_ms: 1000,
    }));
    const github: GithubSnapshot = {
      source: 'contributions', username: 'fixture-user', fetched_at: 1,
      start_day: daily[0].day, end_day: daily[14].day, utc_offset: 0,
      complete: true, warning: null,
      daily: [{ day: daily[0].day, commits: 3 }, { day: daily[1].day, commits: 0 }],
    };
    const short = render(DailyChart, { props: { daily: daily.slice(0, 7), rangeLabel: 'Last 7 days', github } }).body;
    expect(short.match(/class="word-bar(?:\s|\")/g)).toHaveLength(7);
    expect(short.match(/class="github-bar(?:\s|\")/g)).toHaveLength(2);
    const bars = [...short.matchAll(/<path class="(?:word|github)-bar(?:\s[^"]*)?" d="([^"]+)"/g)];
    expect(bars).toHaveLength(9);
    for (const [, path] of bars) {
      expect(path).toMatch(/^M [\d.]+ 148 V [\d.]+ Q [\d.]+ [\d.]+ [\d.]+ [\d.]+ H [\d.]+ Q [\d.]+ [\d.]+ [\d.]+ [\d.]+ V 148 Z$/);
    }
    expect(short).not.toContain('class="github-line');
    expect(short).toContain('0 contributions');

    const long = render(DailyChart, { props: { daily, rangeLabel: 'Last 15 days', github } }).body;
    expect(long).toContain('class="github-line');
    expect(long).toContain('Lines use independent scales to compare trends.');
    expect(long).not.toContain('class="github-bar');
  });
});
