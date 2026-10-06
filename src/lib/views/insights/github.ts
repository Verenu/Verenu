import type { InsightsDay } from './types';

export interface GithubSnapshot {
  username: string;
  fetched_at: number;
  start_day: string;
  end_day: string;
  utc_offset: number;
  complete: boolean;
  daily: Array<{ day: string; commits: number }>;
  warning: string | null;
}

export function alignCommits(daily: InsightsDay[], snapshot: GithubSnapshot | null): Array<number | null> {
  const counts = new Map(snapshot?.daily.map(d => [d.day, d.commits]));
  return daily.map(({ day }) => {
    if (!snapshot || day < snapshot.start_day || day > snapshot.end_day) return null;
    const count = counts.get(day);
    if (count === undefined || (!snapshot.complete && count === 0)) return null;
    return count;
  });
}

/** Unknown dates break the line; never interpolate across missing coverage. */
export function commitPath(counts: Array<number | null>, x: (index: number) => number, y: (count: number) => number): string {
  let connected = false;
  return counts.map((count, i) => {
    if (count === null) { connected = false; return ''; }
    const command = connected ? 'L' : 'M';
    connected = true;
    return `${command} ${x(i)} ${y(count)}`;
  }).filter(Boolean).join(' ');
}
