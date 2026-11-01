#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { cargoTargetDirectory, startOwnedSession } from './verification/session.mjs';
import { createNativeSession } from './verification/native-session.mjs';
import { markCurrentNativePage, waitForNativePage } from './verification/native-page.mjs';
import { verifyNativePill } from './verification/native-pill.mjs';

class NativeDriver {
  constructor(port, id) { this.base = `http://127.0.0.1:${port}/session/${id}`; }
  static async start(port) {
    const id = await createNativeSession(port);
    return new NativeDriver(port, id);
  }
  async command(method, endpoint, body, timeout = 10000) {
    const response = await fetch(`${this.base}${endpoint}`, {
      method, headers: { 'content-type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(timeout),
    });
    const result = await response.json().catch(() => ({}));
    assert.ok(response.ok && !result.value?.error, `WebDriver ${method} ${endpoint} failed: ${result.value?.message || response.status}`);
    return result.value;
  }
  execute(script) { return this.command('POST', '/execute/sync', { script, args: [] }); }
  executeAsync(fn, ...args) { return this.command('POST', '/execute/async', { script: `(${fn.toString()})(...arguments);`, args }); }
  async waitForApp(options) {
    return waitForNativePage(this, options);
  }
  refresh() { return this.command('POST', '/refresh', {}); }
  getWindowHandles() { return this.command('GET', '/window/handles'); }
  getWindowRect() { return this.command('GET', '/window/rect'); }
  async saveScreenshot(file) { await fs.writeFile(file, Buffer.from(await this.command('GET', '/screenshot'), 'base64')); }
  deleteSession() { return this.command('DELETE', ''); }
}

async function linuxProcessTree(rootPid) {
  const ifPresent = promise => promise.catch(error => {
    if (error.code === 'ENOENT' || error.code === 'ESRCH') return null;
    throw error;
  });
  const pending = [rootPid];
  const seen = new Set();
  const processes = [];
  while (pending.length) {
    const pid = pending.pop();
    if (seen.has(pid)) continue;
    seen.add(pid);
    const [stat, tasks] = await Promise.all([
      ifPresent(fs.readFile(`/proc/${pid}/stat`, 'utf8')),
      ifPresent(fs.readdir(`/proc/${pid}/task`)),
    ]);
    if (stat === null || tasks === null) continue;
    const end = stat.lastIndexOf(')');
    const fields = stat.slice(end + 2).trim().split(/\s+/);
    if (end < 0 || fields.length < 20 || !/^\d+$/.test(fields[19])) throw new Error(`Invalid process identity: ${pid}`);
    processes.push({ pid, name: stat.slice(stat.indexOf('(') + 1, end), state: fields[0], parent: Number(fields[1]), start: fields[19] });
    const childLists = await Promise.all(tasks.map(task =>
      ifPresent(fs.readFile(`/proc/${pid}/task/${task}/children`, 'utf8'))));
    for (const children of childLists) {
      if (children === null) continue;
      pending.push(...children.trim().split(/\s+/).filter(Boolean).map(Number));
    }
  }
  return processes;
}

async function installLinuxPageProcessProbe(directory) {
  const extensionDirectory = path.join(directory, 'webkit-probe');
  await fs.mkdir(extensionDirectory, { recursive: true, mode: 0o700 });
  const extensionSource = path.join(directory, 'webkit-probe.c');
  const preloadSource = path.join(directory, 'webkit-preload.c');
  const preload = path.join(directory, 'webkit-preload.so');
  await fs.writeFile(extensionSource, `
#include <webkit2/webkit-web-extension.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static void report(WebKitWebPage *page, GParamSpec *property, gpointer data) {
  (void)property; (void)data;
  const char *uri = webkit_web_page_get_uri(page);
  const char *kind = "other"; int port = 0;
  GUri *parsed = uri ? g_uri_parse(uri, G_URI_FLAGS_NONE, NULL) : NULL;
  if (parsed && g_strcmp0(g_uri_get_scheme(parsed), "http") == 0 &&
      g_strcmp0(g_uri_get_host(parsed), "127.0.0.1") == 0 &&
      !g_uri_get_userinfo(parsed) && !g_uri_get_query(parsed) && !g_uri_get_fragment(parsed) &&
      g_uri_get_port(parsed) > 0) {
    const char *path = g_uri_get_path(parsed);
    if (g_strcmp0(path, "/") == 0) kind = "main";
    else if (g_strcmp0(path, "/pill.html") == 0) kind = "pill";
    if (strcmp(kind, "other") != 0) port = g_uri_get_port(parsed);
  }
  if (parsed) g_uri_unref(parsed);
  char stat[4096]; FILE *file = fopen("/proc/self/stat", "r");
  if (!file || !fgets(stat, sizeof(stat), file)) { if (file) fclose(file); return; }
  fclose(file);
  char *end = strrchr(stat, ')'); if (!end) return;
  char *save = NULL; char *field = strtok_r(end + 2, " ", &save);
  for (int i = 0; field && i < 19; i++) field = strtok_r(NULL, " ", &save);
  if (!field) return;
  fprintf(stderr, "VERENU_NATIVE_PAGE pid=%ld start=%s page=%lu kind=%s port=%d\\n",
    (long)getpid(), field, (unsigned long)webkit_web_page_get_id(page), kind, port);
  fflush(stderr);
}
static void page_created(WebKitWebExtension *extension, WebKitWebPage *page, gpointer data) {
  (void)extension; (void)data;
  g_signal_connect(page, "notify::uri", G_CALLBACK(report), NULL);
  report(page, NULL, NULL);
}
G_MODULE_EXPORT void webkit_web_extension_initialize(WebKitWebExtension *extension) {
  g_signal_connect(extension, "page-created", G_CALLBACK(page_created), NULL);
}
`, { mode: 0o600 });
  await fs.writeFile(preloadSource, `
#define _GNU_SOURCE
#include <webkit2/webkit2.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
static unsigned pill_terminations;
static guint64 observed_page;
static void load_changed(WebKitWebView *view, WebKitLoadEvent event, gpointer data) {
  (void)data;
  if (observed_page && webkit_web_view_get_page_id(view) == observed_page) {
    fprintf(stderr, "VERENU_NATIVE_LOAD page=%llu event=%d time=%lld\\n",
      (unsigned long long)observed_page, event, (long long)g_get_monotonic_time());
    fflush(stderr);
  }
}
static gboolean observation_finished(gpointer data) {
  (void)data;
  fprintf(stderr, "VERENU_NATIVE_OBSERVE_END page=%llu time=%lld\\n",
    (unsigned long long)observed_page, (long long)g_get_monotonic_time());
  fflush(stderr);
  return G_SOURCE_REMOVE;
}
static gboolean early_reload(gpointer data) {
  WebKitWebView *view = WEBKIT_WEB_VIEW(data);
  fprintf(stderr, "VERENU_NATIVE_EARLY_RELOAD pill page=%llu time=%lld\\n",
    (unsigned long long)observed_page, (long long)g_get_monotonic_time());
  fflush(stderr);
  g_timeout_add(5500, observation_finished, NULL);
  webkit_web_view_reload(view);
  g_object_unref(view);
  return G_SOURCE_REMOVE;
}
static void terminated(WebKitWebView *view, WebKitWebProcessTerminationReason reason, gpointer data) {
  (void)reason; (void)data;
  const char *uri = webkit_web_view_get_uri(view);
  GUri *parsed = uri ? g_uri_parse(uri, G_URI_FLAGS_NONE, NULL) : NULL;
  gboolean pill = parsed && g_strcmp0(g_uri_get_scheme(parsed), "http") == 0 &&
    g_strcmp0(g_uri_get_host(parsed), "127.0.0.1") == 0 &&
    g_strcmp0(g_uri_get_path(parsed), "/pill.html") == 0 &&
    !g_uri_get_userinfo(parsed) && !g_uri_get_query(parsed) && !g_uri_get_fragment(parsed);
  if (parsed) g_uri_unref(parsed);
  if (pill && ++pill_terminations == 3) {
    observed_page = webkit_web_view_get_page_id(view);
    fprintf(stderr, "VERENU_NATIVE_TERMINATION page=%llu time=%lld\\n",
      (unsigned long long)observed_page, (long long)g_get_monotonic_time());
    fflush(stderr);
    g_timeout_add(100, early_reload, g_object_ref(view));
  }
}
WebKitWebContext *webkit_web_view_get_context(WebKitWebView *view) {
  static WebKitWebContext *(*next)(WebKitWebView *);
  if (!next) next = dlsym(RTLD_NEXT, "webkit_web_view_get_context");
  if (getenv("VERENU_NATIVE_WEBKIT_PROBE") && !g_object_get_data(G_OBJECT(view), "verenu-native-reload-probe")) {
    g_object_set_data(G_OBJECT(view), "verenu-native-reload-probe", GINT_TO_POINTER(1));
    g_signal_connect(view, "web-process-terminated", G_CALLBACK(terminated), NULL);
    g_signal_connect(view, "load-changed", G_CALLBACK(load_changed), NULL);
  }
  return next(view);
}
static void configure(WebKitWebContext *context) {
  const char *directory = getenv("VERENU_NATIVE_WEBKIT_PROBE");
  if (directory && !g_object_get_data(G_OBJECT(context), "verenu-native-probe")) {
    webkit_web_context_set_web_extensions_directory(context, directory);
    g_object_set_data(G_OBJECT(context), "verenu-native-probe", GINT_TO_POINTER(1));
  }
}
void webkit_web_context_set_cache_model(WebKitWebContext *context, WebKitCacheModel model) {
  static void (*next)(WebKitWebContext *, WebKitCacheModel);
  if (!next) next = dlsym(RTLD_NEXT, "webkit_web_context_set_cache_model");
  configure(context); next(context, model);
}
void webkit_web_context_register_uri_scheme(WebKitWebContext *context, const gchar *scheme,
    WebKitURISchemeRequestCallback callback, gpointer data, GDestroyNotify destroy) {
  static void (*next)(WebKitWebContext *, const gchar *, WebKitURISchemeRequestCallback, gpointer, GDestroyNotify);
  if (!next) next = dlsym(RTLD_NEXT, "webkit_web_context_register_uri_scheme");
  configure(context); next(context, scheme, callback, data, destroy);
}
`, { mode: 0o600 });
  try {
    for (const [source, output, packageName] of [
      [extensionSource, path.join(extensionDirectory, 'libverenu-native-probe.so'), 'webkit2gtk-web-extension-4.1'],
      [preloadSource, preload, 'webkit2gtk-4.1'],
    ]) {
      const flags = execFileSync('pkg-config', ['--cflags', '--libs', packageName], { encoding: 'utf8', timeout: 5000 }).trim().split(/\s+/);
      execFileSync('cc', ['-shared', '-fPIC', '-o', output, source, ...flags, '-ldl'], { timeout: 60000, stdio: 'pipe' });
    }
  } catch (cause) {
    const error = new Error('Native renderer identification requires cc, pkg-config, and WebKitGTK 4.1 development headers', { cause });
    error.verificationIncomplete = true;
    throw error;
  }
  return { extensionDirectory, preload };
}

async function rendererForPage(directory, renderers, origin, kind) {
  const logName = (await fs.readdir(directory)).find(name => /^startup-\d+\.log$/.test(name));
  assert.ok(logName, 'Owned session log missing');
  const log = await fs.readFile(path.join(directory, logName), 'utf8');
  const reports = [...log.matchAll(/^VERENU_NATIVE_PAGE pid=(\d+) start=(\d+) page=(\d+) kind=(main|pill|other) port=(\d+)$/gm)]
    .map(match => ({ pid: Number(match[1]), start: match[2], page: match[3], kind: match[4], port: Number(match[5]) }));
  const latestByPage = new Map();
  for (const report of reports) latestByPage.set(`${report.pid}:${report.start}:${report.page}`, report);
  const port = Number(new URL(origin).port);
  const matches = [...latestByPage.values()].filter(report => report.kind === kind && report.port === port &&
    renderers.some(renderer => renderer.pid === report.pid && renderer.start === report.start));
  assert.equal(matches.length, 1, `WebKit did not uniquely identify the renderer for ${kind}`);
  return renderers.find(renderer => renderer.pid === matches[0].pid && renderer.start === matches[0].start);
}

function terminateOwnedLinuxRenderer({ launcher, app, renderer, appExecutable, rendererExecutable }) {
  // pidfd pins the process identity across validation and signalling, even if a PID is reused.
  const program = `
import os, signal, sys
launcher_pid, launcher_start, app_pid, app_start, app_exe, renderer_pid, renderer_start, renderer_exe = sys.argv[1:]
launcher_pid, app_pid, renderer_pid = int(launcher_pid), int(app_pid), int(renderer_pid)
def identity(pid):
    stat = open(f'/proc/{pid}/stat', encoding='utf-8').read()
    end = stat.rfind(')')
    fields = stat[end + 2:].split()
    if end < 0 or len(fields) < 20 or fields[0] in ('Z', 'X'):
        raise RuntimeError('process identity is unavailable')
    return stat[stat.find('(') + 1:end], int(fields[1]), fields[19]
app_fd = os.pidfd_open(app_pid)
renderer_fd = os.pidfd_open(renderer_pid)
try:
    launcher = identity(launcher_pid)
    app = identity(app_pid)
    renderer = identity(renderer_pid)
    if launcher[2] != launcher_start or app[0] != 'verenu' or app[2] != app_start:
        raise RuntimeError('launcher or app identity changed')
    if os.readlink(f'/proc/{app_pid}/exe') != app_exe:
        raise RuntimeError('app executable changed')
    if renderer[0] != 'WebKitWebProces' or renderer[2] != renderer_start or os.readlink(f'/proc/{renderer_pid}/exe') != renderer_exe:
        raise RuntimeError('renderer identity changed')
    def require_ancestor(child, ancestor):
        seen = set()
        while child != ancestor:
            if child in seen or child <= 1:
                raise RuntimeError('owned process ancestry changed')
            seen.add(child)
            child = identity(child)[1]
    require_ancestor(renderer_pid, app_pid)
    require_ancestor(app_pid, launcher_pid)
    signal.pidfd_send_signal(renderer_fd, signal.SIGKILL)
finally:
    os.close(renderer_fd)
    os.close(app_fd)
`;
  execFileSync('python3', ['-c', program,
    String(launcher.pid), launcher.start, String(app.pid), app.start, appExecutable,
    String(renderer.pid), renderer.start, rendererExecutable], { timeout: 5000, stdio: 'pipe' });
}

async function waitUntil(check, label, timeout = 30000) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    try { if (await check()) return; }
    catch (error) { lastError = error.message; }
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw new Error(`${label} did not recover${lastError ? `: ${lastError}` : ''}`);
}

