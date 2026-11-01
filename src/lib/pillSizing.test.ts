import { describe, expect, it } from 'vitest';
import { pillBarSizing, snapBarHeight } from './pillSizing';

describe('pill bar sizing', () => {
  it.each([1, 1.25, 1.5, 1.75, 2])('uses a whole-pixel width and pitch at scale %s', (scale) => {
    const { width, gap } = pillBarSizing(scale);
    expect(width * scale).toBeCloseTo(Math.round(width * scale));
    expect((width + gap) * scale).toBeCloseTo(Math.round(5 * scale));
    expect(gap).toBeGreaterThan(0);
  });
});

describe('pill bar height snapping', () => {
  it.each([1, 1.25, 1.5, 2])('uses even whole-pixel heights, never shorter than the width, at scale %s', (scale) => {
    const { width } = pillBarSizing(scale);
    for (const h of [3, 3.4, 7.77, 10.5, 16]) {
      const px = snapBarHeight(h, scale, width) * scale;
      expect(px).toBeCloseTo(Math.round(px));
      expect(Math.round(px) % 2).toBe(0);
      expect(px).toBeGreaterThanOrEqual(width * scale);
    }
  });
  it.each([1, 1.25, 1.5, 2])('keeps resting bars dot-sized at scale %s', (scale) => {
    const { width } = pillBarSizing(scale);
    const px = snapBarHeight(3, scale, width) * scale;
    expect(Math.round(px)).toBe(2 * Math.round((width * scale) / 2));
  });
});
