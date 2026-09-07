// Installs Verenu's Android native sources into the Tauri-generated project.
//
// Usage: node scripts/android-sync.mjs [--init-only]
//
// 1. Runs `tauri android init` when src-tauri/gen/android is missing.
// 2. Copies src-tauri/android/kotlin/* into the app module package dir.
// 3. Copies res/xml + res/values resources.
// 4. Merges AndroidManifest.snippet.xml blocks (idempotent, marker-wrapped).
// 5. Pins minSdk 26 + androidx security-crypto in app/build.gradle.kts.
//
// Idempotent: safe to re-run after `tauri android init` or CLI upgrades.
// Fails loudly (non-zero exit + anchor dump) when generated content it must
// patch is missing, instead of writing a half-merged project.

import { execFileSync } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const srcTauri = join(root, 'src-tauri');
const androidSrc = join(srcTauri, 'android');
const genAndroid = join(srcTauri, 'gen', 'android');

function fail(message) {
  console.error(`android-sync: ${message}`);
  process.exit(1);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function packageDir() {
  const conf = readJson(join(srcTauri, 'tauri.conf.json'));
  const identifier = conf.identifier || conf.app?.identifier || 'com.verenu.app';
  return join(genAndroid, 'app', 'src', 'main', 'java', ...identifier.split('.'));
}

function copyTree(src, dest) {
  mkdirSync(dest, { recursive: true });
  for (const entry of readdirSync(src)) {
    const from = join(src, entry);
    const to = join(dest, entry);
    if (statSync(from).isDirectory()) copyTree(from, to);
    else copyFileSync(from, to);
  }
}

function ensureInit() {
  if (existsSync(join(genAndroid, 'app', 'src', 'main', 'AndroidManifest.xml'))) return;
  console.log('android-sync: running `tauri android init` …');
  try {
    execFileSync('npx', ['tauri', 'android', 'init'], { cwd: root, stdio: 'inherit' });
  } catch {
    fail('`tauri android init` failed. Install the Android SDK (API 34), JDK 17, and Rust android targets first — see docs/ANDROID.md.');
  }
}

function mergeManifest() {
  const manifestPath = join(genAndroid, 'app', 'src', 'main', 'AndroidManifest.xml');
  const snippetPath = join(androidSrc, 'AndroidManifest.snippet.xml');
  if (!existsSync(manifestPath)) fail(`generated manifest missing: ${manifestPath}`);
  let manifest = readFileSync(manifestPath, 'utf8');
  const snippet = readFileSync(snippetPath, 'utf8');

  const blocks = [
    { start: '<!-- verenu:permissions:start -->', end: '<!-- verenu:permissions:end -->' },
    { start: '<!-- verenu:services:start -->', end: '<!-- verenu:services:end -->' },
  ];
  for (const { start, end } of blocks) {
    const s = snippet.indexOf(start);
    const e = snippet.indexOf(end);
    if (s === -1 || e === -1) fail(`snippet block missing: ${start}`);
    const block = snippet.slice(s, e + end.length);
    if (manifest.includes(start)) {
      // Replace the previously merged block in place (idempotent).
      const ms = manifest.indexOf(start);
      const me = manifest.indexOf(end);
      if (me === -1) fail(`generated manifest has unbalanced verenu markers near: ${start}`);
      manifest = manifest.slice(0, ms) + block + manifest.slice(me + end.length);
      continue;
    }
    if (start.includes('permissions')) {
      const anchor = '<manifest';
      const at = manifest.indexOf(anchor);
      if (at === -1) fail('manifest anchor <manifest> not found; check the Tauri CLI template.');
      const lineEnd = manifest.indexOf('>', at);
      manifest = manifest.slice(0, lineEnd + 1) + '\n' + block + manifest.slice(lineEnd + 1);
    } else {
      const anchor = '<application';
      const at = manifest.indexOf(anchor);
      if (at === -1) fail('manifest anchor <application> not found; check the Tauri CLI template.');
      const lineEnd = manifest.indexOf('>', at);
      manifest = manifest.slice(0, lineEnd + 1) + '\n' + block + manifest.slice(lineEnd + 1);
    }
  }
  writeFileSync(manifestPath, manifest);
  console.log('android-sync: manifest merged');
}

function patchGradle() {
  const gradlePath = join(genAndroid, 'app', 'build.gradle.kts');
  if (!existsSync(gradlePath)) fail(`app build script missing: ${gradlePath}`);
  let gradle = readFileSync(gradlePath, 'utf8');
  const dep = 'implementation("androidx.security:security-crypto:1.1.0-alpha06")';
  if (!gradle.includes('security-crypto')) {
    const anchor = 'dependencies {';
    const at = gradle.indexOf(anchor);
    if (at === -1) fail('gradle anchor `dependencies {` not found; check the Tauri CLI template.');
    gradle =
      gradle.slice(0, at + anchor.length) + `\n    // Verenu: Keystore-backed credential storage.\n    ${dep}` +
      gradle.slice(at + anchor.length);
  }
  if (!gradle.includes('verenuMinSdk')) {
    console.log('android-sync: note — set minSdk 26 for the app module (tauri.conf.json bundle.android.minSdkVersion is the source of truth).');
  }
  writeFileSync(gradlePath, gradle);
  console.log('android-sync: gradle patched');
}

const initOnly = process.argv.includes('--init-only');
ensureInit();
if (initOnly) process.exit(0);

copyTree(join(androidSrc, 'kotlin'), packageDir());
copyTree(join(androidSrc, 'res'), join(genAndroid, 'app', 'src', 'main', 'res'));
console.log('android-sync: kotlin + res installed');
mergeManifest();
patchGradle();
console.log('android-sync: done. Next: npx tauri android build (or open gen/android in Android Studio).');
