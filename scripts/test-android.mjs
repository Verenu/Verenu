import path from 'node:path';
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';
import { root } from './verification/identity.mjs';
import { summarizeAndroidReports } from './verification/android-summary.mjs';

const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
if (!sdk || !fs.existsSync(sdk)) {
  console.error('Android verification incomplete: set ANDROID_HOME to an installed Android SDK.');
  process.exit(2);
}
function check(command, args, cwd = root) {
  const result = spawnSync(command, args, { cwd, env: process.env, stdio: 'inherit', timeout: 900_000, shell: process.platform === 'win32' && command.endsWith('.bat') });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
}
check(process.execPath, ['scripts/android-sync.mjs', '--sources-only']);
// Tauri's Rust build scripts generate the framework Kotlin classes and Gradle
// dependency files. Compile a real debug APK first rather than inventing stubs.
check(process.platform === 'win32' ? 'npx.cmd' : 'npx', ['tauri', 'android', 'build', '--debug', '--target', 'aarch64', '--apk']);
const directory = path.join(root, 'src-tauri/gen/android');
const results = path.join(directory, 'app/build/test-results/testUniversalDebugUnitTest');
fs.rmSync(results, { recursive: true, force: true });
check(process.platform === 'win32' ? 'gradlew.bat' : './gradlew', [':app:compileUniversalDebugKotlin', ':app:testUniversalDebugUnitTest', '-x', 'rustBuildUniversalDebug', '--no-daemon'], directory);
const expected = fs.readdirSync(path.join(root, 'src-tauri/android/tests/com/verenu/app')).filter(file => file.endsWith('Test.kt')).map(file => `com.verenu.app.${file.slice(0, -3)}`);
const reports = fs.existsSync(results) ? fs.readdirSync(results).filter(file => file.endsWith('.xml')).map(file => fs.readFileSync(path.join(results, file), 'utf8')) : [];
const summary = summarizeAndroidReports(reports, expected);
if (summary.status !== 'passed') {
  console.error(`Android verification ${summary.status}: required suites must execute without skips.`);
  process.exit(summary.status === 'failed' ? 1 : 2);
}
console.log(`Executed ${summary.executed} Android native regression cases.`);
console.log('Android debug APK compilation and JVM regressions passed. Packaged inference runtimes and device behavior were not tested.');
