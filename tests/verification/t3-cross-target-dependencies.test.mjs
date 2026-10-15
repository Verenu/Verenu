import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('../..', import.meta.url));
const manifest = fileURLToPath(new URL('../../src-tauri/Cargo.toml', import.meta.url));
const targets = [
  'aarch64-apple-darwin',
  'aarch64-linux-android',
  'x86_64-pc-windows-msvc',
];

test('T3 WebSocket direct dependencies resolve on macOS, Android, and Windows', () => {
  let macosMetadata;
  for (const target of targets) {
    const metadata = JSON.parse(execFileSync(
      'cargo',
      ['metadata', '--format-version', '1', '--locked', '--manifest-path', manifest, '--filter-platform', target],
      { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, timeout: 180_000 },
    ));
    if (target === 'aarch64-apple-darwin') macosMetadata = metadata;
    const app = metadata.packages.find(pkg => pkg.name === 'verenu');
    assert.ok(app, 'Cargo metadata includes the Verenu package');
    const appNode = metadata.resolve.nodes.find(node => node.id === app.id);
    assert.ok(appNode, `Cargo resolves Verenu for ${target}`);

    for (const name of ['futures-util', 'tokio-tungstenite']) {
      const declaration = app.dependencies.find(dependency => dependency.name === name && dependency.kind === null);
      assert.ok(declaration, `${name} is a direct dependency`);
      assert.equal(declaration.target, null, `${name} is not restricted to Linux`);
      const resolvedName = name.replaceAll('-', '_');
      assert.ok(appNode.deps.some(dependency => dependency.name === resolvedName), `${name} resolves for ${target}`);
    }

    const websocket = app.dependencies.find(dependency => dependency.name === 'tokio-tungstenite');
    assert.ok(websocket.features.includes('rustls-tls-webpki-roots'));

    if (target === 'aarch64-linux-android') {
      const reqwest = app.dependencies.find(dependency => dependency.name === 'reqwest'
        && dependency.target === 'cfg(target_os = "android")');
      assert.ok(reqwest?.features.includes('rustls-tls'));
      assert.ok(!reqwest?.features.includes('native-tls'));
      assert.ok(reqwest, 'Android keeps its target-specific rustls reqwest configuration');
    }
  }

  const nativeTlsReqwest = macosMetadata.packages.find(pkg => pkg.name === 'verenu').dependencies.find(dependency => dependency.name === 'reqwest'
    && dependency.target === 'cfg(not(target_os = "android"))');
  assert.ok(nativeTlsReqwest?.features.includes('native-tls'), 'desktop keeps its existing native-TLS reqwest configuration');
});
