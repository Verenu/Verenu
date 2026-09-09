import { HISTORY_PAGE_SIZE, type Entry } from './helpers';

/** Keep the Home view responsive even when a user has a very long history. */
export const HISTORY_MAX_RETAINED = HISTORY_PAGE_SIZE * 5;

export type HistoryPageResult = {
  entries: Entry[];
  nextOffset: number;
  hasMore: boolean;
};

/**
 * Applies one database page to the in-memory Home window. `nextOffset` is
 * independent from `entries.length` because the window may evict older rows.
 */
export function applyHistoryPage(
  current: Entry[],
  page: Entry[],
  offset: number,
  reset: boolean,
  maxRetained = HISTORY_MAX_RETAINED,
): HistoryPageResult {
  const nextOffset = reset ? page.length : offset + page.length;
  // Pages are newest-first. Once the bounded window is full, drop the rows at
  // the front so the newly requested older page remains reachable.
  const entries = (reset ? page : [...current, ...page]).slice(-maxRetained);
  return {
    entries,
    nextOffset,
    // The retained window is only a memory limit. It must not decide whether
    // the database has another page.
    hasMore: page.length === HISTORY_PAGE_SIZE,
  };
}
