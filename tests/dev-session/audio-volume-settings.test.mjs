import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const accessFile = process.env.VERENU_SESSION_ACCESS_FILE;
if (!accessFile) throw new Error('Set VERENU_SESSION_ACCESS_FILE to a running dev session access.json.');
const access = JSON.parse(await fs.readFile(accessFile, 'utf8'));
const base = new URL(access.localAccessUrl).origin;

async function invoke(command, args = {}) {
  const response = await fetch(`${base}/__verenu_dev/invoke`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, args }),
    signal: AbortSignal.timeout(10_000),
  });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('changing dictation volume replaces legacy mute and persists the new choice', { skip: process.platform !== 'linux' }, async () => {
  const original = await invoke('get_all_settings');
  try {
    await invoke('save_setting', { key: 'mute_audio', value: true });
    await invoke('save_setting', { key: 'duck_target', value: 30 });
    let saved = await invoke('get_all_settings');
    assert.equal(saved.mute_audio, false);
    assert.equal(saved.duck_audio, true);
    assert.equal(saved.duck_target, 30);

    await invoke('save_setting', { key: 'duck_target', value: 0 });
    saved = await invoke('get_all_settings');
    assert.equal(saved.duck_target, 0);
    assert.equal(saved.duck_audio, true);
    assert.equal(saved.mute_audio, false);

    await invoke('save_setting', { key: 'duck_target', value: 100 });
    assert.equal((await invoke('get_all_settings')).duck_target, 100);

    const expected = { duck_target: 100, duck_audio: true, mute_audio: false };
    await invoke('save_setting_if_unchanged', { key: 'duck_target', value: 45, expected });
    saved = await invoke('get_all_settings');
    assert.equal(saved.duck_target, 45);
    assert.equal(saved.duck_audio, true);
    assert.equal(saved.mute_audio, false);
    const stale = await fetch(`${base}/__verenu_dev/invoke`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ command: 'save_setting_if_unchanged', args: { key: 'duck_target', value: 20, expected } }),
      signal: AbortSignal.timeout(10_000),
    });
    assert.notEqual(stale.status, 200);
    assert.equal((await invoke('get_all_settings')).duck_target, 45);
  } finally {
    await invoke('save_setting', { key: 'duck_target', value: original.duck_target ?? 25 });
    await invoke('save_setting', { key: 'duck_audio', value: original.duck_audio ?? false });
    await invoke('save_setting', { key: 'mute_audio', value: original.mute_audio ?? false });
  }
});
