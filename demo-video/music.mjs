// Synthesizes the demo's 60-second music bed from scratch: no samples, no
// third-party audio. Output is a 44.1 kHz 16-bit stereo WAV.
//
// 100 BPM, 25 bars of 2.4 s. Bars 1-2 are a soft pad intro; drums and the
// arpeggio enter at 4.8 s (the first product shot); the last two bars drop back
// to the pad and a held final chord that rings out under the end card.
import { writeFileSync } from 'node:fs';

const out = process.argv[2] || 'music.wav';
const RATE = 44100;
const SECONDS = 60;
const N = RATE * SECONDS;
const BEAT = 0.6;
const BAR = BEAT * 4;
const L = new Float32Array(N);
const R = new Float32Array(N);

// Deterministic noise so every build renders the same file.
let seed = 0x2b2422;
const noise = () => {
  seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
  return ((seed >>> 0) / 0xffffffff) * 2 - 1;
};
const hz = (midi) => 440 * 2 ** ((midi - 69) / 12);

// Cmaj9 - Am7 - Fmaj7 - G6sus, one chord per bar.
const progression = [
  { bass: 36, notes: [48, 55, 59, 62, 64] },
  { bass: 33, notes: [45, 52, 55, 60, 64] },
  { bass: 29, notes: [41, 48, 52, 57, 60] },
  { bass: 31, notes: [43, 50, 55, 60, 62] },
];

function add(buffer, start, samples) {
  const offset = Math.round(start * RATE);
  for (let i = 0; i < samples.length && offset + i < N; i++) buffer[offset + i] += samples[i];
}

function pad(freq, length, gain, pan) {
  const n = Math.round(length * RATE);
  const out = new Float32Array(n);
  const attack = 0.7 * RATE;
  const release = 1.1 * RATE;
  for (let i = 0; i < n; i++) {
    const t = i / RATE;
    const env = Math.min(1, i / attack) * Math.min(1, (n - i) / release);
    const vib = 1 + 0.0025 * Math.sin(2 * Math.PI * 4.6 * t);
    let s = 0;
    // Two slightly detuned voices with a few soft harmonics.
    for (const detune of [0.997, 1.003]) {
      const f = freq * detune * vib;
      s += Math.sin(2 * Math.PI * f * t) + 0.32 * Math.sin(4 * Math.PI * f * t) + 0.12 * Math.sin(6 * Math.PI * f * t);
    }
    out[i] = s * env * gain;
  }
  return { out, pan };
}

function pluck(freq, gain) {
  const n = Math.round(0.9 * RATE);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / RATE;
    const env = Math.min(1, i / 60) * Math.exp(-t * 6.5);
    out[i] = (Math.sin(2 * Math.PI * freq * t) + 0.25 * Math.sin(4 * Math.PI * freq * t) * Math.exp(-t * 12)) * env * gain;
  }
  return out;
}

function kick(gain) {
  const n = Math.round(0.4 * RATE);
  const out = new Float32Array(n);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const t = i / RATE;
    phase += (2 * Math.PI * (48 + 90 * Math.exp(-t * 28))) / RATE;
    out[i] = Math.sin(phase) * Math.exp(-t * 9) * gain;
  }
  return out;
}

function hat(gain, decay = 70) {
  const n = Math.round(0.12 * RATE);
  const out = new Float32Array(n);
  let prev = 0;
  let low = 0;
  for (let i = 0; i < n; i++) {
    const white = noise();
    const high = white - prev; // crude high-pass
    prev = white;
    low += 0.35 * (high - low); // gentle low-pass keeps it from hissing
    out[i] = low * Math.exp(-(i / RATE) * decay) * gain;
  }
  return out;
}

function snap(gain) {
  const n = Math.round(0.25 * RATE);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / RATE;
    out[i] = (noise() * 0.4 * Math.exp(-t * 26) + Math.sin(2 * Math.PI * 190 * t) * 0.5 * Math.exp(-t * 30)) * gain;
  }
  return out;
}

function bassNote(freq, length, gain) {
  const n = Math.round(length * RATE);
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const t = i / RATE;
    const env = Math.min(1, i / 400) * Math.exp(-t * 1.6) * Math.min(1, (n - i) / 2000);
    out[i] = (Math.sin(2 * Math.PI * freq * t) + 0.2 * Math.sin(4 * Math.PI * freq * t)) * env * gain;
  }
  return out;
}

const BARS = Math.ceil(SECONDS / BAR);
const groove = (bar) => bar >= 2 && bar < 23;

