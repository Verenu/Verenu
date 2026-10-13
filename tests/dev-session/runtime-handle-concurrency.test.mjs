import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const access = JSON.parse(await fs.readFile(process.env.VERENU_SESSION_ACCESS_FILE, 'utf8'));
const origin = new URL(access.localAccessUrl).origin;
async function invoke(command, args = {}) {
  const response = await fetch(`${origin}/__verenu_dev/invoke`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, args }),
    signal: AbortSignal.timeout(30_000),
  });
  assert.equal(response.status, 200, command);
  return response.json();
}

test('native IPC remains usable after sustained concurrent handle lifetimes', { timeout: 120_000 }, async () => {
  const before = await invoke('get_contexts');
  const beforeSettings = await invoke('get_all_settings');
  await Promise.all(Array.from({ length: 8 }, async () => {
    for (let batch = 0; batch < 64; batch++) {
      const [contexts, settings, memory] = await Promise.all([
        invoke('get_contexts'),
        invoke('get_all_settings'),
        invoke('get_memory_mb'),
      ]);
      assert.deepEqual(contexts, before);
      assert.deepEqual(settings, beforeSettings);
      assert.equal(typeof memory, 'number');
      assert.ok(Number.isFinite(memory) && memory >= 0);
    }
  }));
  assert.deepEqual(await invoke('get_contexts'), before);
});
