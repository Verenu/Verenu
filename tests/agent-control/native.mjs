// Run only against a disposable native app: VERENU_AGENT_TEST_SYNTHETIC=1 node ...
// The grant is inherited, never printed. No installed data or provider calls.
import assert from 'node:assert/strict';
import { invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';

assert.equal(process.env.VERENU_AGENT_TEST_SYNTHETIC, '1', 'Use a disposable synthetic native app, never installed user data');
const range = { start: '2000-01-01', end: '2100-01-01' };
const api = await invoke('describe');
assert.equal(api.version, 1);
assert.ok(api.native_commands.find((c) => c.command === 'update_context').arguments.contextId);
assert.ok(api.native_commands.find((c) => c.command === 'create_context').arguments.pasteInChunks);
assert.ok(api.native_commands.find((c) => c.command === 'get_sub_apps'));
assert.ok(Array.isArray(await invoke('get_sub_apps')));
assert.ok(Array.isArray(await invoke('get_dictionary')));
assert.ok(Array.isArray(await invoke('get_snippets')));
for (const command of ['get_dictionary_entry_contexts', 'get_snippet_entry_contexts', 'set_dictionary_context_assignment', 'set_snippet_context_assignment']) {
  assert.ok(api.native_commands.some((entry) => entry.command === command));
}
assert.ok(!api.writable_settings.includes('key_openai'));
const original = await invoke('get_setting', { key: 'mute_audio' });
await invoke('save_setting', { key: 'mute_audio', value: !original });
assert.equal(await invoke('get_setting', { key: 'mute_audio' }), !original);
await assert.rejects(invoke('save_setting', { key: 'mute_audio', value: 'invalid' }));
assert.equal(await invoke('get_setting', { key: 'mute_audio' }), !original);
await assert.rejects(invoke('save_setting', { key: 'key_openai', value: 'synthetic-not-a-key' }));
await assert.rejects(invoke('execute_sql', { sql: 'SELECT 1' }));
await assert.rejects(invoke('get_all_settings', { unexpected: true }));
const created = await invoke('create_context', { name: 'Synthetic agent group', contextualFormattingDisabled: false, pasteInChunks: true });
try {
  await invoke('update_context', { contextId: created.id, name: 'Synthetic agent edited' });
  await invoke('update_context_settings', { contextId: created.id, icon: 'AG', tone: 'professional', cleanupIntensity: 'light', customInstructions: 'Use public synthetic examples.', contextualFormattingDisabled: true, pasteInChunks: false });
  const group = (await invoke('get_contexts')).find((c) => c.id === created.id);
  assert.equal(group.name, 'Synthetic agent edited');
  assert.equal(group.contextual_formatting_disabled, true);
  assert.equal(group.paste_in_chunks, false);
  await invoke('assign_context_website', { contextId: created.id, domain: 'example.org' });
  assert.ok((await invoke('get_context_websites', { contextId: created.id })).some((w) => w.domain === 'example.org'));
  await invoke('remove_context_website', { contextId: created.id, domain: 'example.org' });
  assert.deepEqual(await invoke('get_context_websites', { contextId: created.id }), []);
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
for (const headers of [{}, { authorization: `Bearer ${'a'.repeat(64)}` }, { authorization: `Bearer ${process.env.VERENU_AGENT_TOKEN}`, origin: 'http://localhost' }]) {
  const response = await fetch(url, { method: 'POST', headers, body: JSON.stringify({ command: 'describe' }) });
  assert.equal(response.status, 401);
}
await invoke('save_setting', { key: 'mute_audio', value: original ?? false });
console.log('Native agent API: discovery, settings validation/readback, Context edits, journal opt-in/clear, history bounds and transport authorization passed.');
