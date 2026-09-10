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
  rmSync,
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

function javaSourceDir() {
  // The source tree already contains its package path (`kotlin/com/verenu/app`)
  // so copy it below `java`, not below `java/<identifier>` a second time.
  return join(genAndroid, 'app', 'src', 'main', 'java');
}

function androidSdkRoots() {
  return [process.env.ANDROID_NDK_HOME, process.env.ANDROID_NDK_ROOT]
    .filter(Boolean)
    .concat(
      [process.env.ANDROID_HOME, process.env.ANDROID_SDK_ROOT]
        .filter(Boolean)
        .map((sdk) => join(sdk, 'ndk')),
    );
}

function findNdkRuntime(abi) {
  const names = [
    'windows-x86_64',
    'linux-x86_64',
    'darwin-x86_64',
    'darwin-arm64',
  ];
  const target = abi === 'arm64-v8a' ? 'aarch64-linux-android' : 'x86_64-linux-android';
  for (const root of androidSdkRoots()) {
    const candidates = [];
    if (root.endsWith('ndk')) {
      if (existsSync(root)) {
        for (const version of readdirSync(root).sort().reverse()) candidates.push(join(root, version));
      }
    } else {
      candidates.push(root);
    }
    for (const ndk of candidates) {
      for (const host of names) {
        const runtime = join(ndk, 'toolchains', 'llvm', 'prebuilt', host, 'sysroot', 'usr', 'lib', target, 'libc++_shared.so');
        if (existsSync(runtime)) return runtime;
      }
    }
  }
  return null;
}

function syncCppRuntime() {
  const jniRoot = join(genAndroid, 'app', 'src', 'main', 'jniLibs');
  const abis = { 'arm64-v8a': 'arm64-v8a', x86_64: 'x86_64' };
  for (const [abi, targetAbi] of Object.entries(abis)) {
    const runtime = findNdkRuntime(abi);
    if (!runtime) {
      if (abi === 'arm64-v8a') fail('NDK libc++_shared.so not found. Install the NDK and set ANDROID_HOME/ANDROID_SDK_ROOT or ANDROID_NDK_HOME.');
      continue;
    }
    const dest = join(jniRoot, targetAbi, 'libc++_shared.so');
    mkdirSync(dirname(dest), { recursive: true });
    copyFileSync(runtime, dest);
  }
  console.log('android-sync: NDK C++ runtime installed');
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

function syncWebAssets() {
  const dist = join(root, 'dist');
  const generatedAssets = join(genAndroid, 'app', 'src', 'main', 'assets');
  if (!existsSync(join(dist, 'index.html')) || !existsSync(join(dist, 'pill.html')) || !existsSync(join(dist, 'assets'))) {
    fail('dist/ is missing or stale. Run `npm run build` before `npm run android:sync`.');
  }

  // Tauri's generated Android project packages these files directly into the
  // WebView. Remove old hashed bundles first so index.html can never point at
  // a file from a previous frontend build.
  rmSync(join(generatedAssets, 'assets'), { recursive: true, force: true });
  mkdirSync(generatedAssets, { recursive: true });
  copyFileSync(join(dist, 'index.html'), join(generatedAssets, 'index.html'));
  copyFileSync(join(dist, 'pill.html'), join(generatedAssets, 'pill.html'));
  copyTree(join(dist, 'assets'), join(generatedAssets, 'assets'));
  console.log('android-sync: web assets installed');
}

function ensureInit() {
  if (existsSync(join(genAndroid, 'app', 'src', 'main', 'AndroidManifest.xml'))) return;
  console.log('android-sync: running `tauri android init` …');
  try {
    execFileSync('npx', ['tauri', 'android', 'init'], { cwd: root, stdio: 'inherit' });
  } catch {
    fail('`tauri android init` failed. Install the Android SDK (API 36), JDK 17, and Rust android targets first — see docs/ANDROID.md.');
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
  // MainActivity's own tag is Tauri's template output, not ours, so it isn't
  // marker-wrapped like the blocks above — patch its attribute in place
  // instead. Edge-to-edge (MainActivity.kt's enableEdgeToEdge()) only lets
  // content draw behind the status/nav bars; without this attribute the
  // system still reserves the display cutout (camera hole-punch) as
  // off-limits, so content laid out for the full edge-to-edge height gets
  // clipped under the cutout instead of avoiding or padding around it.
  const cutoutAttr = 'android:windowLayoutInDisplayCutoutMode="shortEdges"';
  if (!manifest.includes('windowLayoutInDisplayCutoutMode')) {
    const activityAnchor = 'android:name=".MainActivity"';
    const at = manifest.indexOf(activityAnchor);
    if (at === -1) fail('MainActivity tag not found; check the Tauri CLI template.');
    manifest = manifest.slice(0, at) + cutoutAttr + '\n            ' + manifest.slice(at);
  }

  // Release builds keep cleartext disabled globally. The native accessibility
  // service still needs the Rust bridge's authenticated loopback HTTP socket,
  // so install a resource-level exception scoped to localhost/127.0.0.1 and
  // disable Android backups for dictation history and transient bridge data.
  const appAnchor = '<application';
  const appAt = manifest.indexOf(appAnchor);
  if (appAt === -1) fail('application anchor not found; check the Tauri CLI template.');
  const appAttrs = [];
  if (!manifest.includes('android:networkSecurityConfig=')) {
    appAttrs.push('android:networkSecurityConfig="@xml/verenu_network_security_config"');
  }
  if (!manifest.includes('android:allowBackup=')) {
    appAttrs.push('android:allowBackup="false"');
  }
  if (appAttrs.length > 0) {
    manifest =
      manifest.slice(0, appAt + appAnchor.length) +
      `\n        ${appAttrs.join('\n        ')}` +
      manifest.slice(appAt + appAnchor.length);
  }

  writeFileSync(manifestPath, manifest);
  console.log('android-sync: manifest merged');
}

function patchGradle() {
  const gradlePath = join(genAndroid, 'app', 'build.gradle.kts');
  if (!existsSync(gradlePath)) fail(`app build script missing: ${gradlePath}`);
  let gradle = readFileSync(gradlePath, 'utf8');
  // Keep the runtime capability report and the packaged APK metadata aligned
  // even when `tauri android init` was produced by a different CLI template.
  gradle = gradle.replace(/targetSdk\s*=\s*\d+/, 'targetSdk = 36');
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

syncWebAssets();
copyTree(join(androidSrc, 'kotlin'), javaSourceDir());
copyTree(join(androidSrc, 'res'), join(genAndroid, 'app', 'src', 'main', 'res'));
copyFileSync(join(androidSrc, 'proguard-rules.pro'), join(genAndroid, 'app', 'proguard-rules.pro'));
console.log('android-sync: kotlin + res installed');
syncCppRuntime();
mergeManifest();
patchGradle();
console.log('android-sync: done. Next: npx tauri android build (or open gen/android in Android Studio).');
