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
  const body = await response.json();
  assert.equal(response.status, 200, `${command}: ${body?.error ?? ''}`);
  return body;
}

test('dictionary confidence reaches cleanup and controls final substitution', { timeout: 180_000 }, async () => {
  const requests = [];
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/v1/audio/transcriptions') {
      res.end(JSON.stringify({ text: 'Please post-grass clippings.' }));
    } else {
      requests.push(JSON.parse(Buffer.concat(chunks).toString()));
      res.end(JSON.stringify({ choices: [{ message: { content: 'Please post-grass clippings.' } }] }));
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789023', name: 'Synthetic dictionary confidence',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['speech'], cleanup_models: ['cleanup'],
  };
  const settings = {
    custom_providers: [provider], transcription_default_model: `${provider.id}/speech`, cleanup_default_model: `${provider.id}/cleanup`,
    transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false,
    cleanup_enabled: true, cleanup_cache_enabled: false, cleanup_intensity: 'light',
    transcription_language: 'en', voice_commands_enabled: false,
  };
  let original; let context; let entry;
  try {
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    context = await invoke('create_context', { name: 'Synthetic confidence', tone: 'casual', cleanupIntensity: 'light', contextualFormattingDisabled: true });
    entry = await invoke('create_dictionary_entry', { term: 'PostgreSQL', mistake: 'post-grass', contextId: context.id });
    const fixture = await request('/fixtures/plain.wav');
    assert.equal(fixture.status, 200);
    const audio = await fixture.arrayBuffer();
    async function run(expected) {
      const before = new Set((await invoke('get_recent')).map(row => row.id));
      const cursor = (await (await request('/events?after=0')).json()).cursor;
      const response = await request(`/audio?context=${context.id}&process=synthetic-confidence`, { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      const result = await response.json();
      assert.equal(result.pipeline, 'production');
      assert.equal(result.text, expected);
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1);
      assert.equal(rows[0].clean_text, expected);
      const events = (await (await request(`/events?after=${cursor}`)).json()).events;
      assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === expected));
    }
    await run('Please PostgreSQL clippings.');
    await invoke('edit_dictionary_entry', { id: entry.id, contextId: context.id, term: 'PostgreSQL', mistake: 'post-grass', replacementConfidence: 'high', expected: { term: 'PostgreSQL', mistake: 'post-grass', replacement_confidence: 'default' } });
    await run('Please post-grass clippings.');
    const system = requests.at(-1).messages.filter(message => message.role === 'system').map(message => message.content).join('\n');
    assert.ok(system.includes('- preferred: "PostgreSQL"; possible STT variants: "post-grass"; cautious'));
    assert.ok(system.includes("preserve a variant used as a person's name or an ordinary word"));
  } finally {
    try {
      if (entry) await invoke('remove_dictionary_entry', { id: entry.id, contextId: context?.id });
      if (context) await invoke('delete_context', { contextId: context.id });
      for (const key of original ? Object.keys(settings) : []) await invoke('save_setting', { key, value: original[key] ?? settings[key] });
    } finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
});
