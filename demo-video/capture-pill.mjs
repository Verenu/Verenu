// Captures the real dictation pill (src/PillApp.svelte) as a transparent PNG
// sequence. A stub Tauri bridge feeds it the same events the backend emits
// (pill-state, pill-context, audio-envelope, pill-stage). The pill runs in
// real time; frames are captured as fast as Chrome allows with timestamps and
// resampled to a fixed frame rate. (Fake clocks and seeked animations left the
// composited pill blank on some frames, so this trades a little temporal
// precision for frames that match what the app draws.)
import { chromium } from 'playwright';
import { copyFileSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [baseUrl = 'http://127.0.0.1:47613', outDir = 'pill-frames'] = process.argv.slice(2);
const FPS = 30;
const DURATION_MS = 7600;
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/google-chrome-stable' });
const page = await browser.newPage({ viewport: { width: 420, height: 120 }, deviceScaleFactor: 3 });
page.on('pageerror', (e) => console.error('pageerror', e.message));
await page.addInitScript(() => {
  const callbacks = new Map();
  const handlers = new Map();
  let next = 0;
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'pill' }, currentWebview: { windowLabel: 'pill', label: 'pill' } },
    transformCallback(cb) { const id = ++next; callbacks.set(id, cb); return id; },
    unregisterCallback(id) { callbacks.delete(id); },
    convertFileSrc: (s) => s,
    async invoke(cmd, args) {
      if (cmd === 'plugin:event|listen') {
        handlers.set(args.event, [...(handlers.get(args.event) ?? []), args.handler]);
        return args.handler;
      }
      if (cmd === 'get_setting') return args?.key === 'appearance_mode' ? 'light' : null;
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  const emit = (event, payload) => {
    for (const id of handlers.get(event) ?? []) callbacks.get(id)?.({ event, id: 0, payload });
  };
  window.__ready = () => handlers.has('pill-stage') && handlers.has('audio-envelope');
  // Deterministic pseudo-speech envelope: syllable bursts over a quiet floor.
  const rand = (n) => ((Math.sin(n * 12.9898) * 43758.5453) % 1 + 1) % 1;
  window.__startTimeline = () => {
    const t0 = performance.now();
    window.__t0 = t0;
    const at = (ms, fn) => setTimeout(fn, ms);
    at(500, () => emit('pill-state', 'recording'));
    at(560, () => emit('pill-context', 'Team chat'));
    let sample = 0;
    const feed = setInterval(() => {
      const t = performance.now() - t0;
      if (t < 500 || t > 4600) return;
      const values = [];
      for (let i = 0; i < 5; i++, sample++) {
        const ms = sample * 10;
        if (ms < 400) { values.push(0.002 + 0.001 * rand(sample)); continue; }
        const syllable = Math.max(0, Math.sin(ms / 1000 * Math.PI * 4.2 + Math.sin(ms / 700)));
        const phrase = ms % 1900 < 1600 ? 1 : 0.08;
        values.push(0.003 + 0.42 * syllable * phrase * (0.6 + 0.4 * rand(sample * 3.1)));
      }
      emit('audio-envelope', values);
      emit('audio-level-raw', Math.max(...values) * 0.4);
      if (sample > 50) emit('pill-speech-detected');
    }, 50);
    at(4600, () => { clearInterval(feed); emit('pill-state', 'processing'); emit('pill-stage', 'transcribing'); });
    at(5400, () => emit('pill-stage', 'cleaning'));
    at(6300, () => emit('pill-stage', 'pasting'));
    at(6900, () => emit('pill-state', 'idle'));
  };
});
await page.goto(`${baseUrl}/pill.html`);
await page.addStyleTag({ content: 'html, body { background: transparent !important; }' });
await page.waitForFunction(() => window.__ready());
await page.waitForTimeout(800);

const cdp = await page.context().newCDPSession(page);
await cdp.send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
const rawDir = join(outDir, 'raw');
mkdirSync(rawDir, { recursive: true });
const shots = [];
await page.evaluate(() => window.__startTimeline());
for (;;) {
  const { data } = await cdp.send('Page.captureScreenshot', {
    format: 'png',
    // CDP ignores the emulated device scale; ask for 3x explicitly.
    clip: { x: 0, y: 0, width: 420, height: 120, scale: 3 },
  });
  const t = await page.evaluate(() => performance.now() - window.__t0);
  const file = join(rawDir, `raw_${String(shots.length).padStart(4, '0')}.png`);
  writeFileSync(file, Buffer.from(data, 'base64'));
  shots.push({ t, file });
  if (t > DURATION_MS) break;
}
await browser.close();

// Resample: each output frame takes the latest capture at or before its time.
const frames = Math.round((DURATION_MS / 1000) * FPS);
let j = 0;
for (let f = 0; f < frames; f++) {
  const t = (f * 1000) / FPS;
  while (j + 1 < shots.length && shots[j + 1].t <= t) j++;
  copyFileSync(shots[j].file, join(outDir, `pill_${String(f).padStart(4, '0')}.png`));
}
rmSync(rawDir, { recursive: true });
const fps = (shots.length / (shots.at(-1).t / 1000)).toFixed(1);
console.log(`captured ${shots.length} shots (~${fps} fps), wrote ${frames} frames to ${outDir}`);
