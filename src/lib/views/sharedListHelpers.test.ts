import { afterEach, describe, expect, it, vi } from 'vitest';
import { fmtDate, countCodePoints } from './sharedListHelpers';
import { fmtDate as dictionaryDate, countCodePoints as dictionaryCount } from './dictionary/helpers';
import { fmtDate as snippetDate, countCodePoints as snippetCount } from './snippets/helpers';

describe('shared legacy list formatting', () => {
  afterEach(() => vi.useRealTimers());

  it('keeps both page exports on the same implementation', () => {
    expect(dictionaryDate).toBe(fmtDate);
    expect(snippetDate).toBe(fmtDate);
    expect(dictionaryCount).toBe(countCodePoints);
    expect(snippetCount).toBe(countCodePoints);
  });

  it('handles UTC timestamps and Unicode code points', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-05T12:00:00Z'));
    expect(fmtDate('2026-10-05T11:00:00')).toBe('Today');
    expect(fmtDate('2026-10-04T11:00:00Z')).toBe('Yesterday');
    expect(countCodePoints('A😀')).toBe(2);
  });

  it.each(['', 'abc', '😀😀', '\ud800', '\udc00', 'a\ud800b', 'e\u0301', '👩‍💻'])('counts %j without changing Unicode semantics', (value) => {
    expect(countCodePoints(value)).toBe([...value].length);
  });
});
