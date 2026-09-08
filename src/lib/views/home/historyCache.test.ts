import { describe, expect, it } from 'vitest';
import { HISTORY_PAGE_SIZE } from './helpers';
import { applyHistoryPage } from './historyCache';

function page(start: number, count: number) {
  return Array.from({ length: count }, (_, index) => ({
    id: start - index,
    clean_text: `entry ${start - index}`,
    words: 1,
    created_at: new Date(2026, 0, 1, 0, 0, start - index).toISOString(),
  }));
}

describe('history page cache', () => {
  it('advances the database cursor independently of the retained array', () => {
    const first = applyHistoryPage([], page(99, HISTORY_PAGE_SIZE), 0, true);
    const second = applyHistoryPage(first.entries, page(-1, HISTORY_PAGE_SIZE), first.nextOffset, false);

    expect(first.entries).toHaveLength(HISTORY_PAGE_SIZE);
    expect(second.nextOffset).toBe(HISTORY_PAGE_SIZE * 2);
    expect(second.entries).toHaveLength(HISTORY_PAGE_SIZE * 2);
    expect(second.entries[HISTORY_PAGE_SIZE].id).toBe(-1);
  });

  it('evicts newer pages while keeping older pages reachable', () => {
    const max = HISTORY_PAGE_SIZE * 2;
    const first = applyHistoryPage([], page(199, HISTORY_PAGE_SIZE), 0, true, max);
    const second = applyHistoryPage(first.entries, page(99, HISTORY_PAGE_SIZE), first.nextOffset, false, max);
    const third = applyHistoryPage(second.entries, page(-1, HISTORY_PAGE_SIZE), second.nextOffset, false, max);

    expect(third.entries).toHaveLength(max);
    expect(third.entries[0].id).toBe(99);
    expect(third.entries[third.entries.length - 1]?.id).toBe(-100);
    expect(third.nextOffset).toBe(HISTORY_PAGE_SIZE * 3);
    expect(third.hasMore).toBe(true);

    const fourth = applyHistoryPage(third.entries, page(-101, 3), third.nextOffset, false, max);
    expect(fourth.entries).toHaveLength(max);
    expect(fourth.entries[0].id).toBe(96);
    expect(fourth.entries[fourth.entries.length - 1]?.id).toBe(-103);
    expect(fourth.hasMore).toBe(false);
  });

  it('keeps paginating after the retention window is full', () => {
    const result = applyHistoryPage([], page(99, HISTORY_PAGE_SIZE), 0, true, HISTORY_PAGE_SIZE);
    expect(result.hasMore).toBe(true);
  });
});
