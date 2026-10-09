import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { readDemoVersion } from '../../demo-video/release-version.mjs';

const exec = promisify(execFile);
const helper = fileURLToPath(new URL('../../demo-video/work-dir.sh', import.meta.url));
const prepare = (cwd, requested, cache) => exec('bash', ['-euc',
  'source "$1"; prepare_demo_work_dir "$2" "$3"', 'demo-test', helper, requested, cache,
], { cwd, timeout: 10_000 }).then(({ stdout }) => stdout.trim());

test('demo explicit relative output stays under an outside caller directory', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-demo-path-'));
  try {
    const work = await prepare(root, 'nested output/job', path.join(root, 'cache'));
    assert.equal(work, path.join(await fs.realpath(root), 'nested output/job'));
    assert.equal((await fs.stat(work)).isDirectory(), true);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('concurrent default demo invocations own different directories', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-demo-jobs-'));
  try {
    const jobs = await Promise.all(Array.from({ length: 8 }, () => prepare(root, '', path.join(root, 'cache'))));
    assert.equal(new Set(jobs).size, jobs.length);
    for (const job of jobs) assert.equal((await fs.stat(job)).isDirectory(), true);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('demo version follows canonical metadata changes and rejects missing metadata', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-demo-version-'));
  try {
    for (const version of ['1.2.3', '2.0.0-beta.1']) {
      await fs.writeFile(path.join(root, 'package.json'), JSON.stringify({ version }));
      assert.equal(readDemoVersion(root), version);
    }
    await fs.writeFile(path.join(root, 'package.json'), '{}');
    assert.throws(() => readDemoVersion(root), /valid package.json version/);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
