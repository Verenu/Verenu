import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { detectedHarnesses, installSkill, invoke } from '../../Agent-Skills/verenu-control/scripts/verenu-agent.mjs';
import { muteForRoute } from '../../Agent-Skills/verenu-control/scripts/audio-policy.mjs';

test('skill installation requires consent and an explicit selection of detected harnesses', async () => {
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-agent-install-'));
  try {
    await fs.mkdir(path.join(home, '.codex'));
    await fs.mkdir(path.join(home, '.claude'));
    assert.deepEqual((await detectedHarnesses(home)).map((h) => h.id), ['codex', 'claude']);
    await assert.rejects(installSkill(['codex'], { home }), /consent/);
    await assert.rejects(installSkill([], { home, permanent: true }), /Choose/);
    await assert.rejects(installSkill(['grok'], { home, permanent: true }), /not detected/);
    const [destination] = await installSkill(['codex'], { home, permanent: true });
    assert.match(await fs.readFile(path.join(destination, 'SKILL.md'), 'utf8'), /name: verenu-control/);
    assert.match(await fs.readFile(path.join(destination, 'scripts/verenu-agent.mjs'), 'utf8'), /export async function invoke/);
    await assert.rejects(fs.stat(path.join(home, '.claude/skills/verenu-control')), /ENOENT/);
    await assert.rejects(installSkill(['codex'], { home, permanent: true }), /already exists/);
    await installSkill(['codex'], { home, permanent: true, replace: true });
  } finally { await fs.rm(home, { recursive: true, force: true }); }
});

test('CLI refuses malformed or absent local grants before making a request', async () => {
  await assert.rejects(invoke('describe', {}, {}), /Set VERENU_AGENT_TOKEN/);
  await assert.rejects(invoke('describe', {}, { VERENU_AGENT_TOKEN: 'a'.repeat(64), VERENU_AGENT_PORT: 'https://example.com' }), /Set VERENU_AGENT_TOKEN/);
});

test('audio policy gives confirmed Yeti headphones priority and leaves unknown routes unchanged', () => {
  assert.equal(muteForRoute({ input: 'Yeti Nano', output: 'Headphones', activePort: 'headphones' }), false);
  assert.equal(muteForRoute({ input: 'Yeti Nano', output: 'Pebble V3', activePort: 'speakers' }), true);
  assert.equal(muteForRoute({ input: 'Yeti Nano', output: 'Unknown', activePort: 'speakers' }), null);
  assert.equal(muteForRoute({ input: 'Other', output: 'Pebble V3', activePort: 'speakers' }), true);
});
