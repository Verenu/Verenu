
const STIFFNESS = 600;
const DAMPING = 43;
const MAX_SPEED = 26;

/** How far a digit travels per roll, in em — one slot of the wheel. */
export const ROLL_TRAVEL = 0.85;

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

export class RollSpring {
  pos = 0;
  vel = 0;
  dir: 1 | -1 = 1;
  active = false;

  start(dir: 1 | -1) {
    this.dir = dir;
    this.vel = clamp(this.vel, -MAX_SPEED, MAX_SPEED);
    this.pos = 1;
    this.active = true;
  }

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
