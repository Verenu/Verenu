export const BARS = 12;
export const BAR_MIN_H = 3;
export const BAR_MAX_H = 16;

export const ENVELOPE_WINDOW_MS = 10;

const AGE_STEP_MS = 45;
const HALF_SPAN = (BARS - 1) / 2;

export const HISTORY_SPAN_MS = HALF_SPAN * AGE_STEP_MS;

const RING_MS = HISTORY_SPAN_MS + 500;
const RING_LEN = Math.ceil(RING_MS / ENVELOPE_WINDOW_MS);

const TARGET_LATENCY_MS = 70;
const RESYNC_RATE = 0.06;

const SILENT_DB = -90;

export function toDb(linear: number): number {
  return linear > 0 ? 20 * Math.log10(linear) : SILENT_DB;
}

const NOISE_RISE_TAU_MS = 8000;
const NOISE_FALL_TAU_MS = 300;
const NOISE_TRACK_BAND_DB = 4;
const NOISE_SEED_MS = 400;

const NOISE_DEV_MAX_DB = 4;
const NOISE_DEV_TAU_MS = 900;
const NOISE_DEV_K = 2.15;
const NOISE_MIN_MARGIN_DB = 5;
const MIN_WINDOW_DB = 18;

const REF_RISE_TAU_MS = 900;
const REF_FALL_TAU_MS = 5000;
const HEADROOM_DB = 6;
const REF_ACTIVE_MARGIN_DB = 6;

export function mapWindow(db: number, loDb: number, hiDb: number): number {
  const span = Math.max(hiDb - loDb, 1);
  const n = (db - loDb) / span;
  return Math.min(Math.max(n, 0), 1);
}

const GATE_OPEN_TAU_MS = 45;
const GATE_CLOSE_TAU_MS = 400;
const GATE_LO = 0.06;
const GATE_HI = 0.30;
const GATE_FLOOR = 0.16;

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(Math.max((x - edge0) / (edge1 - edge0), 0), 1);
  return t * t * (3 - 2 * t);
}

const MIN_CUTOFF_HZ = 0.9;
const BETA_UP = 6;
const BETA_DOWN = 1.4;
const DERIV_CUTOFF_HZ = 1;

function alphaFor(cutoffHz: number, dtSec: number): number {
  const tau = 1 / (2 * Math.PI * cutoffHz);
  return 1 / (1 + tau / dtSec);
}

const IDLE_TAU_MS = 350;
const IDLE_AMPL = 0.055;

const CARRIER_TAU_MS = 260;
const DETAIL_GAIN = 4.3;
const OUTPUT_SOFT_KNEE = 0.12;

const SPRING_OMEGA = 42;

const EDGE_TAPER = 0.72;
const barTaper: number[] = Array.from({ length: BARS }, (_, i) => {
  const d = Math.abs(i - HALF_SPAN) / HALF_SPAN;
  return 1 - d * (1 - EDGE_TAPER);
});
const barAge: number[] = Array.from(
  { length: BARS },
  (_, i) => Math.abs(i - HALF_SPAN) * AGE_STEP_MS
);

const dtSec = ENVELOPE_WINDOW_MS / 1000;
const kNoiseUp = 1 - Math.exp(-ENVELOPE_WINDOW_MS / NOISE_RISE_TAU_MS);
const kNoiseDown = 1 - Math.exp(-ENVELOPE_WINDOW_MS / NOISE_FALL_TAU_MS);
const kNoiseDev = 1 - Math.exp(-ENVELOPE_WINDOW_MS / NOISE_DEV_TAU_MS);
const kGateOpen = 1 - Math.exp(-ENVELOPE_WINDOW_MS / GATE_OPEN_TAU_MS);
const kGateClose = 1 - Math.exp(-ENVELOPE_WINDOW_MS / GATE_CLOSE_TAU_MS);
const kCarrier = 1 - Math.exp(-ENVELOPE_WINDOW_MS / CARRIER_TAU_MS);
const kIdle = 1 - Math.exp(-ENVELOPE_WINDOW_MS / IDLE_TAU_MS);
const kRefUp = 1 - Math.exp(-ENVELOPE_WINDOW_MS / REF_RISE_TAU_MS);
const kRefDown = 1 - Math.exp(-ENVELOPE_WINDOW_MS / REF_FALL_TAU_MS);
const aD = alphaFor(DERIV_CUTOFF_HZ, dtSec);

export interface PillVisualizer {
  pushEnvelope(samples: ArrayLike<number>): void;
  step(nowMs: number): number[];
  reset(): void;
}

