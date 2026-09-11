/*
 * The spring behind RollingNumber's digit roll.
 *
 * A spring rather than an easing curve for two reasons: it whips into place and
 * settles without a bounce in a way a fixed curve can't, and it exposes a
 * per-frame velocity — which is what the motion blur is scaled by. Lives in its
 * own module so the settle budget stays checkable while the feel is tuned.
 */

const STIFFNESS = 600;
// Just under critical damping (2*sqrt(STIFFNESS) ~= 49): sharp, no bounce.
const DAMPING = 43;
const MAX_SPEED = 26;

/** How far a digit travels per roll, in em — one slot of the wheel. */
export const ROLL_TRAVEL = 0.85;

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

export class RollSpring {
  /** 1 = a full slot away from home, 0 = settled. */
  pos = 0;
  vel = 0;
  /** 1 rolls the new digit up from below, -1 brings it down from above. */
  dir: 1 | -1 = 1;
  active = false;

  /**
   * (Re)start the roll. Any speed left over from a roll still in flight is
   * carried rather than zeroed: scrubbing a chart restarts a digit mid-flight,
   * and keeping the momentum is both what makes a fast sweep blur harder and
   * what stops it stuttering back to a standstill between steps.
   */
  start(dir: 1 | -1) {
    this.dir = dir;
    this.vel = clamp(this.vel, -MAX_SPEED, MAX_SPEED);
    this.pos = 1;
    this.active = true;
  }

  /** Advance by dt seconds. Returns false once the roll has settled. */
  step(dt: number): boolean {
    this.vel = clamp(this.vel + (-STIFFNESS * this.pos - DAMPING * this.vel) * dt, -MAX_SPEED, MAX_SPEED);
    this.pos += this.vel * dt;
    if (Math.abs(this.pos) < 0.004 && Math.abs(this.vel) < 0.08) this.reset();
    return this.active;
  }

  reset() {
    this.pos = 0;
    this.vel = 0;
    this.active = false;
  }
}