for (let bar = 0; bar < BARS; bar++) {
  const start = bar * BAR;
  const chord = progression[bar % progression.length];
  const finalBar = bar === 23;
  if (bar > 23) break;
  const padLength = finalBar ? SECONDS - start : BAR + 0.9;

  chord.notes.forEach((note, k) => {
    const { out, pan } = pad(hz(note), padLength, 0.022, (k / (chord.notes.length - 1)) * 2 - 1);
    const left = new Float32Array(out.length);
    const right = new Float32Array(out.length);
    for (let i = 0; i < out.length; i++) {
      left[i] = out[i] * (1 - pan * 0.45);
      right[i] = out[i] * (1 + pan * 0.45);
    }
    add(L, start, left);
    add(R, start, right);
  });

  if (finalBar) {
    const b = bassNote(hz(chord.bass + 12), SECONDS - start, 0.16);
    add(L, start, b); add(R, start, b);
    continue;
  }

  if (!groove(bar)) continue;
  const fill = bar % 4 === 3;
  for (let beat = 0; beat < 4; beat++) {
    const t = start + beat * BEAT;
    const k = kick(beat === 0 || beat === 2 ? 0.5 : 0.0);
    add(L, t, k); add(R, t, k);
    if (beat === 1 || beat === 3) { const s = snap(0.14); add(L, t, s); add(R, t, s.map((v) => v * 0.9)); }
    for (let eighth = 0; eighth < 2; eighth++) {
      const h = hat(eighth ? 0.04 : 0.028);
      add(eighth ? R : L, t + eighth * BEAT / 2, h);
    }
    if (fill && beat === 3) { const h = hat(0.035, 40); add(L, t + BEAT * 0.75, h); add(R, t + BEAT * 0.75, h); }
  }
  // Bass on beats 1 and 3 with a pickup.
  const root = hz(chord.bass + 12);
  for (const [beat, len] of [[0, 1.1], [2, 0.7], [3.5, 0.28]]) {
    const b = bassNote(beat === 3.5 ? root * 1.5 : root, len, 0.2);
    add(L, start + beat * BEAT, b); add(R, start + beat * BEAT, b);
  }
  // Sixteenth-note arpeggio, up an octave, alternating sides.
  const arp = [0, 2, 3, 4, 3, 2, 1, 2];
  for (let step = 0; step < 16; step++) {
    if (step % 4 === 3 && bar % 2 === 0) continue;
    const note = chord.notes[arp[step % arp.length]] + 12;
    const p = pluck(hz(note), 0.05 * (step % 4 === 0 ? 1 : 0.7));
    add(step % 2 ? R : L, start + step * BEAT / 4, p);
    add(step % 2 ? L : R, start + step * BEAT / 4 + 0.012, p.map((v) => v * 0.45));
  }
}

// Short stereo feedback-delay reverb.
function reverb(src, other, delays, feedback, mix) {
  const out = new Float32Array(N);
  for (const d of delays) {
    const delay = Math.round(d * RATE);
    const line = new Float32Array(N);
    for (let i = 0; i < N; i++) {
      const input = (src[i] * 0.7 + other[i] * 0.3) + (i >= delay ? line[i - delay] * feedback : 0);
      line[i] = input;
      if (i >= delay) out[i] += line[i - delay] * mix / delays.length;
    }
  }
  return out;
}
const wetL = reverb(L, R, [0.0297, 0.0371, 0.0411, 0.0437], 0.78, 0.55);
const wetR = reverb(R, L, [0.0311, 0.0357, 0.0423, 0.0451], 0.78, 0.55);

// Mix, fade, soft-clip, normalize.
let peak = 0;
for (let i = 0; i < N; i++) {
  const t = i / RATE;
  const fade = Math.min(1, t / 1.2) * Math.min(1, (SECONDS - t) / 2.5);
  L[i] = Math.tanh((L[i] + wetL[i]) * 1.1) * fade;
  R[i] = Math.tanh((R[i] + wetR[i]) * 1.1) * fade;
  peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
}
const gain = 0.89 / peak; // about -1 dBFS

const data = Buffer.alloc(N * 4);
for (let i = 0; i < N; i++) {
  data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, L[i] * gain)) * 32767), i * 4);
  data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, R[i] * gain)) * 32767), i * 4 + 2);
}
const header = Buffer.alloc(44);
header.write('RIFF', 0); header.writeUInt32LE(36 + data.length, 4); header.write('WAVE', 8);
header.write('fmt ', 12); header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(2, 22);
header.writeUInt32LE(RATE, 24); header.writeUInt32LE(RATE * 4, 28); header.writeUInt16LE(4, 32); header.writeUInt16LE(16, 34);
header.write('data', 36); header.writeUInt32LE(data.length, 40);
writeFileSync(out, Buffer.concat([header, data]));
console.log(`wrote ${out}`);
