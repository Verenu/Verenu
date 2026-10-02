#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const index = process.argv.indexOf('--out');
const output = index < 0 ? path.join(os.homedir(), '.cache', 'verenu', 'test-audio') : path.resolve(process.argv[index + 1]);
const cases = JSON.parse(await fs.readFile(path.join(root, 'tests', 'fixtures', 'dev-audio.json'), 'utf8'));
await fs.mkdir(output, { recursive: true });
const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-tts-'));
try {
  const manifest = [];
  for (const item of cases) {
    const raw = path.join(temp, 'speech.wav');
    try { execFileSync('espeak-ng', ['-v', 'en-us', '-s', '145', '-w', raw, item.text], { stdio: 'ignore', timeout: 30_000 }); }
    catch { throw new Error('Install espeak-ng and ffmpeg to generate offline synthetic fixtures, or import ElevenLabs audio through the dev panel.'); }
    const destination = path.join(output, `${item.name}.wav`);
    execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', '-i', raw, '-ar', '16000', '-ac', '1', '-c:a', 'pcm_s16le', destination], { stdio: 'ignore', timeout: 30_000 });
    const bytes = await fs.readFile(destination);
    manifest.push({ ...item, file: `${item.name}.wav`, sha256: createHash('sha256').update(bytes).digest('hex'), generator: 'espeak-ng' });
  }
  const silence = path.join(output, 'silence.wav');
  execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'anullsrc=r=16000:cl=mono', '-t', '2', '-c:a', 'pcm_s16le', silence], { stdio: 'ignore', timeout: 30_000 });
  manifest.push({ name: 'silence', file: 'silence.wav', expected: 'rejected-before-provider', sha256: createHash('sha256').update(await fs.readFile(silence)).digest('hex') });
  await fs.writeFile(path.join(output, 'manifest.json'), JSON.stringify(manifest, null, 2));
  console.log(`Generated ${manifest.length} synthetic fixtures in ${output}`);
} finally { await fs.rm(temp, { recursive: true, force: true }); }
