import type { InsightsDay } from './types';
import { buildSegments, segmentsToPath } from './chartCurve';

export interface GithubSnapshot {
  source: 'contributions' | 'public_commits';
  username: string;
  fetched_at: number;
  start_day: string;
  end_day: string;
  utc_offset: number;
  complete: boolean;
  daily: Array<{ day: string; commits: number | null }>;
  warning: string | null;
}

export function activityLabel(snapshot: GithubSnapshot): string {
  return snapshot.source === 'public_commits' ? 'Public commits' : 'Contributions';
}

export function activityDescription(count: number | null | undefined, snapshot: GithubSnapshot): string {
  if (count === null || count === undefined) {
    return snapshot.source === 'public_commits' ? 'Public commit count unknown' : 'Contribution count unavailable';
  }
  const label = snapshot.source === 'public_commits'
    ? `${fmtActivityCount(count, 'public commit')}`
    : `${fmtActivityCount(count, 'contribution')}`;
  return snapshot.complete ? label : `${label} (partial results)`;
}

function fmtActivityCount(count: number, noun: string): string {
  return `${new Intl.NumberFormat().format(count)} ${noun}${count === 1 ? '' : 's'}`;
}

export function activitySourceInfo(snapshot: GithubSnapshot): string {
  if (snapshot.source === 'public_commits') {
    return 'GitHub search returns public commits and can miss activity outside its public index. Results are limited to 1,000 commits.';
  }
  return "GitHub's profile calendar groups contributions such as commits, pull requests, and reviews by its calendar dates. It can include anonymized private contributions only when you enable Show private contributions on your profile.";
}

export function alignActivity(daily: InsightsDay[], snapshot: GithubSnapshot | null): Array<number | null> {
  const counts = new Map(snapshot?.daily.map(d => [d.day, d.commits]));
  return daily.map(({ day }) => {
    if (!snapshot || day < snapshot.start_day || day > snapshot.end_day) return null;
    return counts.get(day) ?? null;
  });
}

/**
 * Smooth path through each run of known GitHub dates, using the same curve as
 * the words line. Unknown dates break the path; they are never filled as zero
 * or interpolated. `area` closes each run down to the baseline for the fill.
 */
export function activityPath(
  counts: Array<number | null>,
  x: (index: number) => number,
  y: (count: number) => number,
  bounds: { top: number; bottom: number },
  area = false,
): string {
  const paths: string[] = [];
  let run: Array<{ x: number; y: number }> = [];
  const flush = () => {
    if (run.length > 1) {
      const line = segmentsToPath(buildSegments(run, bounds.top, bounds.bottom));
      paths.push(area ? `${line} L ${run[run.length - 1].x} ${bounds.bottom} L ${run[0].x} ${bounds.bottom} Z` : line);
    }
    run = [];
  };
  counts.forEach((count, i) => {
    if (count === null) flush();
    else run.push({ x: x(i), y: y(count) });
  });
  flush();
  return paths.filter(Boolean).join(' ');
}
