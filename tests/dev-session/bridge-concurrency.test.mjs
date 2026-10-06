import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to a running dev session access.json.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const base = new URL(access.localAccessUrl).origin;

test('concurrent appearance updates keep UI-thread icon handling and worker alive', { timeout: 60_000 }, async () => {
  async function invoke(command, args = {}) {
    const response = await fetch(`${base}/__verenu_dev/invoke`, {
      method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(10_000),
    });
    assert.equal(response.status, 200, command);
    return response.json();
  }
  const before = await invoke('get_setting', { key: 'appearance_mode' });
  try {
    for (let batch = 0; batch < 24; batch++) {
      await Promise.all(['light', 'dark', 'system'].map(value => invoke('save_setting', { key: 'appearance_mode', value })));
      // Let the UI queue process native icon callbacks before the next batch.
      await invoke('get_setting', { key: 'appearance_mode' });
    }
    const response = await fetch(`${base}/__verenu_dev/session`, { headers: { Authorization: `Bearer ${access.token}` }, signal: AbortSignal.timeout(10_000) });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).transport, 'rust-live');
  } finally { await invoke('save_setting', { key: 'appearance_mode', value: before ?? 'system' }); }
});

test('concurrent browser polling and native commands keep the worker alive', { timeout: 60_000 }, async () => {
  async function request(route, body) {
    const response = await fetch(`${base}/__verenu_dev/${route}`, {
      method: body ? 'POST' : 'GET',
      headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(10_000),
    });
    assert.equal(response.status, 200, route);
    await response.json();
  }
  // Separate browser tabs poll and subscribe while settings issue native IPC.
  // Tauri runtime state must not be cloned concurrently by Axum's workers.
  for (let batch = 0; batch < 40; batch++) {
    await Promise.all(Array.from({ length: 32 }, (_, index) => {
      if (index % 3 === 0) return request('events?after=0');
      if (index % 3 === 1) return request('listen', { event: 'verenu:transcribed' });
      return request('invoke', { command: 'get_default_cleanup_prompt', args: { intensity: 'light' } });
    }));
  }
  await request('session');
});