export function createPillVisualizer(): PillVisualizer {
  const ring = new Float32Array(RING_LEN);
  let written = 0;
  let refDb = 0;
  let noiseFloorDb = SILENT_DB;
  let noiseDevDb = 3;
  let seedMinDb = Infinity;
  let seedMs = 0;
  let refSeeded = false;
  let justSeeded = false;
  let smoothed = 0;
  let smoothedDeriv = 0;
  let gateCtl = 0;
  let idleSlow = 0;
  let carrier = 0;
  let readHeadMs = 0;
  let playheadSeeded = false;
  let lastFrameAt = 0;

  const barLevel = new Float32Array(BARS);
  const barVel = new Float32Array(BARS);

  const writeHeadMs = () => written * ENVELOPE_WINDOW_MS;

  const sampleAt = (tMs: number): number => {
    const pos = tMs / ENVELOPE_WINDOW_MS;
    const oldest = Math.max(0, written - RING_LEN + 1);
    if (pos <= oldest) return ring[oldest % RING_LEN] ?? 0;
    const newest = written - 1;
    if (pos >= newest) return newest >= 0 ? ring[newest % RING_LEN] : 0;
    const i0 = Math.floor(pos);
    const frac = pos - i0;
    const a = ring[i0 % RING_LEN];
    const b = ring[(i0 + 1) % RING_LEN];
    return a + (b - a) * frac;
  };

  return {
    pushEnvelope(samples: ArrayLike<number>) {
      for (let i = 0; i < samples.length; i++) {
        const db = toDb(samples[i]);

        const acquiring = seedMs < NOISE_SEED_MS;
        if (acquiring) {
          seedMs += ENVELOPE_WINDOW_MS;
          if (db < seedMinDb) seedMinDb = db;
          noiseFloorDb = seedMinDb;
        }
        const nearFloor = db < noiseFloorDb + NOISE_TRACK_BAND_DB;
        if (!acquiring) {
          noiseFloorDb += (db - noiseFloorDb) * (db < noiseFloorDb ? kNoiseDown : kNoiseUp);
        }
        if (nearFloor) {
          noiseDevDb += (Math.abs(db - noiseFloorDb) - noiseDevDb) * kNoiseDev;
          if (noiseDevDb > NOISE_DEV_MAX_DB) noiseDevDb = NOISE_DEV_MAX_DB;
        }

        const loDb =
          noiseFloorDb + Math.max(NOISE_MIN_MARGIN_DB, NOISE_DEV_K * noiseDevDb);
        const hiDb = refSeeded
          ? Math.max(refDb + HEADROOM_DB, loDb + MIN_WINDOW_DB)
          : loDb + MIN_WINDOW_DB;
        const level = mapWindow(db, loDb, hiDb);

        if (db > loDb + REF_ACTIVE_MARGIN_DB) {
          if (!refSeeded) {
            refDb = Math.max(db, loDb + MIN_WINDOW_DB - HEADROOM_DB);
            refSeeded = true;
            justSeeded = true;
          } else {
            refDb += (db - refDb) * (db > refDb ? kRefUp : kRefDown);
          }
        }

        const rawDeriv = (level - smoothed) / dtSec;
        smoothedDeriv += aD * (rawDeriv - smoothedDeriv);
        const beta = smoothedDeriv >= 0 ? BETA_UP : BETA_DOWN;
        const cutoff = MIN_CUTOFF_HZ + beta * Math.abs(smoothedDeriv);

        if (justSeeded) {
          smoothed = level;
          carrier = level;
          gateCtl = level;
          justSeeded = false;
        } else {
          smoothed += alphaFor(cutoff, dtSec) * (level - smoothed);
          gateCtl += (level - gateCtl) * (level > gateCtl ? kGateOpen : kGateClose);
          carrier += (smoothed - carrier) * kCarrier;
        }

        const gate = GATE_FLOOR + (1 - GATE_FLOOR) * smoothstep(GATE_LO, GATE_HI, gateCtl);

        idleSlow += (mapWindow(db, noiseFloorDb, loDb) - idleSlow) * kIdle;

        const detail = (smoothed - carrier) * DETAIL_GAIN * carrier * gate;
        const value = (carrier + detail) * gate + idleSlow * IDLE_AMPL;
        const bounded = Math.min(Math.max(value, 0), 1);
        ring[written % RING_LEN] = bounded * (1 - OUTPUT_SOFT_KNEE * bounded);
        written++;
      }
    },

    step(nowMs: number): number[] {
      if (!lastFrameAt) lastFrameAt = nowMs;
      const dt = Math.min(nowMs - lastFrameAt, 50);
      lastFrameAt = nowMs;

      readHeadMs += dt;

      const desired = writeHeadMs() - TARGET_LATENCY_MS;
      if (!playheadSeeded) {
        playheadSeeded = true;
        readHeadMs = Math.max(0, desired);
      } else {
        readHeadMs += (desired - readHeadMs) * RESYNC_RATE;
      }
      if (readHeadMs > writeHeadMs()) readHeadMs = writeHeadMs();

      const dts = dt / 1000;
      const decay = Math.exp(-SPRING_OMEGA * dts);
      const next: number[] = new Array(BARS);
      for (let i = 0; i < BARS / 2; i++) {
        const target = sampleAt(readHeadMs - barAge[i]) * barTaper[i];
        const y0 = barLevel[i] - target;
        const b = barVel[i] + SPRING_OMEGA * y0;
        const y = (y0 + b * dts) * decay;
        barLevel[i] = target + y;
        barVel[i] = (b - SPRING_OMEGA * (y0 + b * dts)) * decay;
        const level = Math.min(Math.max(barLevel[i], 0), 1);
        next[i] = BAR_MIN_H + level * (BAR_MAX_H - BAR_MIN_H);
        next[BARS - 1 - i] = next[i];
      }
      return next;
    },

    reset() {
      ring.fill(0);
      written = 0;
      refDb = 0;
      noiseFloorDb = SILENT_DB;
      noiseDevDb = 3;
      seedMinDb = Infinity;
      seedMs = 0;
      refSeeded = false;
      justSeeded = false;
      smoothed = 0;
      smoothedDeriv = 0;
      gateCtl = 0;
      idleSlow = 0;
      carrier = 0;
      readHeadMs = 0;
      playheadSeeded = false;
      lastFrameAt = 0;
      barLevel.fill(0);
      barVel.fill(0);
    },
  };
}
