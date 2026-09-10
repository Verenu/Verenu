#!/usr/bin/env node

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (relative) => readFileSync(path.join(repoRoot, relative), 'utf8');
const json = (relative) => JSON.parse(read(relative));
const failures = [];
const assert = (condition, message) => { if (!condition) failures.push(message); };

const production = json('src-tauri/tauri.conf.json');
const development = json('src-tauri/tauri.dev.conf.json');
const windowsDevelopment = json('src-tauri/tauri.dev.windows.conf.json');
const sourcePlist = read('src-tauri/Info.plist');
const runner = read('scripts/tauri-macos-dev-runner.mjs');
const cli = read('scripts/tauri-cli.mjs');
const buildScript = read('src-tauri/build.rs');
const notify = read('src-tauri/src/system/notify.rs');
const relaunch = read('src-tauri/src/commands/permissions.rs');

assert(production.identifier === 'com.verenu.app', 'Production bundle ID must be com.verenu.app.');
assert(production.productName === 'Verenu', 'Production product name must be Verenu.');
assert(production.bundle?.macOS?.bundleName === 'Verenu', 'Production macOS bundle name must be Verenu.');
assert(production.bundle?.macOS?.signingIdentity !== '-', 'Production must not allow ad-hoc signing.');
assert(development.identifier === 'com.verenu.app.dev', 'Development bundle ID must be com.verenu.app.dev.');
assert(development.productName === 'Verenu Development', 'Development product name must be Verenu Development.');
assert(development.bundle?.macOS?.bundleName === 'Verenu Development', 'Development bundle name must be Verenu Development.');
assert(windowsDevelopment.identifier === 'com.verenu.app.dev', 'Windows development must isolate with com.verenu.app.dev.');
assert(windowsDevelopment.productName === undefined, 'Windows development must not rebrand productName to Verenu Development.');
assert(windowsDevelopment.bundle?.macOS === undefined, 'Windows development config must not carry macOS bundle branding.');
assert(
  !notify.includes('Verenu Development.lnk') || notify.includes('obsolete_windows_dev_branded_shortcut'),
  'Windows toast setup must not keep creating a Verenu Development Start Menu shortcut.',
);
assert(
  notify.includes('WINDOWS_TOAST_APP_ID: &str = "com.verenu.app"'),
  'Windows toasts must keep the normal Verenu AUMID branding.',
);
const developmentCapabilities = development.app?.security?.capabilities ?? [];
const windowsDevelopmentCapabilities = windowsDevelopment.app?.security?.capabilities ?? [];
const viteCapability = developmentCapabilities.find(
  (capability) => typeof capability === 'object' && capability.identifier === 'development-vite-main',
);
const windowsViteCapability = windowsDevelopmentCapabilities.find(
  (capability) => typeof capability === 'object' && capability.identifier === 'development-vite-main',
);
const defaultCapability = json('src-tauri/capabilities/default.json');
assert(
  viteCapability?.remote?.urls?.includes('http://127.0.0.1:1420/*'),
  'Development must authorize the exact local Vite origin for main-window IPC.',
);
assert(viteCapability?.local === false, 'The Vite IPC capability must not broaden local packaged-content access.');
assert(
  Array.isArray(viteCapability?.permissions) && viteCapability.permissions.includes('verenu-security:default'),
  'The Vite remote capability must grant plugin permissions, not a bare app ACL default.',
);
assert(
  !viteCapability?.permissions?.includes('default'),
  'The Vite remote capability must not reference a bare app ACL default permission.',
);
assert(
  windowsViteCapability?.remote?.urls?.includes('http://127.0.0.1:1420/*'),
  'Windows development must authorize the exact local Vite origin for main-window IPC.',
);
assert(
  Array.isArray(windowsViteCapability?.permissions)
    && windowsViteCapability.permissions.includes('verenu-security:default'),
  'Windows Vite remote capability must grant plugin permissions, not a bare app ACL default.',
);
assert(
  Array.isArray(defaultCapability.permissions) && defaultCapability.permissions.includes('verenu-security:default'),
  'Main capability must grant verenu-security plugin permissions explicitly.',
);
assert(
  !defaultCapability.permissions?.includes('default'),
  'Main capability must not reference a bare app ACL default permission.',
);
assert(
  buildScript.includes('cargo:rerun-if-changed=tauri.dev.conf.json'),
  'Tauri must regenerate its embedded authority table when the dev capability config changes.',
);
assert(
  buildScript.includes('cargo:rerun-if-changed=tauri.dev.windows.conf.json'),
  'Tauri must regenerate its embedded authority table when the Windows dev capability config changes.',
);
assert(
  buildScript.includes('.plugin(\n        "verenu-security"') || buildScript.includes('.plugin("verenu-security"'),
  'build.rs must register verenu-security as an inlined plugin ACL, not an app ACL.',
);
assert(
  !existsSync(path.join(repoRoot, 'src-tauri/permissions/verenu-security.toml')),
  'verenu-security must not live at the app permissions root (that creates a restrictive __app-acl__).',
);
assert(
  existsSync(path.join(repoRoot, 'src-tauri/permissions/verenu-security/default.toml')),
  'verenu-security plugin permissions must live under permissions/verenu-security/.',
);
const aclManifestPath = path.join(repoRoot, 'src-tauri/gen/schemas/acl-manifests.json');
if (existsSync(aclManifestPath)) {
  const acl = JSON.parse(readFileSync(aclManifestPath, 'utf8'));
  assert(!Object.prototype.hasOwnProperty.call(acl, '__app-acl__'), 'Generated ACL must not define a restrictive __app-acl__.');
  assert(Object.prototype.hasOwnProperty.call(acl, 'verenu-security'), 'Generated ACL must expose verenu-security as a plugin.');
}
assert(sourcePlist.includes('<string>com.verenu.app</string>'), 'Source Info.plist must retain the production bundle ID.');
assert(sourcePlist.includes('<string>Verenu</string>'), 'Source Info.plist must retain the production name.');
assert(cli.includes("'--config', macDevConfig"), 'Normal tauri dev must merge the canonical development config.');
assert(cli.includes("process.platform === 'win32'"), 'Windows tauri dev must use the isolated development identity.');
assert(cli.includes("'--config', windowsDevConfig"), 'Windows tauri dev must merge the Windows-specific development config.');
assert(cli.includes('tauri.dev.windows.conf.json'), 'Windows development config path must be wired into tauri-cli.');
assert(cli.includes('Refusing an ad-hoc macOS production build'), 'macOS production builds must fail without a signing identity.');
assert(runner.includes("spawn('/usr/bin/open'"), 'Development must launch its app through /usr/bin/open.');
assert(!runner.includes('spawn(bundledBinary'), 'Development must never spawn Contents/MacOS/Verenu directly.');
assert(!runner.includes("const APP_BUNDLE_NAME = 'Verenu.app'"), 'Development must not create an ambiguously named Verenu.app.');
assert(runner.includes("configured !== '-'"), 'Development signing must reject ad-hoc identity selection.');
assert(relaunch.includes('exec /usr/bin/open -n'), 'macOS Relaunch must use LaunchServices.');
assert(runner.includes('.staging-'), 'Development bundles must be staged before installation.');
assert(runner.includes('installPreparedBundle'), 'Development bundle installation must use the atomic swap helper.');
assert(runner.includes('Invalid Page'), 'Development runner must document the live-code-signature rebuild hazard.');

if (failures.length > 0) {
  console.error('macOS identity contract failed:');
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log('macOS identity contract verified: prod=com.verenu.app, dev=com.verenu.app.dev.');
