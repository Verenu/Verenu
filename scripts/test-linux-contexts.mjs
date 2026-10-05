#!/usr/bin/env node
// Native browser fixtures need a real Wayland window. They never use the
// installed browser profile, user tabs, clipboard, microphone, or credentials.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import http from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { sourceIdentity } from './verification/identity.mjs';

const root = path.resolve(import.meta.dirname, '..');
await fs.mkdir(path.join(root, 'test-results'), { recursive: true });
const directory = await fs.mkdtemp(path.join(root, 'test-results', 'linux-contexts-'));
const identity = sourceIdentity();
const checks = [];
let browser;
let browserStarted = false;
let socket;
function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve(true);
  return new Promise((resolve) => {
    const onExit = () => {
      clearTimeout(timer);
      resolve(true);
    };
    const timer = setTimeout(() => {
      child.off('exit', onExit);
      resolve(false);
    }, timeoutMs);
    child.once('exit', onExit);
  });
}
const server = http.createServer((request, response) => {
  const host = request.headers.host.split(':')[0];
  response.writeHead(200, { 'Content-Type': 'text/html' });
  response.end(`<title>${host}</title><h1>Public Context fixture</h1><p>${host}</p><input value="wrong.example.test"><textarea>Public synthetic editor</textarea>`);
});
try {
  assert.equal(process.platform, 'linux');
  assert.ok(process.env.HYPRLAND_INSTANCE_SIGNATURE, 'Requires a real Hyprland session');
  assert.equal(process.env.VERENU_CONTEXT_TEST_ISOLATED, '1', 'Run in a disposable Hyprland compositor with VERENU_CONTEXT_TEST_ISOLATED=1');
  const build = execFileSync('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib', '--no-run', '--message-format=json'], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
  const executable = build.split('\n').filter(Boolean).flatMap((line) => {
    try {
      return [JSON.parse(line)];
    } catch {
      return [];
    }
  })
    .findLast((row) => row.reason === 'compiler-artifact' && row.profile.test && row.executable)?.executable;
  assert.ok(executable, 'Missing native test executable');
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  const firstUrl = `http://coding.example.test:${port}`;
  const chrome = process.env.VERENU_CHROME_BINARY || '/opt/google/chrome/chrome';
  browser = spawn(chrome, [
    `--user-data-dir=${path.join(directory, 'profile')}`, '--no-first-run', '--no-default-browser-check',
    '--disable-sync', '--disable-background-networking', '--no-proxy-server',
    '--host-resolver-rules=MAP *.example.test 127.0.0.1', '--remote-debugging-port=0',
    '--ozone-platform=wayland', '--disable-features=Vulkan', firstUrl,
  ], { stdio: ['ignore', 'ignore', 'ignore'] });
  await new Promise((resolve, reject) => {
    browser.once('spawn', () => {
      browserStarted = true;
      resolve();
    });
    browser.once('error', reject);
  }).catch((error) => {
    throw new Error(`Could not start Chrome at ${chrome}: ${error.message}`);
  });
  let debuggerPort;
  for (let attempt = 0; attempt < 100; attempt++) {
    const activePort = await fs.readFile(path.join(directory, 'profile', 'DevToolsActivePort'), 'utf8').catch(() => '');
    if (activePort) { debuggerPort = Number(activePort.split('\n')[0]); break; }
    if (browser.exitCode !== null || browser.signalCode !== null) {
      throw new Error(`Owned Chrome exited before initialization (${browser.exitCode ?? browser.signalCode})`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(debuggerPort, 'Owned Chrome startup timed out');
  const version = await (await fetch(`http://127.0.0.1:${debuggerPort}/json/version`)).json();
  socket = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  let sequence = 0;
  const pending = new Map();
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (message.id) { pending.get(message.id)?.(message); pending.delete(message.id); }
  });
  async function cdp(method, params = {}) {
    const id = ++sequence;
    const result = new Promise((resolve) => pending.set(id, resolve));
    socket.send(JSON.stringify({ id, method, params }));
    const response = await Promise.race([result, new Promise((_, reject) => setTimeout(() => reject(new Error(`CDP timeout: ${method}`)), 5000))]);
    assert.ok(!response.error, `CDP failed: ${method}`);
    return response.result;
  }
  async function probe(name, host, expected = host) {
    const title = `${host} - Google Chrome`;
    let window;
    for (let attempt = 0; attempt < 100; attempt++) {
      const clients = JSON.parse(execFileSync('hyprctl', ['-j', 'clients'], { encoding: 'utf8' }));
      const matchingWindows = clients.filter((client) => client.pid === browser.pid && client.title === title);
      window = matchingWindows[0];
      if (window && (expected !== 'unavailable' || matchingWindows.length >= 2)) break;
      window = undefined;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.ok(window, `Native fixture window missing: ${name}`);
    const started = Date.now();
    let log;
    try {
      log = execFileSync(executable, ['atspi_live_address_bar', '--ignored'], {
        cwd: root, encoding: 'utf8', timeout: 10_000,
        env: { ...process.env, VERENU_ATSPI_PID: String(browser.pid), VERENU_ATSPI_EXPECTED_DOMAIN: expected, VERENU_ATSPI_WINDOW_ADDRESS: window.address },
      });
    } catch (error) {
      await fs.writeFile(path.join(directory, `${name}.log`), `${error.stdout || ''}${error.stderr || ''}`);
      throw new Error(`Native fixture failed: ${name}. Inspect ${path.join(directory, `${name}.log`)}`);
    }
    await fs.writeFile(path.join(directory, `${name}.log`), log);
    checks.push({ name, status: 'passed', elapsedMs: Date.now() - started });
  }
  await probe('cold-browser', 'coding.example.test');
  const first = (await cdp('Target.getTargets')).targetInfos.find((target) => target.url.startsWith(firstUrl));
  const second = await cdp('Target.createTarget', { url: `http://mail.example.test:${port}` });
  await cdp('Target.activateTarget', { targetId: second.targetId });
  await probe('switch-tab', 'mail.example.test');
  await cdp('Target.activateTarget', { targetId: first.targetId });
  await probe('switch-tab-back', 'coding.example.test');
  const third = await cdp('Target.createTarget', { url: `http://docs.example.test:${port}`, newWindow: true });
  await cdp('Target.activateTarget', { targetId: third.targetId });
  await probe('second-window-same-pid', 'docs.example.test');
  await cdp('Target.activateTarget', { targetId: first.targetId });
  await probe('first-window-same-pid', 'coding.example.test');
  for (let index = 0; index < 24; index++) {
    await cdp('Target.createTarget', { url: `http://background${index}.example.test:${port}`, background: true });
  }
  await cdp('Target.activateTarget', { targetId: first.targetId });
  await probe('many-background-tabs', 'coding.example.test');
  await cdp('Target.createTarget', { url: firstUrl, newWindow: true });
  await probe('duplicate-window-title-fallback', 'coding.example.test', 'unavailable');
  console.log(`Native Linux Context checks passed: ${checks.length}. Evidence: ${directory}`);
} catch (error) {
  checks.push({ name: 'native-browser', status: 'failed', reason: error.message });
  process.exitCode = 1;
  console.error(error.message);
} finally {
  socket?.close();
  if (browser && browserStarted && browser.exitCode === null && browser.signalCode === null) {
    browser.kill('SIGTERM');
    if (!(await waitForExit(browser, 3_000))) {
      browser.kill('SIGKILL');
      if (!(await waitForExit(browser, 3_000))) browser.unref();
    }
  }
  if (server.listening) {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
  await fs.writeFile(path.join(directory, 'verification.json'), JSON.stringify({ identity, checks, status: checks.length === 7 && checks.every((check) => check.status === 'passed') ? 'verified' : 'failed' }, null, 2));
}
