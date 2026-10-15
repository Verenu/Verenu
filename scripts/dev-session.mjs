#!/usr/bin/env node
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { randomBytes, randomUUID } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { sourceIdentity } from './verification/identity.mjs';
import { syncBuildIdentityFile } from './verification/build-identity.mjs';
import { cargoTargetDirectory } from './verification/session.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const stateRoot = path.join(os.homedir(), '.local', 'state', 'verenu', 'dev-sessions');

function option(name, fallback) {
  const index = args.indexOf(name);
  if (index < 0) return fallback;
  if (!args[index + 1] || args[index + 1].startsWith('--')) throw new Error(`${name} requires a value`);
  return args[index + 1];
}
function git(...arguments_) {
  return execFileSync('git', arguments_, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
}
async function freePorts(count) {
  const servers = [];
  try {
    const ports = [];
    for (let index = 0; index < count; index++) {
      const server = net.createServer();
      await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
      servers.push(server);
      ports.push(server.address().port);
    }
    return ports;
  } finally {
    await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))));
  }
}
async function list() {
  const directories = await fs.readdir(stateRoot).catch(() => []);
  const rows = [];
  for (const directory of directories) {
    try {
      const row = JSON.parse(await fs.readFile(path.join(stateRoot, directory, 'session.json'), 'utf8'));
      rows.push({ id: row.id, branch: row.branch, localUrl: row.localUrl, shareUrl: row.shareUrl, status: row.status, startedAt: row.startedAt });
    } catch { /* Ignore directories without a manifest. */ }
  }
  console.log(JSON.stringify(rows, null, 2));
}

if (args.includes('--list')) {
  await list();
} else {
  await start().catch((error) => { console.error(error.message); process.exitCode = 1; });
}

