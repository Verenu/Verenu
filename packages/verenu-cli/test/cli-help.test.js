import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const cli = fileURLToPath(new URL('../bin/verenu.js', import.meta.url));
const pkg = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));

function run(args, options = {}) {
  return spawnSync(process.execPath, [cli, ...args], { encoding: 'utf8', timeout: 15000, ...options });
}

test('--help prints usage and exits 0 without running checks', async t => {
  const dataDir = await mkdtemp(join(os.tmpdir(), 'verenu-triage-'));
  t.after(() => rm(dataDir, { recursive: true, force: true }));
  for (const args of [['--help'], ['-h'], ['triage', '--help'], ['triage', '-h'], ['triage', '--agent', 'claude', '--help']]) {
    const invoked = run([...args, '--data-dir', dataDir]);
    assert.equal(invoked.status, 0, args.join(' '));
    assert.match(invoked.stdout, /^Usage: verenu triage \[options\]/, args.join(' '));
    assert.match(invoked.stdout, /--agent claude\|codex/);
    assert.doesNotMatch(invoked.stdout, /"results"|PASS|FAIL|WARN/, args.join(' '));
    assert.equal(invoked.stderr, '', args.join(' '));
  }
});

test('--version prints the package version', () => {
  for (const args of [['--version'], ['-v']]) {
    const invoked = run(args);
    assert.equal(invoked.status, 0, args.join(' '));
    assert.equal(invoked.stdout.trim(), pkg.version);
  }
});

test('no command prints usage to stderr and exits 2', () => {
  const invoked = run([]);
  assert.equal(invoked.status, 2);
  assert.match(invoked.stderr, /^Usage: verenu triage/);
  assert.equal(invoked.stdout, '');
});

test('an unknown option exits 2 and points to --help', () => {
  const invoked = run(['triage', '--bogus']);
  assert.equal(invoked.status, 2);
  assert.match(invoked.stderr, /Unknown or incomplete option: --bogus/);
  assert.match(invoked.stderr, /verenu triage --help/);
});
