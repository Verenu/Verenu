#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { startOwnedSession } from './verification/session.mjs';

class NativeDriver {
  constructor(port, id) { this.base = `http://127.0.0.1:${port}/session/${id}`; }
  static async start(port) {
    const response = await fetch(`http://127.0.0.1:${port}/session`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ capabilities: { alwaysMatch: { 'wdio:tauriServiceOptions': { windowLabel: 'main' } } } }),
    });
    const body = await response.json();
    assert.ok(response.ok && body.value?.sessionId, `Could not start native WebDriver session: ${body.value?.message || response.status}`);
    return new NativeDriver(port, body.value.sessionId);
  }
  async command(method, endpoint, body) {
    const response = await fetch(`${this.base}${endpoint}`, {
      method, headers: { 'content-type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const result = await response.json().catch(() => ({}));
    assert.ok(response.ok && !result.value?.error, `WebDriver ${method} ${endpoint} failed: ${result.value?.message || response.status}`);
    return result.value;
  }
  execute(script) { return this.command('POST', '/execute/sync', { script, args: [] }); }
  executeAsync(fn, ...args) { return this.command('POST', '/execute/async', { script: `(${fn.toString()})(...${JSON.stringify(args)}, arguments[arguments.length - 1]);`, args }); }
  async waitForApp() {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      if (await this.execute('return !!document.querySelector(".app");')) return;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    throw new Error('Native WebView did not render .app');
  }
  refresh() { return this.command('POST', '/refresh', {}); }
  getWindowHandles() { return this.command('GET', '/window/handles'); }
  getWindowRect() { return this.command('GET', '/window/rect'); }
  async saveScreenshot(file) { await fs.writeFile(file, Buffer.from(await this.command('GET', '/screenshot'), 'base64')); }
  deleteSession() { return this.command('DELETE', ''); }
}

const args = process.argv.slice(2);
const directory = path.join(root, 'test-results', `native-${randomUUID()}`);
const index = args.indexOf('--report');
const reportArgument = index >= 0 ? args[index + 1] : undefined;
if (index >= 0 && (!reportArgument || reportArgument.startsWith('--'))) {
  throw new Error('--report requires a file path');
}
const reportPath = index < 0 ? path.join(directory, 'verification.json') : path.resolve(reportArgument);
const report = { schemaVersion: 1, identity: sourceIdentity(), status: 'incomplete', scope: ['webview', 'ipc', 'windows'], platform: process.platform, checks: [], artifacts: [], nativeIntegration: { status: 'not-tested', reason: 'Global shortcuts, external insertion, permissions and microphone require dedicated platform fixtures' } };
let session, browser;
try {
  const listener = net.createServer();
  await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
  const port = listener.address().port;
  await new Promise((resolve) => listener.close(resolve));
  process.env.TAURI_WEBDRIVER_PORT = String(port);
  session = await startOwnedSession({ id: `native-${randomUUID()}`, fixtures: directory, directory, native: true });
  browser = await NativeDriver.start(port);
  const invoke = async (command, args = {}) => {
    const result = await browser.executeAsync((name, values, done) => {
      window.__TAURI__.core.invoke(name, values).then((value) => done({ value }), (error) => done({ error: String(error) }));
    }, command, args);
    assert.ok(!result.error, `Real native IPC ${command} failed: ${result.error || ''}`);
    return result.value;
  };
  await browser.waitForApp();
  const identity = await browser.execute('return { native: !!window.__TAURI_INTERNALS__, mocks: !!window.__wdio_mocks__ && Object.keys(window.__wdio_mocks__).length > 0 };');
  assert.equal(identity.native, true); assert.equal(identity.mocks, false);
  const created = await invoke('create_context', { name: 'Synthetic native', contextualFormattingDisabled: false });
  try {
    await browser.refresh(); await browser.waitForApp();
    const contexts = await invoke('get_contexts');
    assert.ok(contexts.some((row) => row.id === created.id && row.name === 'Synthetic native'));
  } finally { await invoke('delete_context', { contextId: created.id }); }
  assert.ok((await browser.getWindowHandles()).length >= 1);
  const rect = await browser.getWindowRect();
  assert.ok(rect.width > 0 && rect.height > 0);
  const screenshot = path.join(directory, 'native-webview.png'); await browser.saveScreenshot(screenshot);
  report.artifacts.push(artifact(screenshot));
  report.checks.push({ name: 'Actual native WebView, IPC, reload persistence and window geometry', status: 'passed' });
  report.status = 'verified';
  if (sourceIdentity().fingerprint !== report.identity.fingerprint) { report.status = 'incomplete'; report.reason = 'Source changed during native verification'; }
} catch (error) { report.status = 'failed'; report.reason = error.message; }
finally {
  if (browser) await browser.deleteSession().catch(() => {});
  if (session) {
    try { await session.stop(); }
    catch (error) {
      report.status = 'failed';
      report.reason = `Could not stop owned session: ${error.message}`;
    }
  }
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Native WebView verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