function incomplete(reason) {
  const error = new Error(reason);
  error.verificationIncomplete = true;
  throw error;
}

function verifyManualReloadObservation(log) {
  const termination = /^VERENU_NATIVE_TERMINATION page=(\d+) time=(\d+)$/m.exec(log);
  const early = /^VERENU_NATIVE_EARLY_RELOAD pill page=(\d+) time=(\d+)$/m.exec(log);
  const end = /^VERENU_NATIVE_OBSERVE_END page=(\d+) time=(\d+)$/m.exec(log);
  assert.ok(termination && early && end, 'Native manual-reload observation is incomplete');
  assert.equal(termination[1], early[1]);
  assert.equal(termination[1], end[1]);
  const terminatedAt = Number(termination[2]);
  const injectedAt = Number(early[2]);
  const endedAt = Number(end[2]);
  assert.ok(injectedAt >= terminatedAt && endedAt - terminatedAt >= 5_000_000, 'Native observation did not cover the pending recovery timer');
  const events = [...log.matchAll(/^VERENU_NATIVE_LOAD page=(\d+) event=(\d+) time=(\d+)$/gm)]
    .filter(match => match[1] === termination[1])
    .map(match => ({ event: Number(match[2]), time: Number(match[3]) }))
    .filter(event => event.time >= injectedAt && event.time <= endedAt);
  const starts = events.filter(event => event.event === 0);
  const finishes = events.filter(event => event.event === 3);
  assert.equal(starts.length, 1, 'Pending recovery caused an additional pill load');
  assert.equal(finishes.length, 1, 'Manual pill load did not finish exactly once');
  assert.ok(finishes[0].time >= starts[0].time, 'Manual pill load finished before it began');
  if (starts[0].time - terminatedAt >= 3_500_000) incomplete('Native manual load did not begin before the pending recovery timer');
}

