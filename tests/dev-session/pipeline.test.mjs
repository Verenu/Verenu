import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';

const access = JSON.parse(await fs.readFile(process.env.VERENU_SESSION_ACCESS_FILE, 'utf8'));
const base = new URL(access.localAccessUrl).origin;
const headers = { Authorization: `Bearer ${access.token}` };
async function request(route, init = {}) {
  return fetch(`${base}/__verenu_dev${route}`, { ...init, headers: { ...headers, ...init.headers }, signal: AbortSignal.timeout(120_000) });
}
async function invoke(command, args = {}) {
  const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('production pipeline uses HTTP fallback and preserves speech when cleanup fails', { timeout: 180_000 }, async () => {
  const calls = [];
  let cleanupFails = false;
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString();
    const transcription = req.url === '/v1/audio/transcriptions';
    const primary = transcription && body.includes('synthetic-primary');
    calls.push({ transcription, primary, contextApplied: !transcription && body.includes('Synthetic HTTP instruction') });
    res.setHeader('Content-Type', 'application/json');
    if (primary || (!transcription && cleanupFails)) {
      res.statusCode = 503;
      res.end(JSON.stringify({ error: { message: 'Public synthetic provider outage' } }));
    } else if (transcription) res.end(JSON.stringify({ text: 'Public synthetic fallback speech.' }));
    else res.end(JSON.stringify({ choices: [{ message: { content: 'Public synthetic cleaned speech.' } }] }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789012', name: 'Synthetic HTTP regression',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null,
    extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic-primary', 'synthetic-fallback'], cleanup_models: ['synthetic-cleanup'],
  };
  const keys = ['custom_providers', 'transcription_default_model', 'transcription_fallback_models', 'cleanup_default_model', 'cleanup_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'cleanup_cache_enabled'];
  let original;
  let context;
  try {
    original = await invoke('get_all_settings');
    const settings = {
      custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic-primary`,
      transcription_fallback_models: [`${provider.id}/synthetic-fallback`],
      cleanup_default_model: `${provider.id}/synthetic-cleanup`, cleanup_fallback_models: [],
      dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: false,
    };
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    context = await invoke('create_context', { name: `HTTP flow ${Date.now()}`, tone: 'casual', cleanupIntensity: 'medium', customInstructions: 'Synthetic HTTP instruction', contextualFormattingDisabled: true });
    const fixture = await request('/fixtures/plain.wav');
    assert.equal(fixture.status, 200);
    const audio = await fixture.arrayBuffer();
    for (const [fails, expected] of [[false, 'Public synthetic cleaned speech.'], [true, 'Public synthetic fallback speech.']]) {
      cleanupFails = fails;
      const before = new Set((await invoke('get_recent')).map(row => row.id));
      const cursor = (await (await request('/events?after=0')).json()).cursor;
      const response = await request(`/audio?context=${context.id}&process=synthetic-http`, { method: 'POST', body: audio });
      assert.equal(response.status, 200, 'Production pipeline must complete despite the primary provider outage');
      const result = await response.json();
      assert.equal(result.pipeline, 'production');
      assert.equal(result.text, expected);
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1, 'Every completed production run must create exactly one new history row');
      assert.equal(rows[0].clean_text, expected);
      const events = (await (await request(`/events?after=${cursor}`)).json()).events;
      assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === expected));
    }
    assert.ok(calls.some(row => row.primary), 'Primary transcription must actually reach HTTP');
    assert.ok(calls.some(row => row.transcription && !row.primary), 'Fallback must actually reach HTTP');
    assert.ok(calls.some(row => !row.transcription), 'Cleanup must actually reach HTTP');
    assert.ok(calls.some(row => row.contextApplied), 'The production cleanup request must include the selected Context instructions');
  } finally {
    try {
      for (const key of original ? keys : []) {
        const defaults = { custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: true };
        await invoke('save_setting', { key, value: original[key] ?? defaults[key] ?? null });
      }
      if (context) await invoke('delete_context', { contextId: context.id });
    } finally {
      await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); });
    }
  }
});
