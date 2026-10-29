// Run only against a disposable native app: VERENU_AGENT_TEST_SYNTHETIC=1 node ...
// The grant is inherited, never printed. No installed data or provider calls.
import assert from 'node:assert/strict';
import { invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';

assert.equal(process.env.VERENU_AGENT_TEST_SYNTHETIC, '1', 'Use a disposable synthetic native app, never installed user data');
const range = { start: '2000-01-01', end: '2100-01-01' };
const api = await invoke('describe');
async function save(key, value) {
  const expected = { [key]: await invoke('get_setting', { key }) };
  return invoke('save_setting', { key, value, expected });
}
assert.equal(api.version, 1);
assert.ok(!api.native_commands.some((c) => c.command === 'update_context'));
assert.ok(api.native_commands.find((c) => c.command === 'create_context').arguments.pasteInChunks);
assert.ok(api.native_commands.find((c) => c.command === 'get_sub_apps'));
assert.ok(Array.isArray(await invoke('get_sub_apps')));
assert.ok(Array.isArray(await invoke('get_dictionary')));
assert.ok(Array.isArray(await invoke('get_snippets')));
for (const command of ['get_dictionary_entry_contexts', 'get_snippet_entry_contexts', 'set_dictionary_context_assignment', 'set_snippet_context_assignment']) {
  assert.ok(api.native_commands.some((entry) => entry.command === command));
}
assert.ok(!api.writable_settings.includes('key_openai'));
for (const key of ['default_tone', 'cleanup_intensity', 'voice_commands_enabled', 'appearance_mode']) assert.ok(api.writable_settings.includes(key));
for (const command of ['create_dictionary_entry', 'edit_dictionary_entry', 'create_snippet', 'edit_snippet', 'create_sub_app', 'save_hotkey']) assert.ok(api.native_commands.some((item) => item.command === command));
for (const [command, key, args, stale] of [
  ['save_hotkey', 'hotkey', { keys: ['ControlLeft', 'MetaLeft'] }, ['synthetic-stale-key']],
  ['set_autostart', 'autostart_enabled', { enabled: false }, 'synthetic-stale-value'],
]) {
  const current = await invoke('get_setting', { key });
  assert.ok(api.native_commands.find((item) => item.command === command).arguments.expected);
  await assert.rejects(invoke(command, args), /Expected current setting/);
  await assert.rejects(invoke(command, { ...args, expected: stale }), /changed while you were editing/);
  assert.deepEqual(await invoke('get_setting', { key }), current);
}
for (const [key, value] of [['default_tone', 'formal'], ['cleanup_intensity', 'high'], ['voice_commands_enabled', false], ['appearance_mode', 'dark']]) {
  const before = await invoke('get_setting', { key });
  await save(key, value);
  assert.equal(await invoke('get_setting', { key }), value);
  await assert.rejects(invoke('save_setting', { key, value: { bad: true }, expected: { [key]: value } }));
  assert.equal(await invoke('get_setting', { key }), value);
  if (before !== null) await save(key, before);
}
await assert.rejects(invoke('credentials.save', { provider: 'openai', key: 123, unexpected: true }), /Expected provider and key strings/);
await assert.rejects(invoke('credentials.save', { provider: 'not-a-provider', key: 'synthetic-secret-marker' }), (error) => !error.message.includes('synthetic-secret-marker'));
await assert.rejects(invoke('get_api_key', { provider: 'openai' }));
await assert.rejects(invoke('t3.connect', { pairingLink: 'synthetic-private-pairing' }), (error) => !error.message.includes('synthetic-private-pairing'));
const provider = { id: 'custom:87654321-1234-4234-8234-123456789021', name: 'Synthetic provider', protocol: 'openai', base_url: 'http://127.0.0.1:18181/v1', requires_key: false, supports_transcription: true, supports_cleanup: false, transcription_models: ['synthetic-model'], cleanup_models: [] };
await save('custom_providers', [provider]);
await assert.rejects(save('custom_providers', []), /delete_custom_provider/);
assert.equal((await invoke('get_setting', { key: 'custom_providers' })).length, 1);
await invoke('delete_custom_provider', { provider: provider.id });
assert.deepEqual(await invoke('get_setting', { key: 'custom_providers' }), []);
const original = await invoke('get_setting', { key: 'mute_audio' });
await save('mute_audio', !original);
await assert.rejects(invoke('save_setting', { key: 'mute_audio', value: Boolean(original), expected: { mute_audio: original } }), /changed while you were editing/);
assert.equal(await invoke('get_setting', { key: 'mute_audio' }), !original);
await assert.rejects(invoke('save_setting', { key: 'mute_audio', value: 'invalid', expected: { mute_audio: !original } }));
assert.equal(await invoke('get_setting', { key: 'mute_audio' }), !original);
await assert.rejects(invoke('save_setting', { key: 'key_openai', value: 'synthetic-not-a-key', expected: { key_openai: null } }));
await assert.rejects(invoke('execute_sql', { sql: 'SELECT 1' }));
await assert.rejects(invoke('get_all_settings', { unexpected: true }));
for (const command of ['describe', 'get_microphones', 'diagnostics.snapshot']) {
  await assert.rejects(invoke(command, { unexpected: true }));
}
const created = await invoke('create_context', { name: 'Synthetic agent group', contextualFormattingDisabled: false, pasteInChunks: true });
try {
  const originalContext = (await invoke('get_contexts')).find((c) => c.id === created.id);
  await invoke('save_context_if_unchanged', { original: originalContext, updated: { ...originalContext, name: 'Synthetic agent edited' } });
  const patch = async (settings) => invoke('context.patch_settings', { contextId: created.id, settings, expected: (await invoke('get_contexts')).find((c) => c.id === created.id) });
  await patch({ icon: 'AG', tone: 'professional', cleanupIntensity: 'light', customInstructions: 'Use public synthetic examples.', contextualFormattingOverride: true, pasteInChunks: false });
  const group = (await invoke('get_contexts')).find((c) => c.id === created.id);
  assert.equal(group.name, 'Synthetic agent edited');
  assert.equal(group.contextual_formatting_disabled, true);
  assert.equal(group.paste_in_chunks, false);
  const patched = await patch({ tone: 'casual' });
  await assert.rejects(invoke('context.patch_settings', { contextId: created.id, settings: { tone: 'stale' }, expected: group }), /changed while you were editing/);
  assert.equal(patched.tone, 'casual');
  assert.equal(patched.icon, 'AG');
  assert.equal(patched.custom_instructions, 'Use public synthetic examples.');
  assert.equal(patched.contextual_formatting_override, true);
  assert.equal(patched.paste_in_chunks, false);
  await assert.rejects(invoke('context.patch_settings', { contextId: created.id, settings: { tone: 'formal', pasteInChunks: null } }));
  await assert.rejects(invoke('context.patch_settings', { contextId: created.id, settings: { typo: true } }));
  await assert.rejects(invoke('context.patch_settings', { contextId: created.id, settings: {} }));
  assert.equal((await invoke('get_contexts')).find((c) => c.id === created.id).tone, 'casual');
  const cleared = await patch({ icon: null, contextualFormattingOverride: null });
  assert.equal(cleared.icon, null);
  assert.equal(cleared.contextual_formatting_override, null);
  assert.equal(cleared.custom_instructions, 'Use public synthetic examples.');
  let currentContext = (await invoke('get_contexts')).find((c) => c.id === created.id);
  await invoke('save_context_if_unchanged', { original: currentContext, updated: { ...currentContext, t3_skill_mentions_disabled: true } });
  assert.equal((await invoke('get_contexts')).find((c) => c.id === created.id).t3_skill_mentions_disabled, true);
  currentContext = (await invoke('get_contexts')).find((c) => c.id === created.id);
  await invoke('save_context_if_unchanged', { original: currentContext, updated: { ...currentContext, t3_skill_mentions_disabled: false } });
  assert.equal((await invoke('get_contexts')).find((c) => c.id === created.id).t3_skill_mentions_disabled, false);
  currentContext = (await invoke('get_contexts')).find((c) => c.id === created.id);
  await invoke('context.set_pinned', { contextId: created.id, pinned: true, expected: currentContext });
  await assert.rejects(invoke('context.set_pinned', { contextId: created.id, pinned: false, expected: currentContext }), /changed while you were editing/);
  await invoke('assign_context_website', { contextId: created.id, domain: 'example.org' });
  assert.ok((await invoke('get_context_websites', { contextId: created.id })).some((w) => w.domain === 'example.org'));
  await invoke('remove_context_website', { contextId: created.id, domain: 'example.org' });
  assert.deepEqual(await invoke('get_context_websites', { contextId: created.id }), []);
  const word = await invoke('create_dictionary_entry', { term: 'SyntheticTerm', mistake: 'synthetic term', contextId: created.id });
  assert.ok((await invoke('get_context_dictionary', { contextId: created.id })).some((entry) => entry.id === word.id));
  await assert.rejects(invoke('edit_dictionary_entry', { id: word.id, term: 'Stale', contextId: created.id }), /Expected original/);
  await invoke('edit_dictionary_entry', { id: word.id, term: 'SyntheticUpdated', mistake: 'synthetic term', contextId: created.id, expected: { term: 'SyntheticTerm', mistake: 'synthetic term' } });
  await assert.rejects(invoke('edit_dictionary_entry', { id: word.id, term: 'Stale', mistake: 'synthetic term', contextId: created.id, expected: { term: 'SyntheticTerm', mistake: 'synthetic term' } }));
  assert.ok((await invoke('get_context_dictionary', { contextId: created.id })).some((entry) => entry.term === 'SyntheticUpdated'));
  const snippet = await invoke('create_snippet', { trigger: 'synthetic trigger', expansion: 'Synthetic expansion', instructions: '', contextId: created.id });
  await assert.rejects(invoke('edit_snippet', { id: snippet.id, trigger: 'synthetic trigger', expansion: 'Stale', instructions: '' }), /Expected original/);
  await invoke('edit_snippet', { id: snippet.id, trigger: 'synthetic trigger', expansion: 'Updated expansion', instructions: '', expected: { trigger: 'synthetic trigger', expansion: 'Synthetic expansion', instructions: '' } });
  await assert.rejects(invoke('edit_snippet', { id: snippet.id, trigger: 'synthetic trigger', expansion: 'Stale', instructions: '', expected: { trigger: 'synthetic trigger', expansion: 'Synthetic expansion', instructions: '' } }));
  assert.ok((await invoke('get_context_snippets', { contextId: created.id })).some((entry) => entry.expansion === 'Updated expansion'));
  const second = await invoke('create_context', { name: 'Synthetic assignment group' });
  try {
    await invoke('set_dictionary_context_assignment', { contextId: second.id, dictionaryId: word.id, assigned: true });
    await invoke('set_snippet_context_assignment', { contextId: second.id, snippetId: snippet.id, assigned: true });
    assert.ok((await invoke('get_context_dictionary', { contextId: second.id })).some((entry) => entry.term === 'SyntheticUpdated'));
    assert.ok((await invoke('get_context_snippets', { contextId: second.id })).some((entry) => entry.expansion === 'Updated expansion'));
    await invoke('set_dictionary_context_assignment', { contextId: second.id, dictionaryId: word.id, assigned: false });
    await invoke('set_snippet_context_assignment', { contextId: second.id, snippetId: snippet.id, assigned: false });
    assert.ok(!(await invoke('get_context_snippets', { contextId: second.id })).some((entry) => entry.expansion === 'Updated expansion'));
  } finally {
    await invoke('delete_context', { contextId: second.id });
  }
  const subApp = await invoke('create_sub_app', { executable: 'synthetic-agent-app', label: 'Synthetic tab', titlePattern: 'Synthetic title', matchMode: 'contains' });
  await invoke('assign_sub_app', { id: subApp.id, contextId: created.id });
  assert.equal((await invoke('get_sub_apps')).find((item) => item.id === subApp.id).context_id, created.id);
  await assert.rejects(invoke('create_sub_app', { executable: 'synthetic-agent-app', label: 'Invalid', titlePattern: 'test', matchMode: 'invalid' }));
  await invoke('delete_sub_app', { id: subApp.id });
  await invoke('remove_snippet', { id: snippet.id });
  await invoke('remove_dictionary_entry', { id: word.id });
} finally {
  await invoke('delete_context', { contextId: created.id });
}
assert.ok(!(await invoke('get_contexts')).some((c) => c.id === created.id));
assert.equal((await invoke('diagnostics.retention')).retention_days, 0);
await invoke('diagnostics.retention', { days: 90 });
// Existing native command validation emits a failure, never dictated text.
await assert.rejects(invoke('delete_context', { contextId: -9999 }));
const deadline = Date.now() + 5000;
let top;
do {
  top = await invoke('diagnostics.top', range);
  if (top.top_ten.length) break;
  await new Promise((resolve) => setTimeout(resolve, 50));
} while (Date.now() < deadline);
assert.equal(top.retention_days, 90);
assert.ok(top.top_ten.length, 'Native failures must reach the opted-in persistent journal');
assert.ok(top.top_ten.every((g) => !Object.hasOwn(g, 'message')));
await invoke('diagnostics.retention', { days: 0 });
assert.deepEqual((await invoke('diagnostics.top', range)).top_ten, []);
await assert.rejects(invoke('diagnostics.retention', { days: 366 }));
const report = await invoke('history.report', range);
assert.ok(report.retained_count >= 0);
const usage = await invoke('history.usage', range);
assert.equal(usage.retained_count, report.retained_count);
assert.ok(usage.app_contexts.every((row) => !Object.hasOwn(row, 'raw_text') && !Object.hasOwn(row, 'clean_text')));
const page = await invoke('history.page', { ...range, limit: 1 });
assert.ok(page.rows.every((r) => !Object.hasOwn(r, 'raw_text')));
await assert.rejects(invoke('history.page', { ...range, limit: 501 }));
await assert.rejects(invoke('history.report', { start: 'yesterday', end: 'tomorrow' }));
await invoke('prompts.lint', { template: '{text}' });
await invoke('diagnostics.snapshot');
assert.ok(api.additional_commands['diagnostics.logs']);
const logs = await invoke('diagnostics.logs', { level: 'warn', limit: 1 });
assert.ok(logs.logs.length <= 1);
assert.ok(logs.matching_retained >= logs.logs.length);
assert.equal(logs.omitted_by_limit, logs.matching_retained - logs.logs.length);
assert.ok(Number.isInteger(logs.evicted_since_reset));
assert.ok(Number.isInteger(logs.file_records_dropped_this_process));
await assert.rejects(invoke('diagnostics.logs', { limit: 0 }));
const url = `http://127.0.0.1:${process.env.VERENU_AGENT_PORT}/v1/invoke`;
for (const headers of [{}, { authorization: `Bearer ${'a'.repeat(64)}` }, { authorization: `Bearer ${process.env.VERENU_AGENT_TOKEN}`, origin: 'http://localhost' }, { authorization: `Bearer ${process.env.VERENU_AGENT_TOKEN}`, 'sec-fetch-site': 'same-origin' }]) {
  const response = await fetch(url, { method: 'POST', headers, body: JSON.stringify({ command: 'save_setting', args: { key: 'mute_audio', value: original } }) });
  assert.equal(response.status, 401);
  assert.equal(await invoke('get_setting', { key: 'mute_audio' }), !original);
}
await save('mute_audio', original ?? false);
console.log('Native agent API: discovery, settings validation/readback, Context edits, journal opt-in/clear, history bounds and transport authorization passed.');
