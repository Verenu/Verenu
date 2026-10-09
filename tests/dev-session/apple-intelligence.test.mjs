import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

const access = JSON.parse(await fs.readFile(process.env.VERENU_SESSION_ACCESS_FILE, 'utf8'));
const base = new URL(access.localAccessUrl).origin;
const headers = { Authorization: `Bearer ${access.token}` };
const routePrefix = process.env.VERENU_SESSION_DIRECT_BRIDGE === '1' ? '' : '/__verenu_dev';
async function request(route, init = {}) {
  return fetch(`${base}${routePrefix}${route}`, { ...init, headers: { ...headers, ...init.headers }, signal: AbortSignal.timeout(120_000) });
}
async function invoke(command, args = {}) {
  const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('Apple cleanup uses native availability, preserves selection and completes the production audio pipeline without implicit cloud cleanup', { timeout: 180_000 }, async () => {
  const status = await invoke('get_apple_intelligence_availability');
  assert.equal(typeof status.message, 'string');
  assert.equal(status.available, status.state === 'available');
  const calls = [];
  // Speech is a public synthetic HTTP fixture; Apple cleanup, when available,
  // is real FoundationModels inference through the production pipeline.
  const server = http.createServer(async (req, res) => {
    for await (const _ of req) { /* Drain synthetic audio without retaining it. */ }
    calls.push(req.url);
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ text: 'please send the note tomorrow' }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-4234-8234-123456789017', name: 'Synthetic Apple speech fixture', protocol: 'openai',
    base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: false, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic-speech'], cleanup_models: [],
  };
  const keys = ['custom_providers', 'transcription_default_model', 'transcription_fallback_models', 'cleanup_default_model', 'cleanup_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'cleanup_cache_enabled'];
  const original = await invoke('get_all_settings');
  let context;
  try {
    for (const [key, value] of Object.entries({
      custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic-speech`, transcription_fallback_models: [],
      cleanup_default_model: 'apple-intelligence/system', cleanup_fallback_models: [], dual_transcription_enabled: false,
      cleanup_enabled: true, cleanup_cache_enabled: false,
    })) await invoke('save_setting', { key, value });
    assert.equal(await invoke('get_setting', { key: 'cleanup_default_model' }), 'apple-intelligence/system');
    assert.equal((await invoke('get_api_key_status'))['apple-intelligence'] === true, false);
    context = await invoke('create_context', { name: 'Public Apple cleanup test', tone: 'casual', cleanupIntensity: 'light', contextualFormattingDisabled: true });
    const fixture = await request('/fixtures/plain.wav');
    assert.equal(fixture.status, 200);
    const before = new Set((await invoke('get_recent')).map(row => row.id));
    const cursor = (await (await request('/events?after=0')).json()).cursor;
    const response = await request(`/audio?context=${context.id}&process=synthetic-apple`, { method: 'POST', body: await fixture.arrayBuffer() });
    assert.equal(response.status, 200);
    const result = await response.json();
    assert.equal(result.pipeline, 'production');
    assert.equal(result.text.toLowerCase().replace(/[.!?]/g, ''), 'please send the note tomorrow');
    const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
    assert.equal(rows.length, 1);
    assert.equal(rows[0].clean_text, result.text);
    // RecentEntry intentionally omits provider metadata. Inspect only this
    // owned session's matching persisted row; never the installed database.
    const database = new DatabaseSync(path.join(path.dirname(process.env.VERENU_SESSION_ACCESS_FILE), 'data', 'verenu.db'), { readOnly: true });
    try {
      const history = database.prepare('SELECT clean_text, api_used FROM transcriptions WHERE id = ?').get(rows[0].id);
      assert.equal(history.clean_text, result.text);
      assert.equal(history.api_used.includes('cleanup=apple-intelligence/system'), status.available);
      assert.equal(history.api_used.split(';')[0], `${provider.id}/synthetic-speech/transcription`);
    } finally { database.close(); }
    assert.deepEqual(calls, ['/v1/audio/transcriptions']);
    const events = (await (await request(`/events?after=${cursor}`)).json()).events;
    assert.equal(events.some(event => event.event === 'verenu:transcribed' && event.payload === result.text), true);
    assert.equal(await invoke('get_setting', { key: 'cleanup_default_model' }), 'apple-intelligence/system');
  } finally {
    try {
      if (context) await invoke('delete_context', { contextId: context.id });
      const defaults = { custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: true };
      for (const key of keys) await invoke('save_setting', { key, value: original[key] ?? defaults[key] ?? null });
    } finally {
      await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); });
    }
  }
});
