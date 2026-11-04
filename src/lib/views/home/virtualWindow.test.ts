import { describe, expect, it } from 'vitest';
import { virtualWindowUnchanged } from './virtualWindow';

describe('virtualWindowUnchanged', () => {
  const rows = [{ key: 'row-30' }, { key: 'row-29' }];

  it('reuses the slice when nothing changed', () => {
    const state = { start: 0, end: 2, items: rows, totalHeight: 148 };
    expect(virtualWindowUnchanged(state, { ...state })).toBe(true);
  });

  it('refreshes when a row is deleted but the index range stays the same', () => {
    const prev = { start: 0, end: 2, items: rows, totalHeight: 148 };
    const afterDelete = rows.filter((row) => row.key !== 'row-30');
    const next = { start: 0, end: 2, items: afterDelete, totalHeight: 74 };
    expect(virtualWindowUnchanged(prev, next)).toBe(false);
    expect(afterDelete.slice(next.start, next.end).map((r) => r.key)).toEqual(['row-29']);
  });

  it('refreshes when only the measured height changes', () => {
    const prev = { start: 0, end: 2, items: rows, totalHeight: 148 };
    expect(virtualWindowUnchanged(prev, { ...prev, totalHeight: 160 })).toBe(false);
  });
});
