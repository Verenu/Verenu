import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { syncBuildIdentityFile } from '../../scripts/verification/build-identity.mjs';

test('build identity input changes only when the source fingerprint changes', async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-build-identity-'));
  const file = path.join(directory, 'src-tauri', '.verenu-build-identity');
  const first = 'a'.repeat(64);
  const second = 'b'.repeat(64);
  try {
    assert.equal(await syncBuildIdentityFile(file, first), true);
    assert.equal(await fs.readFile(file, 'utf8'), `${first}\n`);
    assert.equal(await syncBuildIdentityFile(file, first), false);
    assert.equal(await syncBuildIdentityFile(file, second), true);
    assert.equal(await fs.readFile(file, 'utf8'), `${second}\n`);
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
});

test('Cargo build script watches the dev-session source identity input and environment', async () => {
  const buildScript = await fs.readFile(new URL('../../src-tauri/build.rs', import.meta.url), 'utf8');
  assert.match(buildScript, /cargo:rerun-if-env-changed=VERENU_BUILD_FINGERPRINT/);
  assert.match(buildScript, /cargo:rerun-if-env-changed=VERENU_BUILD_FINGERPRINT_FILE/);
  assert.match(buildScript, /cargo:rerun-if-changed=\{fingerprint_file\}/);
});
