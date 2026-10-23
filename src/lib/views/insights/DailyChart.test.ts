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
    expect(body).toContain('Lines use independent scales to compare trends.');
    expect(body).not.toContain('Chart scale details');
    expect(body).not.toContain('<details');
  });
});
