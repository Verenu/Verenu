// Renders stage.html to video frames and encodes the final MP4.
//
//   node demo-video/render.mjs <assetsDir> <out.mp4>          full render
//   node demo-video/render.mjs <assetsDir> <outDir> --stills 2,9,20
//
// <assetsDir> must contain ui/*.png, pill-frames/*.png, and music.wav (see
// build.sh). Frames are piped straight into ffmpeg; nothing large is written
// besides the output file.
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import { createReadStream, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readDemoVersion } from './release-version.mjs';

const [assetsArg, outArg, flag, stillsArg] = process.argv.slice(2);
if (!assetsArg || !outArg) {
  console.error('usage: render.mjs <assetsDir> <out.mp4 | outDir --stills t1,t2>');
  process.exit(2);
}
const assets = resolve(assetsArg);
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const version = readDemoVersion(repo);
const FPS = 30;
const DURATION = 60;

const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.png': 'image/png',
  '.woff2': 'font/woff2', '.woff': 'font/woff', '.svg': 'image/svg+xml' };
const server = createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const [root, rel] = path.startsWith('/__assets/') ? [assets, path.slice('/__assets/'.length)] : [repo, path.slice(1)];
  const file = normalize(join(root, rel));
  if (!file.startsWith(root) || !existsSync(file) || !statSync(file).isFile()) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(file)] || 'application/octet-stream' });
  createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const pillFrames = readdirSync(join(assets, 'pill-frames')).filter((f) => f.endsWith('.png')).length;
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/google-chrome-stable' });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
page.on('pageerror', (e) => console.error('pageerror', e.message));
await page.goto(`${base}/demo-video/stage.html?pillFrames=${pillFrames}&version=${encodeURIComponent(version)}`);
await page.evaluate(() => window.demo.load('/__assets'));

const frameAt = async (t) => {
  await page.evaluate((t) => window.demo.render(t), t);
  return page.screenshot({ type: 'png' });
};

if (flag === '--stills') {
  mkdirSync(outArg, { recursive: true });
  for (const t of (stillsArg || '2').split(',').map(Number)) {
    const { writeFileSync } = await import('node:fs');
    writeFileSync(join(outArg, `still_${t.toFixed(2)}.png`), await frameAt(t));
  }
  console.log(`stills written to ${outArg}`);
} else {
  const ffmpeg = spawn('ffmpeg', [
    '-y', '-loglevel', 'error',
    '-f', 'image2pipe', '-framerate', String(FPS), '-i', '-',
    '-i', join(assets, 'music.wav'),
    '-map', '0:v', '-map', '1:a',
    '-c:v', 'libx264', '-threads', '4', '-preset', 'slow', '-crf', '17', '-pix_fmt', 'yuv420p', '-tune', 'animation',
    '-af', 'loudnorm=I=-16:TP=-1.5:LRA=11',
    '-c:a', 'aac', '-b:a', '192k', '-ar', '48000',
    '-t', String(DURATION), '-movflags', '+faststart',
    outArg,
  ], { stdio: ['pipe', 'inherit', 'inherit'] });
  const total = FPS * DURATION;
  const started = Date.now();
  for (let f = 0; f < total; f++) {
    const png = await frameAt(f / FPS);
    if (!ffmpeg.stdin.write(png)) await new Promise((r) => ffmpeg.stdin.once('drain', r));
    if (f % 150 === 0) console.log(`frame ${f}/${total} (${((Date.now() - started) / 1000).toFixed(0)} s)`);
  }
  ffmpeg.stdin.end();
  const code = await new Promise((r) => ffmpeg.on('close', r));
  if (code !== 0) throw new Error(`ffmpeg exited with ${code}`);
  console.log(`wrote ${outArg}`);
}
await browser.close();
server.close();
