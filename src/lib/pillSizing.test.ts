import { describe, expect, it } from 'vitest';
import { pillBarSizing } from './pillSizing';

describe('pill bar sizing', () => {
  it.each([1, 1.25, 1.5, 1.75, 2])('uses a whole-pixel width and pitch at scale %s', (scale) => {
    const { width, gap } = pillBarSizing(scale);
    expect(width * scale).toBeCloseTo(Math.round(width * scale));
    expect((width + gap) * scale).toBeCloseTo(Math.round(5 * scale));
    expect(gap).toBeGreaterThan(0);
  });
});
