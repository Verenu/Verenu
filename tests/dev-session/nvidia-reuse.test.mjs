import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import { chromium } from 'playwright';

const accessPath = process.env.VERENU_SESSION_ACCESS_FILE;
const runtime = process.env.VERENU_REUSE_TEST_RUNTIME;

test('advanced noise reduction is visible at desktop and phone widths', { skip: !accessPath, timeout: 90_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const response = await fetch(`${new URL(access.localAccessUrl).origin}/__verenu_dev/invoke`, {
    method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command: 'nvidia_reuse_check', args: {} }),
  });
  assert.equal(response.status, 200);
  const statusResponse = await fetch(`${new URL(access.localAccessUrl).origin}/__verenu_dev/invoke`, {
    method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command: 'nvidia_reuse_status', args: {} }),
  });
  const status = await statusResponse.json();
  const browser = await chromium.launch({ headless: true });
  try {
    for (const [name, viewport] of [['desktop', { width: 1320, height: 860 }], ['phone', { width: 390, height: 844 }]]) {
      const page = await browser.newPage({ viewport: { width: 1320, height: 860 } });
      await page.goto(access.localAccessUrl);
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
      await page.getByRole('button', { name: 'Audio', exact: true }).click();
      await page.setViewportSize(viewport);
      const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
      if (status.state === 'unavailable' && !status.has_files) {
        await section.waitFor({ state: 'hidden' });
        continue;
      }
      if (status.state === 'unavailable') {
        await section.getByText('Downloaded NVIDIA RE-USE files').waitFor();
        assert.equal(await section.getByRole('switch', { name: 'Use NVIDIA RE-USE' }).count(), 0);
      }
      await section.getByText('NVIDIA RE-USE', { exact: false }).first().waitFor();
      const box = await section.boundingBox();
      assert.ok(box && box.x >= 0 && box.x + box.width <= viewport.width, `${name} setting must fit the viewport`);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      await section.screenshot({ path: path.join(path.dirname(accessPath), `nvidia-reuse-${name}.png`) });
      await page.close();
    }
  } finally { await browser.close(); }
});

test('downloaded files can be removed after the GPU becomes unavailable', { skip: !accessPath, timeout: 90_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  let hasFiles = true;
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      const { command } = route.request().postDataJSON();
      if (command === 'nvidia_reuse_status') {
        await route.fulfill({ json: { state: 'unavailable', message: 'No compatible GPU', has_files: hasFiles } });
      } else if (command === 'nvidia_reuse_remove') {
        hasFiles = false;
        await route.fulfill({ json: null });
      } else await route.continue();
    });
    await page.goto(access.localAccessUrl);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Audio', exact: true }).click();
    const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
    await section.getByText('Downloaded NVIDIA RE-USE files').waitFor();
    assert.equal(await section.getByRole('switch', { name: 'Use NVIDIA RE-USE' }).count(), 0);
    await section.getByRole('button', { name: 'Delete downloaded files' }).click();
    await section.getByRole('button', { name: 'Delete files', exact: true }).click();
    await section.waitFor({ state: 'hidden' });
    assert.equal(hasFiles, false);
  } finally { await browser.close(); }
});

test('turning on NVIDIA RE-USE starts setup and enables it when setup succeeds', { skip: !accessPath, timeout: 90_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const endpoint = `${new URL(access.localAccessUrl).origin}/__verenu_dev/invoke`;
  const invoke = async (command, args = {}) => {
    const response = await fetch(endpoint, { method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
    assert.equal(response.status, 200);
    return response.json();
  };
  const original = await invoke('get_all_settings');
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  let ready = false;
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      const { command } = route.request().postDataJSON();
      if (command === 'nvidia_reuse_status') {
        await route.fulfill({ json: ready ? { state: 'ready', message: 'Ready', has_files: true } : { state: 'not_installed', message: 'Download required', has_files: false } });
      } else if (command === 'nvidia_reuse_install') {
        ready = true;
        await route.fulfill({ json: null });
      } else if (command === 'nvidia_reuse_remove') {
        await route.fulfill({ status: 500, json: { error: 'Synthetic deletion failure' } });
      } else await route.continue();
    });
    await page.goto(access.localAccessUrl);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Audio', exact: true }).click();
    const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
    const toggle = section.getByRole('switch', { name: 'Use NVIDIA RE-USE' });
    await toggle.click();
    await page.waitForFunction(() => document.querySelector('[aria-label="Use NVIDIA RE-USE"]')?.getAttribute('aria-checked') === 'true');
    assert.equal(ready, true);
    await section.getByRole('button', { name: 'Delete downloaded files' }).click();
    await section.getByRole('button', { name: 'Delete files', exact: true }).click();
    await section.getByText('Synthetic deletion failure').waitFor();
    assert.equal(await section.getByText('Could not delete runtime').count(), 1);
    assert.equal(await toggle.getAttribute('aria-checked'), 'false');
  } finally {
    await browser.close();
    await invoke('save_setting', { key: 'noise_reduction', value: original.noise_reduction ?? true });
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: original.nvidia_reuse_enabled ?? false });
  }
});

