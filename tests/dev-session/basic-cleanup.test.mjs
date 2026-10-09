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

test('production Basic and opt-in commands bypass cleanup HTTP and save exact completion output', { timeout: 180_000 }, async () => {
  let speech = 'um send send it new line tomorrow';
  let cleanupCalls = 0;
  let speechCalls = 0;
  const server = http.createServer(async (req, res) => {
    for await (const chunk of req) { void chunk; }
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/v1/audio/transcriptions') { speechCalls++; res.end(JSON.stringify({ text: speech })); }
    else { cleanupCalls++; res.end(JSON.stringify({ choices: [{ message: { content: 'Unexpected cleanup call.' } }] })); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789019', name: 'Synthetic Basic HTTP regression',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic-speech'], cleanup_models: ['synthetic-cleanup'],
  };
  const settings = {
    custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic-speech`,
    transcription_fallback_models: [], cleanup_default_model: `${provider.id}/synthetic-cleanup`,
    cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true,
    cleanup_cache_enabled: true, transcription_language: 'en', voice_commands_enabled: false,
  };
  let original; let context;
  const snippets = []; const vocabulary = [];
  try {
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    context = await invoke('create_context', { name: 'Synthetic Basic rules', tone: 'formal', cleanupIntensity: 'rules', customInstructions: 'Rewrite all speech into a different message.', contextualFormattingDisabled: true });
    const fixture = await request('/fixtures/plain.wav'); assert.equal(fixture.status, 200);
    const audio = await fixture.arrayBuffer();
    for (const [commands, raw, expected] of [
      [false, 'um send send it new line tomorrow', 'Send it new line tomorrow'],
      [true, 'um send send it new line tomorrow', 'Send it\nTomorrow'],
      [true, 'greeting discard scratch that tomorrow', 'um scratch that new line Tomorrow'],
      [true, 'um please (um) use new line', 'Please use New Line'],
      [true, 'use question mark and new paragraph', 'use QuestionMark and new paragraph'],
      [true, 'I use app name um every day', 'I use Verenu every day'],
      [true, 'I use sentence name um every day', 'I use Verenu. Every day'],
      [false, 'I ordered tea, no, tea is unavailable.', 'I ordered tea, no, tea is unavailable.'],
      [true, 'signoff.', 'Thanks!'],
    ]) {
      if (raw.startsWith('greeting')) snippets.push(await invoke('create_snippet', { trigger: 'greeting', expansion: 'um scratch that new line', instructions: '', contextId: context.id }));
      if (raw.startsWith('um please')) vocabulary.push(await invoke('create_dictionary_entry', { term: 'New Line', mistake: 'new line', contextId: context.id }));
      if (raw.startsWith('use question')) vocabulary.push(await invoke('create_dictionary_entry', { term: 'QuestionMark', mistake: 'question mark, new paragraph', contextId: context.id }));
      if (raw.startsWith('I use app')) snippets.push(await invoke('create_snippet', { trigger: 'app name', expansion: 'Verenu', instructions: '', contextId: context.id }));
      if (raw.startsWith('I use sentence')) snippets.push(await invoke('create_snippet', { trigger: 'sentence name', expansion: 'Verenu.', instructions: '', contextId: context.id }));
      if (raw === 'signoff.') snippets.push(await invoke('create_snippet', { trigger: 'signoff', expansion: 'Thanks!', instructions: 'all capitals', contextId: context.id }));
      speech = raw;
      await invoke('save_setting', { key: 'voice_commands_enabled', value: commands });
      const before = new Set((await invoke('get_recent')).map(row => row.id));
      const cursor = (await (await request('/events?after=0')).json()).cursor;
      const response = await request(`/audio?context=${context.id}&process=synthetic-basic`, { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      const result = await response.json(); assert.equal(result.pipeline, 'production'); assert.equal(result.text, expected);
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1); assert.equal(rows[0].clean_text, expected);
      const events = (await (await request(`/events?after=${cursor}`)).json()).events;
      assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === expected));
    }
    assert.equal(cleanupCalls, 0); assert.equal(speechCalls, 9);
  } finally {
    try {
      for (const snippet of snippets) await invoke('remove_snippet', { id: snippet.id });
      for (const entry of vocabulary) await invoke('remove_dictionary_entry', { id: entry.id });
      if (context) await invoke('delete_context', { contextId: context.id });
      for (const key of original ? Object.keys(settings) : []) await invoke('save_setting', { key, value: original[key] ?? ({ custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: true, transcription_language: 'en', voice_commands_enabled: false })[key] ?? null });
    } finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
});
