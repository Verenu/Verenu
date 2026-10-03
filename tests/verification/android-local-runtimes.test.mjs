import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { cmakeArguments, LLAMA_COMMIT, LLAMA_ANDROID_BINARY, ORT_VERSION, RUNTIME_ABIS } from '../../scripts/android-local-runtimes.mjs';

test('Android native runtimes are reproducible and support both shipped ABIs', () => {
  assert.match(LLAMA_COMMIT, /^[a-f0-9]{40}$/);
  assert.equal(ORT_VERSION, '1.24.3');
  assert.deepEqual(RUNTIME_ABIS, ['arm64-v8a', 'x86_64']);
  for (const abi of RUNTIME_ABIS) {
    const args = cmakeArguments('/source', '/build', '/ndk', abi);
    assert.deepEqual(args.slice(0, 2), ['-G', 'Ninja']);
    assert.ok(args.includes(`-DANDROID_ABI=${abi}`));
    assert.ok(args.includes('-DANDROID_PLATFORM=android-28'));
    assert.ok(args.includes('-DANDROID_STL=c++_static'));
    assert.ok(args.includes('-DGGML_NATIVE=OFF'));
    assert.ok(args.includes('-DBUILD_SHARED_LIBS=OFF'));
    assert.ok(args.includes('-DGGML_OPENMP=OFF'));
    assert.ok(args.includes('-DMTMD_VIDEO=OFF'));
    assert.ok(args.includes('-DCMAKE_EXE_LINKER_FLAGS=-Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384'));
  }
  assert.throws(() => cmakeArguments('/s', '/b', '/n', 'armeabi-v7a'), /Unsupported/);
});

test('Cargo launched by Tauri from the repository root uses Android 16 KB linker flags', () => {
  const rootConfig = readFileSync(new URL('../../.cargo/config.toml', import.meta.url), 'utf8');
  const crateConfig = new URL('../../src-tauri/.cargo/config.toml', import.meta.url);
  for (const target of ['aarch64-linux-android', 'x86_64-linux-android']) {
    const targetConfig = rootConfig.split(`[target.${target}]`)[1]?.split('\n[target.')[0] ?? '';
    assert.match(targetConfig, /link-arg=-Wl,-z,max-page-size=16384/);
    assert.match(targetConfig, /link-arg=-Wl,-z,common-page-size=16384/);
    assert.doesNotMatch(targetConfig, /c\+\+_shared|--no-as-needed/);
  }
  assert.equal(existsSync(crateConfig), false, 'Cargo config must be discoverable from the Tauri CLI repository-root working directory');
});

test('Android sync does not package a shared NDK C++ runtime', () => {
  const build = readFileSync(new URL('../../src-tauri/build.rs', import.meta.url), 'utf8');
  const sync = readFileSync(new URL('../../scripts/android-sync.mjs', import.meta.url), 'utf8');
  assert.doesNotMatch(build, /rustc-link-lib=dylib=c\+\+_shared/);
  assert.match(sync, /function removeBundledCppRuntime\(\)/);
  assert.match(sync, /rmSync\(join\(jniRoot, abi, 'libc\+\+_shared\.so'\), \{ force: true \}\)/);
  assert.match(sync, /await syncAndroidLocalRuntimes\(root\);\s+removeBundledCppRuntime\(\);/);
});

test('packaged executable name matches the native resolver', () => {
  const source = readFileSync(new URL('../../src-tauri/src/android/local_ai.rs', import.meta.url), 'utf8');
  assert.ok(source.includes(`pub const LLAMA_BINARY: &str = "${LLAMA_ANDROID_BINARY}";`));
});

test('Android fixture IPC is opt-in and prohibited in release builds', () => {
  const build = readFileSync(new URL('../../src-tauri/build.rs', import.meta.url), 'utf8');
  assert.match(build, /CARGO_FEATURE_ANDROID_LOCAL_TESTING/);
  assert.match(build, /PROFILE.*release/);
  const source = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  assert.match(source, /#\[cfg\(all\(debug_assertions, feature = "android-local-testing", target_os = "android"\)\)\]\s+commands::android_test_local_audio/);
});