async function verifyLinuxRendererRecovery({ browser, invoke, session }) {
  const context = 'Synthetic recovery context';
  await browser.command('POST', '/window', { handle: 'main' });
  await invoke('native_test_pill', { state: 'handsfree', context });
  await browser.command('POST', '/window', { handle: 'pill' });
  const pillReady = () => browser.execute(`return {
    ready: document.readyState === 'complete',
    bridge: !!window.__TAURI_INTERNALS__,
    handsfree: !!document.querySelector('.pill.handsfree'),
    context: document.querySelector('.pill-context-label')?.textContent?.trim(),
    controls: [...document.querySelectorAll('.pill button')].map(button => button.getAttribute('aria-label'))
  };`);
  await waitUntil(async () => {
    const state = await pillReady();
    return state.handsfree && state.context === context && state.controls.join(',') === 'Cancel,Confirm';
  }, 'Pill before renderer termination');
  const appExecutable = await fs.realpath(path.resolve(root, cargoTargetDirectory(), 'debug', 'verenu'));
  let previousRenderer, originalApp, firstTermination;
  for (let attempt = 0; attempt < 3; attempt++) {
    if (attempt >= 1) {
      await browser.command('POST', '/window', { handle: 'main' });
      await invoke('native_test_pill', { state: 'paste_failed', message: 'Partly pasted' });
      await browser.command('POST', '/window', { handle: 'pill' });
      await waitUntil(async () => await browser.execute(
        `return document.querySelector('.pill.paste-failed .paste-failed-text')?.textContent?.trim() === 'Partly pasted';`
      ), 'Partial-paste state before renderer termination');
      await browser.command('POST', '/window', { handle: 'main' });
      await browser.execute(`
        const settings = [...document.querySelectorAll('button')].find(button => button.textContent.trim() === 'Settings');
        if (!document.querySelector('input[aria-label="Search settings"]')) settings?.click();
        return true;
      `);
      await waitUntil(async () => await browser.execute(`return !!document.querySelector('input[aria-label="Search settings"]');`), 'Settings search');
      assert.equal(await browser.execute(`
        const input = document.querySelector('input[aria-label="Search settings"]');
        input.value = 'Synthetic unsaved query'; input.dispatchEvent(new Event('input', { bubbles: true }));
        return input.value;
      `), 'Synthetic unsaved query');
    }
    await browser.command('POST', '/window', { handle: 'pill' });
    const pillMarker = await markCurrentNativePage(browser);
    await browser.command('POST', '/window', { handle: 'main' });
    const mainMarker = await markCurrentNativePage(browser);

    assert.equal(session.child.exitCode, null, 'Owned session has exited');
    assert.equal(session.child.signalCode, null, 'Owned session has been signalled');
    const tree = await linuxProcessTree(session.child.pid);
    const launcher = tree.find(process => process.pid === session.child.pid);
    const apps = tree.filter(process => process.name === 'verenu');
    assert.ok(launcher, 'Owned session launcher is missing');
    assert.equal(apps.length, 1, 'Expected one Verenu process in the owned session');
    const app = apps[0];
    if (originalApp) assert.deepEqual({ pid: app.pid, start: app.start }, originalApp, 'Owned app changed after renderer recovery');
    else originalApp = { pid: app.pid, start: app.start };
    assert.equal(await fs.readlink(`/proc/${app.pid}/exe`), appExecutable, 'Owned app executable does not match this worktree build');
    const renderers = tree.filter(process => process.name === 'WebKitWebProces');
    const origin = new URL(session.access.localAccessUrl).origin;
    const main = await rendererForPage(session.testDirectory, renderers, origin, 'main');
    const pill = await rendererForPage(session.testDirectory, renderers, origin, 'pill');
    const renderer = pill;
    if (attempt === 0) {
      if (main.pid !== pill.pid) incomplete('WebKit did not place main and pill in a shared renderer; shared-crash recovery was not tested');
    } else {
      if (main.pid === pill.pid) incomplete('WebKit re-shared the renderer; pill-only crash and manual-reload recovery were not tested');
    }
    if (attempt === 2) {
      const logsDirectory = path.join(session.directory, 'data', 'logs');
      await waitUntil(async () => {
        const files = (await fs.readdir(logsDirectory)).filter(name => name.endsWith('.log'));
        const logs = (await Promise.all(files.map(name => fs.readFile(path.join(logsDirectory, name), 'utf8')))).join('\n');
        return (logs.match(/Reloading Linux webview pill after renderer termination/g) || []).length >= 2;
      }, 'Two automatic pill reloads before manual-reload test');
    }
    if (previousRenderer) {
      assert.notDeepEqual({ pid: renderer.pid, start: renderer.start }, previousRenderer, 'Recovery did not start a distinct renderer');
      assert.ok(performance.now() - firstTermination < 60000, 'Second renderer termination must occur within 60 seconds');
    }
    previousRenderer = { pid: renderer.pid, start: renderer.start };
    const rendererExecutable = await fs.readlink(`/proc/${renderer.pid}/exe`);
    assert.equal(path.basename(rendererExecutable), 'WebKitWebProcess', 'Unexpected renderer executable');
    terminateOwnedLinuxRenderer({ launcher, app, renderer, appExecutable, rendererExecutable });
    if (attempt === 0) firstTermination = performance.now();

    await waitUntil(async () => {
      await browser.command('POST', '/window', { handle: 'main' });
      const state = await browser.execute(`return {
        ready: document.readyState === 'complete', app: !!document.querySelector('.app'),
        bridge: !!window.__TAURI_INTERNALS__, old: window[${JSON.stringify(mainMarker)}] === true
      };`);
      const draft = attempt === 0 || await browser.execute(`return document.querySelector('input[aria-label="Search settings"]')?.value === 'Synthetic unsaved query';`);
      return state.ready && state.app && state.bridge && (attempt === 0 ? !state.old : state.old) && draft &&
        await invoke('get_setting', { key: 'setup_complete' }) === true;
    }, `Main window after termination ${attempt + 1}`);
    await waitUntil(async () => {
      await browser.command('POST', '/window', { handle: 'pill' });
      const state = await pillReady();
      const old = await browser.execute(`return window[${JSON.stringify(pillMarker)}] === true;`);
      const content = attempt === 0
        ? state.handsfree && state.context === context && state.controls.join(',') === 'Cancel,Confirm'
        : await browser.execute(`return document.querySelector('.pill.paste-failed .paste-failed-text')?.textContent?.trim() === 'Partly pasted';`);
      return state.ready && state.bridge && !old && content &&
        await invoke('get_setting', { key: 'setup_complete' }) === true;
    }, `Pill window after termination ${attempt + 1}`);
    if (attempt === 0) report.checks.push({ name: 'Shared renderer crash restores main and pill', status: 'passed' });
    if (attempt === 1) report.checks.push({ name: 'Pill-only crash preserves unsaved main input', status: 'passed' });
  }
  const logName = (await fs.readdir(session.testDirectory)).find(name => /^startup-\d+\.log$/.test(name));
  assert.ok(logName, 'Owned session log missing');
  const logPath = path.join(session.testDirectory, logName);
  await waitUntil(async () => /^VERENU_NATIVE_OBSERVE_END page=\d+ time=\d+$/m.test(await fs.readFile(logPath, 'utf8')),
    'Native manual-reload observation');
  verifyManualReloadObservation(await fs.readFile(logPath, 'utf8'));
  assert.equal(await browser.execute(`return document.querySelector('.pill.paste-failed .paste-failed-text')?.textContent?.trim();`), 'Partly pasted');
  assert.equal(await invoke('get_setting', { key: 'setup_complete' }), true);
  report.checks.push({ name: 'Manual reload cancels pending pill recovery', status: 'passed' });
  await browser.command('POST', '/window', { handle: 'main' });
  assert.equal(await browser.execute(`return document.querySelector('input[aria-label="Search settings"]')?.value;`), 'Synthetic unsaved query');
  await invoke('native_test_pill', { state: 'idle' });
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
  let probe;
  if (process.platform === 'linux') probe = await installLinuxPageProcessProbe(directory);
  const listener = net.createServer();
  await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
  const port = listener.address().port;
  await new Promise((resolve) => listener.close(resolve));
  process.env.TAURI_WEBDRIVER_PORT = String(port);
  const previousPreload = process.env.LD_PRELOAD;
  const previousProbe = process.env.VERENU_NATIVE_WEBKIT_PROBE;
  try {
    if (probe) {
      process.env.VERENU_NATIVE_WEBKIT_PROBE = probe.extensionDirectory;
      process.env.LD_PRELOAD = [probe.preload, previousPreload].filter(Boolean).join(':');
    }
    session = await startOwnedSession({ id: `native-${randomUUID()}`, fixtures: directory, directory, native: true });
  } finally {
    if (previousPreload === undefined) delete process.env.LD_PRELOAD;
    else process.env.LD_PRELOAD = previousPreload;
    if (previousProbe === undefined) delete process.env.VERENU_NATIVE_WEBKIT_PROBE;
    else process.env.VERENU_NATIVE_WEBKIT_PROBE = previousProbe;
  }
  session.testDirectory = directory;
  browser = await NativeDriver.start(port);
  const invoke = async (command, args = {}) => {
    let result;
    try {
      result = await browser.executeAsync((name, values, done) => {
        window.__TAURI__.core.invoke(name, values).then((value) => done({ value }), (error) => done({ error: String(error) }));
      }, command, args);
    } catch (error) {
      throw new Error(`Native Tauri IPC ${command} WebDriver completion failed: ${error.message}`, { cause: error });
    }
    assert.ok(!result.error, `Real native IPC ${command} failed: ${result.error || ''}`);
    return result.value;
  };
  await browser.waitForApp();
  const identity = await browser.execute('return { native: !!window.__TAURI_INTERNALS__, mocks: !!window.__wdio_mocks__ && Object.keys(window.__wdio_mocks__).length > 0 };');
  assert.equal(identity.native, true); assert.equal(identity.mocks, false);
  if (process.platform === 'linux') {
    assert.equal(await browser.execute(`
      const button = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'Settings');
      button?.click(); return !!button;
    `), true);
    const deadline = Date.now() + 15_000;
    while (!await browser.execute('return !!document.querySelector(".appearance-option");')) {
      assert.ok(Date.now() < deadline, 'Appearance settings did not render');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    for (let cycle = 0; cycle < 8; cycle++) {
      for (const label of ['System', 'Tokyo Night', 'Light', 'Dark']) {
        assert.equal(await browser.execute(`
          const label = ${JSON.stringify(label)};
          const button = [...document.querySelectorAll('.appearance-option, .theme-select')].find(b => (b.getAttribute('aria-label') || b.textContent).trim() === label);
          button?.click(); return !!button;
        `), true, `Theme selection missing: ${label}`);
        const expected = label === 'Tokyo Night' ? 'custom' : label.toLowerCase();
        const settled = Date.now() + 10_000;
        while ((await invoke('get_setting', { key: 'appearance_mode' }) ?? 'system') !== expected) {
          assert.ok(Date.now() < settled, `Native theme did not persist: ${label}`);
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(() => done(true))));
        while (await browser.execute('return !!document.querySelector(".appearance-option[aria-disabled=true]");')) {
          assert.ok(Date.now() < settled, `Native theme selection stayed busy: ${label}`);
          await new Promise(resolve => setTimeout(resolve, 100));
        }
      }
    }
    assert.equal(await browser.execute('return document.documentElement.dataset.theme;'), 'dark');
    report.checks.push({ name: '32 real theme selections including System to Custom on native Linux WebKitGTK', status: 'passed' });
    const pillScreenshots = await verifyNativePill({ browser, invoke, directory, screenshot: file => browser.saveScreenshot(file) });
    const renderers = (await linuxProcessTree(session.child.pid)).filter(process => process.name === 'WebKitWebProces');
    const origin = new URL(session.access.localAccessUrl).origin;
    const mainRenderer = await rendererForPage(directory, renderers, origin, 'main');
    const pillRenderer = await rendererForPage(directory, renderers, origin, 'pill');
    if (mainRenderer.pid !== pillRenderer.pid) incomplete('WebKit did not place main and pill in a shared renderer; shared-renderer coverage was not tested');
    report.artifacts.push(...pillScreenshots.map(artifact));
    report.checks.push({ name: 'Native Linux pill content fit, error growth, control input and stale state rejection', status: 'passed' });
    report.checks.push({ name: 'Main and pill share a WebKit renderer', status: 'passed' });
  }
  const created = await invoke('create_context', { name: 'Synthetic native', contextualFormattingDisabled: false });
  report.checks.push({ name: 'Native IPC remains reachable after pill window cycles', status: 'passed' });
  try {
    const previousPage = await markCurrentNativePage(browser);
    await browser.refresh();
    await browser.waitForApp({ navigationMarker: previousPage });
    const contexts = await invoke('get_contexts');
    assert.ok(contexts.some((row) => row.id === created.id && row.name === 'Synthetic native'));
    report.checks.push({ name: 'Native WebView reload waits for a new ready document before follow-up IPC', status: 'passed' });
  } finally {
    try { await invoke('delete_context', { contextId: created.id }); }
    catch (error) { report.checks.push({ name: 'Temporary context cleanup', status: 'failed', reason: error.message }); }
  }
  assert.ok((await browser.getWindowHandles()).length >= 1);
  const viewport = await browser.execute('return { width: window.innerWidth, height: window.innerHeight };');
  assert.ok(Number.isFinite(viewport.width) && viewport.width > 0 && Number.isFinite(viewport.height) && viewport.height > 0,
    `Native WebView content viewport was empty: ${JSON.stringify(viewport)}`);
  const screenshot = path.join(directory, 'native-webview.png'); await browser.saveScreenshot(screenshot);
  report.artifacts.push(artifact(screenshot));
  report.checks.push({ name: 'Actual native WebView, IPC, reload persistence and content viewport geometry', status: 'passed' });
  if (process.platform === 'linux') {
    await verifyLinuxRendererRecovery({ browser, invoke, session });
    report.checks.push({ name: 'Renderer crash recovery and cancellation after manual reload', status: 'passed' });
  }
  report.status = report.checks.some((row) => row.status === 'failed') ? 'failed' : 'verified';
  if (sourceIdentity().fingerprint !== report.identity.fingerprint) { report.status = 'incomplete'; report.reason = 'Source changed during native verification'; }
} catch (error) {
  report.status = error.verificationIncomplete ? 'incomplete' : 'failed'; report.reason = error.message;
  if (error.startupFailure) report.startupFailure = error.startupFailure;
  if (error.identityMismatch) report.identityMismatch = error.identityMismatch;
}
finally {
  if (browser) await browser.deleteSession().catch(() => {});
  if (session) {
    try { await session.stop(); }
    catch (error) {
      report.status = 'failed';
      report.reason = [report.reason, `Could not stop owned session: ${error.message}`].filter(Boolean).join('; ');
    }
  }
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Native WebView verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
