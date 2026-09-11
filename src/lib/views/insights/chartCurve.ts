/*
 * Catmull-Rom → cubic bezier for the daily chart's curve.
 *
 * The drawn line and the hover indicator have to agree exactly: the indicator
 * rides the curve rather than cutting between data points, so if it derived its
 * own control points any difference would float the dot off the line. Both read
 * the segments built here.
 */

export interface Point {
  x: number;
  y: number;
}

export interface Segment {
  p1: Point;
  c1: Point;
  c2: Point;
  p2: Point;
}

export function buildSegments(points: Point[], minY: number, maxY: number): Segment[] {
  const clampY = (value: number) => Math.max(minY, Math.min(maxY, value));
  const segments: Segment[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i - 1] ?? points[i];
    const p1 = points[i];
    const p2 = points[i + 1];
    const p3 = points[i + 2] ?? p2;
    segments.push({
      p1,
      p2,
      // Catmull-Rom handles gentle curves well, but its control points can
      // overshoot between a large spike and a zero-value day. Keeping them in
      // the plot bounds preserves the curve without inventing negative data.
      c1: { x: p1.x + (p2.x - p0.x) / 6, y: clampY(p1.y + (p2.y - p0.y) / 6) },
      c2: { x: p2.x - (p3.x - p1.x) / 6, y: clampY(p2.y - (p3.y - p1.y) / 6) },
    });
  }
  return segments;
}

export function segmentsToPath(segments: Segment[]): string {
  if (segments.length === 0) return '';
  let path = `M ${segments[0].p1.x} ${segments[0].p1.y}`;
  for (const s of segments) {
    path += ` C ${s.c1.x} ${s.c1.y}, ${s.c2.x} ${s.c2.y}, ${s.p2.x} ${s.p2.y}`;
  }
  return path;
}

/**
 * The point on the curve at a fractional data index, clamped to the ends so an
 * overshooting spring parks the indicator on the last day rather than off the
 * side of the plot.
 */
export function pointOnSegments(segments: Segment[], at: number): Point {
  if (segments.length === 0) return { x: 0, y: 0 };
  const last = segments.length - 1;
  const clamped = Math.max(0, Math.min(segments.length, at));
  const i = Math.max(0, Math.min(last, Math.floor(clamped)));
  const t = clamped - i;
  const s = segments[i];
  const u = 1 - t;
  const w1 = u * u * u;
  const w2 = 3 * u * u * t;
  const w3 = 3 * u * t * t;
  const w4 = t * t * t;
  return {
    x: w1 * s.p1.x + w2 * s.c1.x + w3 * s.c2.x + w4 * s.p2.x,
    y: w1 * s.p1.y + w2 * s.c1.y + w3 * s.c2.y + w4 * s.p2.y,
  };
}
