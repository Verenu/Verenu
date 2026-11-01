import { describe, expect, it } from 'vitest';
import { pillBarHeight, pillBarSizing } from './pillSizing';

describe('pill bar sizing', () => {
  it.each([1, 1.25, 1.5, 1.75, 2])('uses a whole-pixel width and pitch at scale %s', (scale) => {
    const { width, gap } = pillBarSizing(scale);
    expect(width * scale).toBeCloseTo(Math.round(width * scale));
    expect((width + gap) * scale).toBeCloseTo(Math.round(5 * scale));
    expect(gap).toBeGreaterThan(0);
  });
  it.each([1.25, 1.5, 1.75])('keeps the pitch on both grids at display scale %s and backing scale 2', (scale) => {
    const { width, gap, sharedGrid } = pillBarSizing(scale, 2);
    const pitch = width + gap;
    expect(sharedGrid).toBe(true);
    expect(pitch * scale).toBeCloseTo(Math.round(pitch * scale));
    expect(pitch * 2).toBeCloseTo(Math.round(pitch * 2));
    expect(width * scale).toBeCloseTo(Math.round(width * scale));
    expect(gap).toBeGreaterThan(0);
    expect(Math.abs(pitch - 5)).toBeLessThanOrEqual(1);
  });
  it('keeps the usual dimensions when the display and backing scales match', () => {
    expect(pillBarSizing(1, 1)).toEqual({ width: 3, gap: 2, sharedGrid: false });
  });
  it('avoids an excessively wide shared pitch when the grids do not align nearby', () => {
    const sizing = pillBarSizing(1.1, 2);
    expect(sizing.sharedGrid).toBe(false);
    expect(Math.abs(sizing.width + sizing.gap - 5)).toBeLessThan(0.5);
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