async function start() {
  if (args.includes('--help')) {
    console.log('npm run dev:session -- [--share] [--synthetic-seed] [--native-test] [--seed-dir PATH] [--private-history] [--host-mic] [--max-runs N] [--fixtures PATH] [--startup-timeout SEC] [--id NAME] [--list]');
    return;
  }
  const id = option('--id', `${path.basename(root).replace(/[^a-zA-Z0-9-]/g, '-').toLowerCase()}-${randomUUID().slice(0, 8)}`);
  if (!/^[a-zA-Z0-9-]{1,80}$/.test(id)) throw new Error('Session ID must contain 1 to 80 letters, numbers, or hyphens');
  const startupTimeout = Number(option('--startup-timeout', '900'));
  if (!Number.isFinite(startupTimeout) || startupTimeout <= 0) throw new Error('--startup-timeout must be a positive number of seconds');
  const seedDirectory = option('--seed-dir', null);
  const syntheticSeed = args.includes('--synthetic-seed');
  if (syntheticSeed && (seedDirectory || args.includes('--private-history'))) throw new Error('--synthetic-seed cannot copy installed data');
  const directory = path.join(stateRoot, id);
  await fs.mkdir(stateRoot, { recursive: true, mode: 0o700 });
  await fs.chmod(stateRoot, 0o700);
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const lock = await fs.open(path.join(directory, 'launcher.lock'), 'wx', 0o600).catch(() => { throw new Error('Session is already owned. Choose a new --id; do not delete another session lock.'); });
  const children = [];
  let sharedPort;
  let serveWithSudo = false;
  let closing = false;
  let cleanupPromise;
  let manifest;
  const save = async () => fs.writeFile(path.join(directory, 'session.json'), JSON.stringify(manifest, null, 2), { mode: 0o600 });
  const cleanup = () => {
    if (cleanupPromise) return cleanupPromise;
    closing = true;
    cleanupPromise = (async () => {
      for (const child of children) {
        if (child.exitCode === null) {
          if (process.platform === 'win32') child.kill();
          else { try { process.kill(-child.pid, 'SIGTERM'); } catch { /* Already exited. */ } }
        }
      }
      if (sharedPort) {
        try { execFileSync(serveWithSudo ? 'sudo' : 'tailscale', [...(serveWithSudo ? ['-n', 'tailscale'] : []), 'serve', `--https=${sharedPort}`, 'off'], { stdio: 'ignore', timeout: 10_000 }); } catch { console.error(`Could not remove this session's Tailscale listener on ${sharedPort}`); }
      }
      if (manifest) { manifest.status = 'stopped'; await save(); }
      await lock.close();
      await fs.unlink(path.join(directory, 'launcher.lock')).catch(() => {});
    })();
    return cleanupPromise;
  };
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { void cleanup(); });
  function run(component, command, argv, env) {
    const child = spawn(command, argv, { cwd: root, env, stdio: ['ignore', 'inherit', 'inherit'], detached: process.platform !== 'win32' });
    children.push(child);
    child.once('error', (error) => {
      if (manifest) manifest.childFailure = {
        component,
        kind: 'spawn-error',
        ...(typeof error.code === 'string' ? { errorCode: error.code } : {}),
      };
      console.error(error.message);
      void cleanup();
    });
    child.once('exit', (code, signal) => {
      if (!closing) {
        if (manifest) manifest.childFailure = {
          component,
          kind: 'unexpected-exit',
          exitCode: Number.isInteger(code) ? code : null,
          signal: typeof signal === 'string' ? signal : null,
        };
        process.exitCode = code ?? 1;
        void cleanup();
      }
    });
    return child;
  }
  try {
    const [webPort, bridgePort] = await freePorts(2);
    const localUrl = `http://127.0.0.1:${webPort}`;
    let shareUrl = null;
    if (args.includes('--share')) {
      // Use a separate HTTPS listener per session; never reset existing Serve routes.
      const status = JSON.parse(execFileSync('tailscale', ['status', '--json', '--peers=false'], { encoding: 'utf8', timeout: 10_000, maxBuffer: 8 * 1024 * 1024 }));
      if (status.BackendState !== 'Running') throw new Error('Tailscale must be running before --share can be used');
      const dnsName = status.Self?.DNSName?.replace(/\.$/, '');
      if (!dnsName) throw new Error('Tailscale MagicDNS name is unavailable');
      const serve = JSON.parse(execFileSync('tailscale', ['serve', 'status', '--json'], { encoding: 'utf8', timeout: 10_000 }));
      let port = 20000 + Math.floor(Math.random() * 20000);
      while (serve.TCP?.[String(port)]) port++;
      const serveArgs = ['serve', '--bg', `--https=${port}`, localUrl];
      try { execFileSync('tailscale', serveArgs, { stdio: 'pipe', timeout: 15_000 }); }
      catch (error) {
        if (!String(error.stderr || '').includes('Access denied')) throw new Error('Tailscale Serve failed. Verify that HTTPS is enabled for your tailnet and this device can use Serve.');
        try { execFileSync('sudo', ['-n', 'tailscale', ...serveArgs], { stdio: 'pipe', timeout: 15_000 }); serveWithSudo = true; }
        catch { throw new Error('Tailscale Serve requires operator permission or passwordless sudo for the tailscale command.'); }
      }
      sharedPort = port;
      shareUrl = `https://${dnsName}:${port}`;
    }
    const token = randomBytes(32).toString('hex');
    // Capability links belong in a private file, never terminal logs or reports.
    await fs.writeFile(path.join(directory, 'access.json'), JSON.stringify({ token, localAccessUrl: `${localUrl}/#session-token=${token}`, shareAccessUrl: shareUrl ? `${shareUrl}/#session-token=${token}` : null }), { mode: 0o600 });
    const identity = sourceIdentity();
    const buildIdentityFile = path.join(root, 'src-tauri', '.verenu-build-identity');
    await syncBuildIdentityFile(buildIdentityFile, identity.fingerprint);
    manifest = { id, directory, ...identity, dirty: git('status', '--porcelain') !== '', localUrl, shareUrl, status: 'starting', startedAt: new Date().toISOString(), launcherPid: process.pid, privateHistory: args.includes('--private-history'), syntheticSeed: args.includes('--synthetic-seed'), nativeTest: args.includes('--native-test') };
    await save();
    await fs.rm(path.join(directory, 'ready'), { force: true });
    await fs.mkdir(path.join(directory, 'fixtures'), { recursive: true });
    const fixtures = option('--fixtures', path.join(os.homedir(), '.cache', 'verenu', 'test-audio'));
    if (fixtures) {
      for (const name of await fs.readdir(path.resolve(fixtures)).catch(() => [])) {
        if (/^[a-zA-Z0-9_-]+\.wav$/.test(name)) await fs.copyFile(path.join(path.resolve(fixtures), name), path.join(directory, 'fixtures', name));
      }
    }
    const maxRuns = Number(option('--max-runs', '30'));
    if (!Number.isSafeInteger(maxRuns) || maxRuns < 0 || maxRuns > 10000) throw new Error('--max-runs must be an integer from 0 to 10000');
    const env = { ...process.env, VITE_VERENU_SESSION: '1', VERENU_DEV_SESSION_ID: id, VERENU_DEV_SESSION_DIR: directory, VERENU_DEV_BRANCH: manifest.branch, VERENU_DEV_COMMIT: manifest.commit, VERENU_DEV_WEB_PORT: String(webPort), VERENU_DEV_BRIDGE_PORT: String(bridgePort), VERENU_DEV_ORIGINS: [localUrl, shareUrl].filter(Boolean).join(','), VERENU_DEV_PRIVATE_HISTORY: args.includes('--private-history') ? '1' : '0', VERENU_DEV_HOST_MIC: args.includes('--host-mic') ? '1' : '0', VERENU_DEV_MAX_RUNS: String(maxRuns), CARGO_TARGET_DIR: cargoTargetDirectory(process.env) };
    env.VERENU_BUILD_FINGERPRINT = identity.fingerprint;
    env.VERENU_BUILD_FINGERPRINT_FILE = buildIdentityFile;
    env.VERENU_DEV_WORKTREE = identity.worktree;
    if (syntheticSeed) {
      const seed = path.join(directory, 'synthetic-seed');
      await fs.mkdir(seed, { recursive: true });
      await fs.writeFile(path.join(seed, 'settings.json'), JSON.stringify({ setup_complete: true, noise_reduction: false, analytics_enabled: false }), { mode: 0o600 });
      env.VERENU_DEV_SEED_DIR = seed;
    }
    if (args.includes('--native-test')) { delete env.VITE_VERENU_SESSION; env.VITE_VERENU_NATIVE_TEST = '1'; }
    if (seedDirectory) env.VERENU_DEV_SEED_DIR = path.resolve(seedDirectory);
    // No inherited environment credential fallback in local dev sessions.
    env.VITE_VERENU_SESSION_ID = id;
    env.VERENU_DEV_SHARE_URL = shareUrl || '';
    delete env.VERENU_ALLOW_ENV_CREDENTIALS;
    const config = path.join(directory, 'tauri-session.json');
    await fs.writeFile(config, JSON.stringify({ identifier: `com.verenu.session.${id.toLowerCase()}`, build: { devUrl: localUrl, beforeDevCommand: '' }, app: { withGlobalTauri: args.includes('--native-test'), windows: [{ label: 'main', title: 'Verenu dev worker', url: '/', width: 1320, height: 860, visible: args.includes('--native-test') }], security: { devCsp: null, ...(args.includes('--native-test') ? { capabilities: ['default', 'pill', { identifier: 'native-test', windows: ['main'], local: true, remote: { urls: [`${localUrl}/*`] }, permissions: ['wdio:default', 'wdio-webdriver:default'] }] } : {}) } } }));
    run('frontend', process.execPath, [path.join(root, 'node_modules', 'vite', 'bin', 'vite.js'), '--port', String(webPort), '--strictPort'], env);
    run('tauri', process.execPath, [path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), 'dev', '--features', args.includes('--native-test') ? 'native-testing' : 'dev-session', '--no-watch', '--config', config], env);
    console.log(`Session ${id}\nLocal: ${localUrl}\nPhone: ${shareUrl || 'Use --share to create a private Tailscale URL'}\nAccess links: ${path.join(directory, 'access.json')}\nManifest: ${path.join(directory, 'session.json')}\nWaiting for this worktree's Rust backend...`);
    const deadline = Date.now() + startupTimeout * 1000;
    while (!closing) {
      if (Date.now() > deadline) throw new Error('Rust session startup timed out');
      const ready = await fs.readFile(path.join(directory, 'ready'), 'utf8').catch(() => null);
      if (ready) { manifest.status = 'ready'; await save(); console.log(`Session ${id} ready. Open an access link from access.json.`); break; }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    if (!closing) await new Promise((resolve) => { for (const child of children) child.once('exit', resolve); process.once('SIGINT', resolve); process.once('SIGTERM', resolve); });
  } finally { await cleanup(); }
}
