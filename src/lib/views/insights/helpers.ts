const WORDS_PER_BOOK = 80_000;

const subCentUsd = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumSignificantDigits: 2,
  maximumSignificantDigits: 4,
  useGrouping: false,
});

export function fmtNumber(n: number): string {
  return Math.round(n).toLocaleString();
}

export function fmtCompact(n: number): { value: string; suffix: string } {
  const abs = Math.abs(n);
  if (Math.round(abs / 1000) >= 1000) return { value: (n / 1e6).toFixed(1), suffix: 'M' };
  if (Math.round(abs) >= 1000) return { value: (n / 1000).toFixed(1), suffix: 'k' };
  return { value: String(Math.round(n)), suffix: '' };
}

export function fmtUsd(n: number | null): string {
  if (n === null) return '—';
  if (n === 0) return '$0.00';
  if (n > 0 && n < 0.01) {
    return subCentUsd.format(n);
  }
  if (n < 0) {
    const abs = n.toFixed(2).replace('-', '');
    return abs === '0.00' ? '$0.00' : `-$${abs}`;
  }
  return `$${n.toFixed(2)}`;
}

export function fmtDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '0s';
  const totalSeconds = Math.round(ms / 1000);
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${String(s).padStart(2, '0')}s`;
  return `${s}s`;
}

export function pctDelta(current: number, previous: number): number | null {
  if (!Number.isFinite(current) || !Number.isFinite(previous) || previous <= 0) return null;
  return ((current - previous) / previous) * 100;
}

export function bookEquivalent(words: number): number {
  return words / WORDS_PER_BOOK;
}

export function niceCeiling(max: number): number {
  if (!Number.isFinite(max) || max <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(max));
  const normalized = max / magnitude;
  let step: number;
  if (normalized <= 1 + 1e-9) {
    step = 1;
  } else if (normalized <= 2 + 1e-9) {
    step = 2;
  } else if (normalized <= 5 + 1e-9) {
    step = 5;
  } else {
    step = 10;
  }
  return step * magnitude;
}

export function parseLocalDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  if (!Number.isInteger(y) || !Number.isInteger(m) || !Number.isInteger(d) || y < 1) {
    return new Date(NaN);
  }
  const date = new Date(y, m - 1, d);
  return date.getFullYear() === y && date.getMonth() === m - 1 && date.getDate() === d
    ? date
    : new Date(NaN);
}

export function fmtDay(day: string): string {
  try {
    const date = parseLocalDay(day);
    if (Number.isNaN(date.getTime())) return day;
    return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch {
    return day;
  }
}

export function fmtDayLong(day: string): string {
  try {
    const date = parseLocalDay(day);
    if (Number.isNaN(date.getTime())) return day;
    return date.toLocaleDateString([], {
      weekday: 'short',
      month: 'short',
      day: 'numeric',
    });
  } catch {
    return day;
  }
}

export function fmtHour(hour: number): string {
  const suffix = hour < 12 ? 'AM' : 'PM';
  const h = hour % 12 === 0 ? 12 : hour % 12;
  return `${h} ${suffix}`;
}

export const PACE_TICKS = 44;

export function paceScale(bestWpm: number, ticks = PACE_TICKS): { max: number; bestTick: number } {
  const max = Math.min(250, Math.max(200, Math.ceil(bestWpm / 50) * 50));
  if (bestWpm <= 0) return { max, bestTick: -1 };
  const tick = Math.max(0, Math.round((bestWpm / max) * ticks) - 1);
  return { max, bestTick: tick >= ticks - 2 ? ticks - 1 : tick };
}

export const PACE_TICK_W = 2;

export function paceTickOffset(
  i: number,
  rulerWidth: number,
  dpr: number = (typeof window === 'undefined' ? 1 : window.devicePixelRatio) || 1,
  ticks = PACE_TICKS
): string {
  const span = ticks - 1;
  if (rulerWidth <= PACE_TICK_W) return `${(i * 100) / span}%`;
  const raw = (i * (rulerWidth - PACE_TICK_W)) / span;
  return `${Math.round(raw * dpr) / dpr}px`;
}
