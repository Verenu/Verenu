import { describe, expect, it } from 'vitest';
import { RollSpring } from './rollingSpring';

/** Runs a roll to completion and reports how long it took, in ms. */
function settleMs(frameMs: number, carriedVelocity = 0): number {
  const spring = new RollSpring();
  spring.vel = carriedVelocity;
  spring.start(1);
  let elapsed = 0;
  for (let i = 0; i < 2000; i++) {
    elapsed += frameMs;
    if (!spring.step(frameMs / 1000)) return elapsed;
  }
  return Infinity;
}

describe('RollSpring', () => {
  // The budget is a product requirement, not a detail: a digit roll that
  // outlasts it stops reading as feedback and starts reading as lag.
  it('settles inside the 500ms budget at any frame rate', () => {
    for (const frameMs of [8.33, 16.67, 32]) {
      expect(settleMs(frameMs)).toBeLessThan(500);
    }
  });

  it('still settles when restarted mid-flight at full speed', () => {
    expect(settleMs(16.67, -26)).toBeLessThan(500);
    expect(settleMs(16.67, 26)).toBeLessThan(500);
  });

  it('does not overshoot far enough to read as a bounce', () => {
    const spring = new RollSpring();
    spring.start(1);
    let furthestPast = 0;
    while (spring.step(1 / 60)) furthestPast = Math.min(furthestPast, spring.pos);
    expect(furthestPast).toBeGreaterThan(-0.05);
  });

  it('reaches a speed worth blurring on the way', () => {
    const spring = new RollSpring();
    spring.start(1);
    let peak = 0;
    while (spring.step(1 / 60)) peak = Math.max(peak, Math.abs(spring.vel));
    expect(peak).toBeGreaterThan(5);
  });
});
