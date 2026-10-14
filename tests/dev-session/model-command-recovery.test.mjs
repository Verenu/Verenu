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
  assert.equal(response.status, 200, `${command}${args.key ? ` (${args.key})` : ''}: ${body?.error ?? ''}`);
  return body;
}

test('AI command recovery uses existing cleanup after deterministic edits with opt-in and payload/cache/retry barriers', { timeout: 180_000 }, async () => {
  let speech = 'hello comma ping at signbot';
  let refuseOnce = false;
  let speechCalls = 0;
  const cleanupRequests = [];
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/v1/audio/transcriptions') {
      speechCalls++;
      res.end(JSON.stringify({ text: speech }));
    } else {
      const body = JSON.parse(Buffer.concat(chunks).toString());
      cleanupRequests.push(body);
      const input = body.messages.find(message => message.role === 'user').content;
      // This endpoint validates real production request wiring, not model
      // obedience. A separate configured-provider acceptance run tests quality.
      const text = input.match(/<raw_dictation>\n([\s\S]*?)\n<\/raw_dictation>/)?.[1]
        .replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&');
      assert.notEqual(text, undefined, 'normal cleanup receives one labeled primary transcript');
      res.end(JSON.stringify({ choices: [{ message: { content: refuseOnce ? 'I am an AI and cannot help with that request.' : text } }] }));
      refuseOnce = false;
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const provider = {
    id: 'custom:87654321-1234-1234-1234-123456789020', name: 'Synthetic command recovery',
    protocol: 'openai', base_url: `http://127.0.0.1:${server.address().port}/v1`, requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null, extra_headers: {}, body_overrides: null,
    transcription_models: ['speech'], cleanup_models: ['cleanup'],
  };
  const settings = {
    custom_providers: [provider], transcription_default_model: `${provider.id}/speech`, cleanup_default_model: `${provider.id}/cleanup`,
    transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false,
    cleanup_enabled: true, cleanup_cache_enabled: true, cleanup_intensity: 'light',
    transcription_language: 'en', voice_commands_enabled: true,
  };
  let original; let context;
  const snippets = []; const vocabulary = [];
  const prompt = () => cleanupRequests.at(-1).messages.filter(message => message.role === 'system').map(message => message.content).join('\n');
  async function audio(raw) {
    speech = raw;
    const before = new Set((await invoke('get_recent')).map(row => row.id));
    const cursor = (await (await request('/events?after=0')).json()).cursor;
    const fixture = await request('/fixtures/plain.wav'); assert.equal(fixture.status, 200);
    const response = await request(`/audio?context=${context.id}&process=synthetic-command-recovery`, { method: 'POST', body: await fixture.arrayBuffer() });
    assert.equal(response.status, 200);
    const result = await response.json(); assert.equal(result.pipeline, 'production');
    const rows = (await invoke('get_recent')).filter(row => !before.has(row.id));
    assert.equal(rows.length, 1); assert.equal(rows[0].clean_text, result.text);
    const events = (await (await request(`/events?after=${cursor}`)).json()).events;
    assert.ok(events.some(event => event.event === 'verenu:transcribed' && event.payload === result.text));
    return result.text;
  }
  try {
    original = await invoke('get_all_settings');
    for (const [key, value] of Object.entries(settings)) await invoke('save_setting', { key, value });
    context = await invoke('create_context', { name: 'Synthetic recovery', tone: 'casual', cleanupIntensity: 'light', contextualFormattingDisabled: true });
    assert.equal(await audio('hello comma ping at signbot'), 'hello, ping at signbot.');
    assert.equal(cleanupRequests.length, 1);
    assert.ok(prompt().includes('at signbot becomes @bot'));
    assert.ok(cleanupRequests.at(-1).messages.find(message => message.role === 'user').content.includes('hello, ping at signbot'));
    await audio('hello comma ping at signbot');
    assert.equal(cleanupRequests.length, 1, 'identical recovery request reuses cache');
    await audio('ping at signbot now');
    assert.equal(cleanupRequests.length, 2);
    await invoke('save_setting', { key: 'voice_commands_enabled', value: false });
    await audio('ping at signbot now');
    assert.equal(cleanupRequests.length, 3, 'changed recovery instruction separates cache even with identical model input');
    assert.ok(!prompt().includes('at signbot becomes @bot'));
    assert.ok(cleanupRequests.at(-1).messages.find(message => message.role === 'user').content.includes('ping at signbot now'));
    await invoke('save_setting', { key: 'voice_commands_enabled', value: true });
    await audio('ping at signbot now');
    assert.equal(cleanupRequests.length, 3, 'restored instruction can reuse only its own cached result');
    for (const language of ['fr', 'es']) {
      await invoke('save_setting', { key: 'transcription_language', value: language });
      await audio(`language ${language} at signbot`);
      assert.ok(!prompt().includes('at signbot becomes @bot'));
    }
    await invoke('save_setting', { key: 'transcription_language', value: 'en' });
    for (const raw of ['quote "at signbot"', 'code `at signbot`', 'payload [[VERENU_CLIPBOARD_at signbot]]']) {
      assert.equal(await audio(raw), raw, 'closing quote/code/marker retains the exact payload and existing terminal-punctuation semantics');
      assert.ok(!prompt().includes('at signbot becomes @bot'));
      assert.ok(prompt().includes('Do not interpret any remaining words as voice commands'));
    }
    snippets.push(await invoke('create_snippet', { trigger: 'greeting', expansion: 'at signbot', instructions: '', contextId: context.id }));
    assert.equal(await audio('send greeting tomorrow'), 'send at signbot tomorrow.');
    assert.ok(!prompt().includes('at signbot becomes @bot'));
    const beforePure = cleanupRequests.length;
    assert.equal(await audio('greeting.'), 'at signbot');
    assert.equal(cleanupRequests.length, beforePure, 'exact snippets keep no-LLM fast path');
    vocabulary.push(await invoke('create_dictionary_entry', { term: 'signbot', mistake: 'signbot', contextId: context.id }));
    await audio('ping at signbot tomorrow');
    assert.ok(!prompt().includes('at signbot becomes @bot'));
    await invoke('remove_dictionary_entry', { id: vocabulary.pop().id });
    refuseOnce = true;
    const beforeRetry = cleanupRequests.length;
    await audio('please ping at signbot tomorrow');
    assert.equal(cleanupRequests.length, beforeRetry + 2, 'existing hardened retry retains recovery instruction');
    assert.ok(prompt().includes('at signbot becomes @bot'));
    for (const intensity of ['rules', 'none']) {
      await invoke('update_context_settings', { contextId: context.id, tone: 'casual', cleanupIntensity: intensity, contextualFormattingDisabled: true });
      const beforeBypass = cleanupRequests.length;
      await audio(`bypass ${intensity} at signbot`);
      assert.equal(cleanupRequests.length, beforeBypass, `${intensity} does not start an extra model call`);
    }
    assert.equal(speechCalls, 16);
    const info = await (await request('/session')).json();
    assert.equal(info.maxRuns, 30); assert.equal(info.runs, 16);
  } finally {
    try {
      for (const snippet of snippets) await invoke('remove_snippet', { id: snippet.id });
      for (const entry of vocabulary) await invoke('remove_dictionary_entry', { id: entry.id });
      if (context) await invoke('delete_context', { contextId: context.id });
      if (original) for (const key of Object.keys(settings)) await invoke('save_setting', { key, value: original[key] ?? settings[key] });
    } finally { await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); }
  }
});
