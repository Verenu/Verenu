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


export async function runBasicCases(cases) {
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
    for (const [commands, raw, expected] of cases) {
      if (raw.startsWith('greeting')) snippets.push(await invoke('create_snippet', { trigger: 'greeting', expansion: 'um scratch that new line', instructions: '', contextId: context.id }));
      if (raw.startsWith('colon payload')) snippets.push(await invoke('create_snippet', { trigger: 'colon payload', expansion: 'Keep this: scratch that', instructions: '', contextId: context.id }));
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
      if (expected === '') {
        // An entirely rolled-back dictation follows the existing empty-output
        // policy: no text is delivered and no history/completion is saved.
        assert.equal(response.status, 422);
        assert.equal((await response.json()).error, 'Dictation failed or audio was rejected. Inspect session events and redacted logs.');
        assert.equal((await invoke('get_recent')).filter(row => !before.has(row.id)).length, 0);
        const events = (await (await request(`/events?after=${cursor}`)).json()).events;
        assert.ok(!events.some(event => event.event === 'verenu:transcribed'));
        continue;
      }
      assert.equal(response.status, 200);
      const result = await response.json(); assert.equal(result.pipeline, 'production'); assert.equal(result.text, expected);
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1); assert.equal(rows[0].clean_text, expected);
      const events = (await (await request(`/events?after=${cursor}`)).json()).events;
      assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === expected));
    }
    assert.equal(cleanupCalls, 0); assert.equal(speechCalls, cases.length);
  } finally {
    try {
      for (const snippet of snippets) await invoke('remove_snippet', { id: snippet.id });
      for (const entry of vocabulary) await invoke('remove_dictionary_entry', { id: entry.id });
      if (context) await invoke('delete_context', { contextId: context.id });
      for (const key of original ? Object.keys(settings) : []) await invoke('save_setting', { key, value: original[key] ?? ({ custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: true, transcription_language: 'en', voice_commands_enabled: false })[key] ?? null });
    } finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
}

export async function runDualSnippetCases() {
  let primary; let alternate; let cleanupOutput;
  const requests = [];
  let speechCalls = 0;
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString();
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/v1/audio/transcriptions') {
      speechCalls++;
      res.end(JSON.stringify({ text: body.includes('synthetic-alternate') ? alternate : primary }));
    } else {
      requests.push(JSON.parse(body));
      res.end(JSON.stringify({ choices: [{ message: { content: cleanupOutput } }] }));
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789020', name: 'Synthetic dual commands regression',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic-primary', 'synthetic-alternate'], cleanup_models: ['synthetic-cleanup'],
  };
  const settings = {
    custom_providers: [provider], transcription_default_model: `${provider.id}/synthetic-primary`,
    transcription_fallback_models: [`${provider.id}/synthetic-alternate`], dual_transcription_enabled: true,
    cleanup_default_model: `${provider.id}/synthetic-cleanup`, cleanup_fallback_models: [],
    cleanup_enabled: true, cleanup_cache_enabled: false, transcription_language: 'en', voice_commands_enabled: true,
  };
  let original; let context; let snippet;
  try {
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    context = await invoke('create_context', { name: 'Synthetic dual commands', tone: 'casual', cleanupIntensity: 'light', contextualFormattingDisabled: true });
    snippet = await invoke('create_snippet', { trigger: 'signoff, new paragraph', expansion: 'Thanks!', instructions: 'all capitals, no final period', contextId: context.id });
    const audio = await (await request('/fixtures/plain.wav')).arrayBuffer();
    for (const [rawPrimary, rawAlternate, output, expected, admitted] of [
      ['Please write a reply', 'Please write signoff comma tomorrow', 'Please write signoff, tomorrow.', 'Please write signoff, tomorrow.', false],
      ['Please write a reply', 'Please write new paragraph comma tomorrow', 'Please write new paragraph, tomorrow.', 'Please write new paragraph, tomorrow.', false],
      ['Please write signoff', 'Please write signoff comma tomorrow', 'Thanks! tomorrow.', 'THANKS! TOMORROW', true],
    ]) {
      primary = rawPrimary; alternate = rawAlternate; cleanupOutput = output;
      const before = new Set((await invoke('get_recent')).map(row => row.id));
      const cursor = (await (await request('/events?after=0')).json()).cursor;
      const response = await request(`/audio?context=${context.id}&process=synthetic-dual`, { method: 'POST', body: audio });
      assert.equal(response.status, 200);
      const result = await response.json();
      assert.equal(result.pipeline, 'production'); assert.equal(result.text, expected);
      const prompt = JSON.stringify(requests.at(-1).messages);
      assert.equal(prompt.includes('Thanks!'), admitted, 'only primary-admitted snippet payload may enter fusion');
      assert.equal(prompt.includes('all capitals, no final period'), admitted, 'only admitted instructions constrain output');
      if (!admitted) assert.ok(prompt.includes(rawAlternate.replace(' comma', ',')), 'alternate commands process surrounding speech without interpreting snippet aliases');
      const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
      assert.equal(rows.length, 1); assert.equal(rows[0].clean_text, expected);
      const events = (await (await request(`/events?after=${cursor}`)).json()).events;
      assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === expected));
    }
    assert.equal(speechCalls, 6); assert.equal(requests.length, 3);
  } finally {
    try {
      if (snippet) await invoke('remove_snippet', { id: snippet.id });
      if (context) await invoke('delete_context', { contextId: context.id });
      for (const key of original ? Object.keys(settings) : []) await invoke('save_setting', { key, value: original[key] ?? ({ custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, cleanup_cache_enabled: true, transcription_language: 'en', voice_commands_enabled: false })[key] ?? null });
    } finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
}
