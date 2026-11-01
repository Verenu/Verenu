import { describe, expect, it } from 'vitest';
import { pillBarHeight, pillBarSizing } from './pillSizing';

describe('pill bar sizing', () => {
  it.each([1, 1.25, 1.5, 1.75, 2])('uses a whole-pixel width and pitch at scale %s', (scale) => {
    const { width, gap } = pillBarSizing(scale);
    expect(width * scale).toBeCloseTo(Math.round(width * scale));
    expect((width + gap) * scale).toBeCloseTo(Math.round(5 * scale));
    expect(gap).toBeGreaterThan(0);
  });
});

describe('pill bar heights', () => {
  it.each([1, 1.25, 1.5, 1.75, 2])('keeps the rest aligned and movement continuous at scale %s', (scale) => {
    const { width } = pillBarSizing(scale);
    const rest = pillBarHeight(3, scale, width);
    expect(rest).toBeGreaterThanOrEqual(width);
    expect(rest * scale / 2).toBeCloseTo(Math.round(rest * scale / 2));
    expect(pillBarHeight(16, scale, width)).toBeCloseTo(16);
    expect(pillBarHeight(8.1, scale, width)).toBeGreaterThan(pillBarHeight(8, scale, width));
    expect(pillBarHeight(8.1, scale, width) - pillBarHeight(8, scale, width)).toBeLessThan(0.2);
  });
});
