import { HISTORY_PAGE_SIZE, type Entry } from './helpers';

export const HISTORY_MAX_RETAINED = HISTORY_PAGE_SIZE * 5;

export type HistoryPageResult = {
  entries: Entry[];
  nextOffset: number;
  hasMore: boolean;
};

export function applyHistoryPage(
  current: Entry[],
  page: Entry[],
  offset: number,
  reset: boolean,
  maxRetained = HISTORY_MAX_RETAINED,
): HistoryPageResult {
  const nextOffset = reset ? page.length : offset + page.length;
  const entries = (reset ? page : [...current, ...page]).slice(-maxRetained);
  return {
    entries,
    nextOffset,
    hasMore: page.length === HISTORY_PAGE_SIZE,
  };
}
