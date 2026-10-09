import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { root } from '../../scripts/verification/identity.mjs';

test('FoundationModels stages the back-deployment runtime for bundling, dev and tests', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-foundation-runtime-'));
  try {
    const manifest = path.join(temp, 'project');
    const out = path.join(temp, 'target', 'debug', 'build', 'bridge', 'out');
    const bin = path.join(temp, 'tools');
    const swift = path.join(temp, 'toolchain', 'usr', 'bin', 'swiftc');
    const runtime = path.join(temp, 'toolchain', 'usr', 'lib', 'swift-5.5', 'macosx', 'libswift_Concurrency.dylib');
    for (const dir of [manifest, out, bin, path.dirname(swift), path.dirname(runtime)]) fs.mkdirSync(dir, { recursive: true });
    // Fake compiler tools exercise the real Rust build helper's staging and
    // failure paths. They do not prove Mach-O linkage or macOS compatibility.
    fs.writeFileSync(path.join(bin, 'xcrun'), '#!/bin/sh\ncase "$1" in\n--sdk) echo "$VERENU_TEST_SDK";;\n--find) echo "$VERENU_TEST_SWIFT";;\nar) exit 0;;\n*) exit 1;;\nesac\n', { mode: 0o700 });
    fs.writeFileSync(swift, '#!/bin/sh\nwhile [ "$#" -gt 0 ]; do last="$1"; shift; done\n: > "$last"\n', { mode: 0o700 });
    const payload = Buffer.from('synthetic back-deployment runtime');
    fs.writeFileSync(runtime, payload);
    const driver = path.join(temp, 'driver.rs');
    fs.writeFileSync(driver, `#[path = ${JSON.stringify(path.join(root, 'src-tauri/build_support/foundation_models.rs'))}] mod bridge;\nfn main() { bridge::build(); }\n`);
    const executable = path.join(temp, 'driver');
    const compiled = spawnSync('rustc', ['--edition=2021', driver, '-o', executable], { encoding: 'utf8', timeout: 30_000 });
    assert.equal(compiled.status, 0, compiled.stderr);
    const env = { ...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}`, CARGO_CFG_TARGET_OS: 'macos', CARGO_CFG_TARGET_ARCH: 'aarch64', CARGO_MANIFEST_DIR: manifest, OUT_DIR: out, VERENU_TEST_SDK: path.join(temp, 'sdk'), VERENU_TEST_SWIFT: swift };
    const built = spawnSync(executable, [], { env, encoding: 'utf8', timeout: 10_000 });
    assert.equal(built.status, 0, built.stderr);
    const config = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    assert.equal(config.bundle.macOS.minimumSystemVersion, '11.0');
    assert.ok(config.bundle.macOS.frameworks.includes('native/macos/swift-runtime/libswift_Concurrency.dylib'));
    for (const directory of [path.join(manifest, 'native/macos/swift-runtime'), path.join(temp, 'target/debug/swift-runtime'), path.join(temp, 'target/debug/deps/swift-runtime')]) {
      assert.deepEqual(fs.readFileSync(path.join(directory, 'libswift_Concurrency.dylib')), payload);
    }
    assert.ok(built.stdout.includes('cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks'));
    assert.ok(built.stdout.includes('cargo:rustc-link-arg=-Wl,-rpath,@executable_path/swift-runtime'));
    fs.unlinkSync(runtime);
    const missing = spawnSync(executable, [], { env, encoding: 'utf8', timeout: 10_000 });
    assert.notEqual(missing.status, 0);
    assert.match(missing.stderr, /back-deployment runtime is missing/);
    const linux = spawnSync(executable, [], { env: { ...env, CARGO_CFG_TARGET_OS: 'linux' }, encoding: 'utf8', timeout: 10_000 });
    assert.equal(linux.status, 0);
    assert.ok(!linux.stdout.includes('rustc-link-lib'));
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
