import { describe, expect, it } from 'vitest';
import { buildSegments, pointOnSegments, segmentsToPath, type Point } from './chartCurve';

const MIN_Y = 12;
const MAX_Y = 148;

function points(ys: number[]): Point[] {
  return ys.map((y, i) => ({ x: i * 50, y }));
}

describe('chartCurve', () => {
  it('builds one segment per gap, anchored on the data points', () => {
    const pts = points([100, 40, 90, 20]);
    const segments = buildSegments(pts, MIN_Y, MAX_Y);
    expect(segments).toHaveLength(3);
    segments.forEach((s, i) => {
      expect(s.p1).toEqual(pts[i]);
      expect(s.p2).toEqual(pts[i + 1]);
    });
  });

  // The invariant the hover indicator depends on: at a whole index the dot must
  // sit exactly on that day's point, or it visibly floats off the line.
  it('lands exactly on the data point at every whole index', () => {
    const pts = points([100, 40, 90, 20, 75]);
    const segments = buildSegments(pts, MIN_Y, MAX_Y);
    pts.forEach((p, i) => {
      const at = pointOnSegments(segments, i);
      expect(at.x).toBeCloseTo(p.x, 6);
      expect(at.y).toBeCloseTo(p.y, 6);
    });
  });

  it('advances monotonically in x between points', () => {
    const segments = buildSegments(points([100, 40, 90, 20]), MIN_Y, MAX_Y);
    let prev = -Infinity;
    for (let at = 0; at <= 3; at += 0.05) {
      const { x } = pointOnSegments(segments, at);
      expect(x).toBeGreaterThan(prev);
      prev = x;
    }
  });

  it('clamps past either end instead of running off the plot', () => {
    const pts = points([100, 40, 90]);
    const segments = buildSegments(pts, MIN_Y, MAX_Y);
    expect(pointOnSegments(segments, -3).x).toBeCloseTo(pts[0].x, 6);
    expect(pointOnSegments(segments, 99).x).toBeCloseTo(pts[2].x, 6);
  });

  it('keeps the curve inside the plot bounds over a spike next to a zero day', () => {
    const segments = buildSegments(points([MAX_Y, MIN_Y, MAX_Y, MIN_Y]), MIN_Y, MAX_Y);
    for (let at = 0; at <= 3; at += 0.02) {
      const { y } = pointOnSegments(segments, at);
      expect(y).toBeGreaterThanOrEqual(MIN_Y);
      expect(y).toBeLessThanOrEqual(MAX_Y);
    }
  });

  it('emits an empty path when there is nothing to draw', () => {
    expect(segmentsToPath([])).toBe('');
    expect(segmentsToPath(buildSegments(points([50]), MIN_Y, MAX_Y))).toBe('');
  });
});
