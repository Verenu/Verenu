import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

async function runBeforeLaunch(overrides) {
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-context-cli-'));
  try {
    await fs.mkdir(path.join(temporary, 'scripts', 'verification'), { recursive: true });
    const source = await fs.readFile(new URL('../../scripts/test-linux-contexts.mjs', import.meta.url), 'utf8');
    await fs.copyFile(new URL('../../scripts/verification/identity.mjs', import.meta.url), path.join(temporary, 'scripts', 'verification', 'identity.mjs'));
    const executable = path.join(temporary, 'scripts', 'fixture.mjs');
    await fs.writeFile(executable, source);
    const hooks = path.join(temporary, 'hooks');
    await fs.mkdir(hooks);
    for (const args of [
      ['init', '--quiet', `--template=${hooks}`],
      ['-c', 'user.name=Verification Fixture', '-c', 'user.email=verification@example.invalid', '-c', 'commit.gpgsign=false', '-c', `core.hooksPath=${hooks}`, 'commit', '--quiet', '--allow-empty', '-m', 'Initialize fixture'],
    ]) {
      const git = spawnSync('git', args, { cwd: temporary, encoding: 'utf8', timeout: 10_000 });
      assert.equal(git.status, 0, git.stderr);
    }
    const env = { ...process.env, ...overrides };
    delete env.VERENU_CONTEXT_TEST_ISOLATED;
    const result = spawnSync(process.execPath, [executable], { env, encoding: 'utf8', timeout: 10_000 });
    assert.equal(result.status, 1, result.stderr);
    const directories = await fs.readdir(path.join(temporary, 'test-results'));
    assert.equal(directories.length, 1);
    const report = JSON.parse(await fs.readFile(path.join(temporary, 'test-results', directories[0], 'verification.json'), 'utf8'));
    assert.equal(report.status, 'failed');
    assert.equal(report.checks.length, 1);
    assert.equal(report.checks[0].status, 'failed');
    assert.ok(!result.stderr.includes('ERR_SERVER_NOT_RUNNING'));
    return report;
  } finally {
    await fs.rm(temporary, { recursive: true, force: true });
  }
}

test('a fresh results directory still records prerequisite failure before the server starts', async () => {
  await runBeforeLaunch({ HYPRLAND_INSTANCE_SIGNATURE: '' });
});

test('the browser matrix refuses to focus windows without the compositor isolation opt-in', async () => {
  const report = await runBeforeLaunch({ HYPRLAND_INSTANCE_SIGNATURE: 'fixture-not-a-real-compositor' });
  if (process.platform === 'linux') assert.match(report.checks[0].reason, /disposable Hyprland/);
});
