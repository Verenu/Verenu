import { parseLocalDay } from './helpers';
import type { InsightsContextUsage, InsightsDay } from './types';

/** Average adult typing speed used for the "time saved" estimate. */
export const TYPING_WPM = 40;

/**
 * Time saved by speaking instead of typing the same words at `typingWpm`.
 * Never negative: a slow speaker saves nothing rather than "losing" time.
 */
export function timeSavedMs(words: number, speakingMs: number, typingWpm = TYPING_WPM): number {
  if (!Number.isFinite(words) || words <= 0 || typingWpm <= 0) return 0;
  const typingMs = (words / typingWpm) * 60_000;
  const spoken = Number.isFinite(speakingMs) && speakingMs > 0 ? speakingMs : 0;
  return Math.max(0, typingMs - spoken);
}

/** How many times faster speaking was than typing, or null when it cannot be measured. */
export function speedupOverTyping(avgWpm: number, typingWpm = TYPING_WPM): number | null {
  if (!Number.isFinite(avgWpm) || avgWpm <= 0 || typingWpm <= 0) return null;
  return avgWpm / typingWpm;
}

export function activeDayCount(daily: InsightsDay[]): number {
  return daily.reduce((count, day) => count + (day.words > 0 ? 1 : 0), 0);
}

export interface WeekdayStat {
  /** 0 = Sunday, matching `Date.getDay()`. */
  weekday: number;
  total: number;
  /** Calendar days of this weekday inside the range, active or not. */
  days: number;
  /** Mean words on this weekday across the range, counting idle days as zero. */
  average: number;
}

export function weekdayStats(daily: InsightsDay[]): WeekdayStat[] {
  const stats = Array.from({ length: 7 }, (_, weekday) => ({ weekday, total: 0, days: 0, average: 0 }));
  for (const entry of daily) {
    const date = parseLocalDay(entry.day);
    if (Number.isNaN(date.getTime())) continue;
    const slot = stats[date.getDay()];
    slot.days += 1;
    slot.total += Math.max(0, entry.words);
  }
  for (const slot of stats) slot.average = slot.days > 0 ? slot.total / slot.days : 0;
  return stats;
}

/** Monday-first order reads more naturally as a work week. */
export const WEEKDAY_ORDER = [1, 2, 3, 4, 5, 6, 0] as const;
export const WEEKDAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const;
export const WEEKDAY_LONG = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'] as const;

export function peakWeekday(stats: WeekdayStat[]): WeekdayStat | null {
  let best: WeekdayStat | null = null;
  for (const slot of stats) {
    if (slot.average <= 0) continue;
    if (!best || slot.average > best.average) best = slot;
  }
  return best;
}

/** Share of words on Saturday and Sunday, or null with no words. */
export function weekendShare(stats: WeekdayStat[]): number | null {
  const total = stats.reduce((sum, slot) => sum + slot.total, 0);
  if (total <= 0) return null;
  return (stats[0].total + stats[6].total) / total;
}

export interface ContextShare {
  /** `null` is the remainder that no listed context accounts for. */
  contextId: number | null;
  words: number;
  transcriptions: number;
  share: number;
}

/**
 * Turn per-context usage into display shares. Words that no context accounts for
 * (unassigned history, deleted contexts) become a trailing `contextId: null` row so the
 * shares always add up to the range total.
 */
export function contextShares(usage: InsightsContextUsage[] | undefined, totalWords: number, limit = 5): ContextShare[] {
  const rows = (usage ?? []).filter((row) => row.words > 0).sort((a, b) => b.words - a.words || a.context_id - b.context_id);
  const listed = rows.reduce((sum, row) => sum + row.words, 0);
  const total = Math.max(totalWords, listed);
  if (total <= 0) return [];
  const head = rows.slice(0, limit);
  const tail = rows.slice(limit);
  const shares: ContextShare[] = head.map((row) => ({
    contextId: row.context_id,
    words: row.words,
    transcriptions: row.transcriptions,
    share: row.words / total,
  }));
  const tailWords = tail.reduce((sum, row) => sum + row.words, 0) + (total - listed);
  if (tailWords > 0) {
    shares.push({
      contextId: null,
      words: tailWords,
      transcriptions: tail.reduce((sum, row) => sum + row.transcriptions, 0),
      share: tailWords / total,
    });
  }
  return shares;
}

export function costPerThousandWords(cost: number | null, words: number): number | null {
  if (cost === null || !Number.isFinite(cost) || !Number.isFinite(words) || words <= 0) return null;
  return (cost / words) * 1000;
}

/** Mean words per calendar day of the range, idle days included. */
export function dailyAverage(daily: InsightsDay[]): number {
  if (daily.length === 0) return 0;
  return daily.reduce((sum, day) => sum + Math.max(0, day.words), 0) / daily.length;
}

/** Coarse duration for headline figures: "0 min", "45 min", "3.2 hrs", "128 hrs". */
export function fmtHoursMinutes(ms: number): { value: string; unit: string } {
  if (!Number.isFinite(ms) || ms < 60_000) return { value: '0', unit: 'min' };
  const totalMinutes = Math.round(ms / 60_000);
  if (totalMinutes < 60) return { value: String(totalMinutes), unit: 'min' };
  const hours = totalMinutes / 60;
  if (hours < 10) {
    const value = hours.toFixed(1).replace(/\.0$/, '');
    return { value, unit: value === '1' ? 'hr' : 'hrs' };
  }
  return { value: Math.round(hours).toLocaleString(), unit: 'hrs' };
}

export function fmtPercent(fraction: number): string {
  if (!Number.isFinite(fraction)) return '0%';
  const pct = fraction * 100;
  if (pct > 0 && pct < 1) return '<1%';
  if (pct < 100 && pct > 99) return '>99%';
  return `${Math.round(pct)}%`;
}