test('real-time setup can be retried without disabling the existing NVIDIA model', { skip: !accessPath, timeout: 90_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const endpoint = `${new URL(access.localAccessUrl).origin}/__verenu_dev/invoke`;
  const invoke = async (command, args = {}) => {
    const response = await fetch(endpoint, { method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
    assert.equal(response.status, 200);
    return response.json();
  };
  const original = await invoke('get_all_settings');
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  let ready = false;
  let retries = 0;
  try {
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: true });
    await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: true });
    await page.route('**/__verenu_dev/invoke', async route => {
      const { command } = route.request().postDataJSON();
      if (command === 'nvidia_reuse_status') await route.fulfill({ json: { state: 'ready', message: 'Ready', has_files: true } });
      else if (command === 'nvidia_reuse_realtime_ready') await route.fulfill({ json: ready });
      else if (command === 'nvidia_reuse_realtime_install') { retries++; ready = true; await route.fulfill({ json: null }); }
      else await route.continue();
    });
    await page.goto(access.localAccessUrl);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Audio', exact: true }).click();
    const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
    await section.getByText('Real-time model unavailable').waitFor();
    await page.locator('[data-setting-target="audio-noise"]').getByText('Suppress background noise before transcription (RNNoise)').waitFor();
    assert.equal(await section.getByRole('switch', { name: 'Use NVIDIA RE-USE' }).getAttribute('aria-checked'), 'true');
    await section.getByRole('button', { name: 'Retry setup' }).click();
    await section.getByText('Real-time model unavailable').waitFor({ state: 'hidden' });
    await page.locator('[data-setting-target="audio-noise"]').getByText('Enhance audio before transcription with NVIDIA RE-USE').waitFor();
    assert.equal(retries, 1);
  } finally {
    await browser.close();
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: original.nvidia_reuse_enabled ?? false });
    await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: original.nvidia_reuse_realtime_enabled ?? false });
  }
});

test('base runtime retry still runs when real-time is enabled', { skip: !accessPath, timeout: 90_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const endpoint = `${new URL(access.localAccessUrl).origin}/__verenu_dev/invoke`;
  const invoke = async (command, args = {}) => {
    const response = await fetch(endpoint, { method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
    assert.equal(response.status, 200);
    return response.json();
  };
  const original = await invoke('get_all_settings');
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  let baseStatus = 'ready';
  let baseRetries = 0;
  let realtimeRetries = 0;
  try {
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: true });
    await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: true });
    await page.route('**/__verenu_dev/invoke', async route => {
      const { command } = route.request().postDataJSON();
      if (command === 'nvidia_reuse_status') await route.fulfill({ json: { state: baseStatus, message: 'Synthetic runtime failure', has_files: true } });
      else if (command === 'nvidia_reuse_realtime_ready') await route.fulfill({ json: true });
      else if (command === 'nvidia_reuse_install') { baseRetries++; baseStatus = 'ready'; await route.fulfill({ json: null }); }
      else if (command === 'nvidia_reuse_realtime_install') { realtimeRetries++; await route.fulfill({ json: null }); }
      else await route.continue();
    });
    await page.goto(access.localAccessUrl);
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('button', { name: 'Audio', exact: true }).click();
    const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
    await section.getByRole('switch', { name: 'Use real-time RE-USE for research' }).waitFor();
    baseStatus = 'error';
    await section.getByText('NVIDIA RE-USE needs attention').waitFor({ timeout: 20000 });
    await section.getByRole('button', { name: 'Retry setup' }).click();
    await section.getByText('NVIDIA RE-USE needs attention').waitFor({ state: 'hidden' });
    assert.equal(baseRetries, 1);
    assert.equal(realtimeRetries, 0);
  } finally {
    await browser.close();
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: original.nvidia_reuse_enabled ?? false });
    await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: original.nvidia_reuse_realtime_enabled ?? false });
  }
});

