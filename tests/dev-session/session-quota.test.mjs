import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';

const access = JSON.parse(await fs.readFile(process.env.VERENU_SESSION_ACCESS_FILE, 'utf8'));
const base = new URL(access.localAccessUrl).origin;
async function request(route, init = {}) {
  return fetch(`${base}/__verenu_dev${route}`, { ...init, headers: { Authorization: `Bearer ${access.token}`, ...init.headers }, signal: AbortSignal.timeout(120_000) });
}
async function invoke(command, args = {}) {
  const response = await request('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }) });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('default session admits 30 audio attempts and rejects the 31st without provider or history effects', { timeout: 180_000 }, async () => {
  const initialResponse = await request('/session');
  assert.equal(initialResponse.status, 200);
  const initial = await initialResponse.json();
  assert.equal(initial.maxRuns, 30);
  assert.equal(initial.runs, 0);
  let calls = 0;
  const server = http.createServer(async (req, res) => {
    calls++;
    for await (const chunk of req) { void chunk; }
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify({ text: 'Unexpected provider call.' }));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = { id: 'custom:87654321-1234-4234-8234-123456789021', name: 'Synthetic quota fixture', protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false, supports_transcription: true, supports_cleanup: false, auth_header: null, extra_headers: {}, body_overrides: null, transcription_models: ['synthetic-speech'], cleanup_models: [] };
  const settings = { custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic-speech`, transcription_fallback_models: [], dual_transcription_enabled: false };
  let original;
  try {
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    const history = await invoke('get_recent');
    const fixture = await request('/fixtures/silence.wav');
    assert.equal(fixture.status, 200);
    const silence = await fixture.arrayBuffer();
    for (let i = 0; i < 30; i++) {
      const response = await request('/audio', { method: 'POST', body: silence });
      assert.equal(response.status, 422);
      await response.arrayBuffer();
    }
    const cursor = (await (await request('/events?after=0')).json()).cursor;
    // Use valid speech for the rejected request: the quota must prevent the
    // provider request that this audio would otherwise trigger.
    const speech = await request('/fixtures/plain.wav');
    assert.equal(speech.status, 200);
    const rejected = await request('/audio', { method: 'POST', body: await speech.arrayBuffer() });
    assert.equal(rejected.status, 429);
    assert.equal((await rejected.json()).error, 'Session live-run limit reached. Start a new session or raise --max-runs explicitly.');
    const ready = await request('/session');
    assert.equal(ready.status, 200);
    const final = await ready.json();
    assert.equal(final.runs, 30);
    assert.equal(final.maxRuns, 30);
    assert.equal(final.fingerprint, initial.fingerprint);
    assert.equal(calls, 0);
    assert.deepEqual(await invoke('get_recent'), history);
    const events = (await (await request(`/events?after=${cursor}`)).json()).events;
    assert.ok(!events.some(event => ['verenu:transcribed', 'verenu:pill-state'].includes(event.event)));
  } finally {
    try { for (const key of original ? Object.keys(settings) : []) await invoke('save_setting', { key, value: original[key] ?? ({ custom_providers: [], transcription_fallback_models: [], dual_transcription_enabled: false })[key] ?? null }); }
    finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
});