test('NVIDIA RE-USE reports availability and preserves audio when the worker fails', { skip: !accessPath, timeout: 300_000 }, async () => {
  const access = JSON.parse(await fs.readFile(accessPath, 'utf8'));
  const base = new URL(access.localAccessUrl).origin;
  const headers = { Authorization: `Bearer ${access.token}` };
  const request = (route, init = {}) => fetch(`${base}/__verenu_dev${route}`, { ...init, headers: { ...headers, ...init.headers }, signal: AbortSignal.timeout(180_000) });
  async function invoke(command, args = {}) {
    const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
    assert.equal(response.status, 200, command);
    return response.json();
  }
  if (!runtime) {
    await invoke('nvidia_reuse_check');
    let status = await invoke('nvidia_reuse_status');
    const deadline = Date.now() + 120_000;
    while (status.state === 'checking' && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 500));
      status = await invoke('nvidia_reuse_status');
    }
    assert.ok(['unavailable', 'not_installed'].includes(status.state));
    return;
  }
  assert.equal(path.resolve(runtime), path.join(path.dirname(accessPath), 'data', 'runtimes', 'nvidia-reuse-v1'));
  const received = [];
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks);
    const start = body.indexOf('RIFF');
    if (start >= 0) received.push(body.subarray(start, start + 8 + body.readUInt32LE(start + 4)));
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ text: 'Synthetic speech was transcribed.' }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789012', name: 'Synthetic RE-USE test',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: false, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic'], cleanup_models: [],
  };
  const keys = ['custom_providers', 'transcription_default_model', 'transcription_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'noise_reduction', 'nvidia_reuse_enabled', 'nvidia_reuse_realtime_enabled', 'local_model_memory_policy'];
  const worker = path.join(runtime, 'worker.py');
  const disabledWorker = `${worker}.test-disabled`;
  let workerDisabled = false;
  let original;
  try {
    await invoke('nvidia_reuse_check');
    let state = (await invoke('nvidia_reuse_status')).state;
    const deadline = Date.now() + 120_000;
    while (state === 'checking' && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 500));
      state = (await invoke('nvidia_reuse_status')).state;
    }
    assert.equal(state, 'ready');
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries({ custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic`, transcription_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: false, noise_reduction: true })) {
      await invoke('save_setting', { key, value });
    }
    const audioResponse = await request('/fixtures/plain.wav');
    assert.equal(audioResponse.status, 200);
    const audio = await audioResponse.arrayBuffer();
    async function run() {
      const response = await request('/audio?process=synthetic-reuse', { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      assert.equal((await response.json()).text, 'Synthetic speech was transcribed.');
    }
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: false });
    await run();
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: true });
    assert.equal((await invoke('get_all_settings')).nvidia_reuse_enabled, true);
    await run();
    assert.equal(received.length, 2);
    assert.notDeepEqual(received[1], received[0], 'The GPU model must change the audio sent to the provider');
    if (process.env.VERENU_REUSE_REALTIME_TEST === '1') {
      assert.equal(await invoke('nvidia_reuse_realtime_ready'), true);
      await invoke('save_setting', { key: 'local_model_memory_policy', value: 'unload_immediately' });
      await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: true });
      await run();
      assert.notDeepEqual(received[2], received[0], 'The real-time GPU model must change the audio sent to the provider');
      const realtimeWorker = path.join(runtime, 'realtime_worker.py');
      const disabledRealtimeWorker = `${realtimeWorker}.test-disabled`;
      await fs.rename(realtimeWorker, disabledRealtimeWorker);
      try {
        await fs.writeFile(realtimeWorker, 'raise RuntimeError("synthetic worker failure")\n');
        await run();
        assert.deepEqual(received[3], received[0], 'A failed real-time worker must preserve the original recording');
        assert.equal((await invoke('nvidia_reuse_status')).state, 'ready', 'A real-time failure must not disable the existing model');
        assert.equal(await invoke('nvidia_reuse_realtime_ready'), false, 'A failed real-time worker must require setup again');
      } finally {
        await fs.rm(realtimeWorker, { force: true });
        await fs.rename(disabledRealtimeWorker, realtimeWorker);
      }
      await invoke('save_setting', { key: 'nvidia_reuse_realtime_enabled', value: false });
      assert.equal((await invoke('nvidia_reuse_status')).state, 'ready');
      await run();
      assert.notDeepEqual(received.at(-1), received[0], 'The existing model must still enhance audio after a real-time failure');
    }
    await fs.rename(worker, disabledWorker);
    workerDisabled = true;
    await run();
    assert.deepEqual(received.at(-1), received[0], 'A failed worker must preserve the original recording');
    assert.equal((await invoke('nvidia_reuse_status')).state, 'error');
  } finally {
    if (workerDisabled) await fs.rename(disabledWorker, worker);
    if (original) for (const key of keys) {
      const defaults = { custom_providers: [], transcription_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, noise_reduction: true, nvidia_reuse_enabled: false, nvidia_reuse_realtime_enabled: false, local_model_memory_policy: 'unload_after_5m' };
      await invoke('save_setting', { key, value: original[key] ?? defaults[key] ?? null });
    }
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
  if (process.env.VERENU_REUSE_REMOVE_TEST === '1') {
    assert.equal(path.resolve(runtime), path.join(path.dirname(accessPath), 'data', 'runtimes', 'nvidia-reuse-v1'));
    await invoke('nvidia_reuse_check');
    assert.equal((await invoke('nvidia_reuse_status')).has_files, true);
    await invoke('save_setting', { key: 'nvidia_reuse_enabled', value: true });
    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();
      await page.goto(access.localAccessUrl);
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
      await page.getByRole('button', { name: 'Audio', exact: true }).click();
      const section = page.locator('[data-setting-target="audio-nvidia-reuse"]');
      await section.getByRole('button', { name: 'Delete downloaded files' }).click();
      await section.getByRole('button', { name: 'Delete files', exact: true }).click();
      const deadline = Date.now() + 120_000;
      let after = await invoke('nvidia_reuse_status');
      while (after.has_files && Date.now() < deadline) {
        await new Promise(resolve => setTimeout(resolve, 250));
        after = await invoke('nvidia_reuse_status');
      }
      assert.equal(after.has_files, false);
      assert.equal((await invoke('get_all_settings')).nvidia_reuse_enabled, false);
      await assert.rejects(fs.access(runtime));
    } finally { await browser.close(); }
  }
});
