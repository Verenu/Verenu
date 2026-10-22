import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { auditBuild, executions } from './audit.mjs';

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-compiler-audit-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return { directory, artifact: path.join(directory, 'program'), env: process.env };
}

function nodeBuild(options, source) {
  return auditBuild({ ...options, command: process.execPath, args: ['--input-type=module', '-e', source], timeout: 15_000 });
}

test('the audit requires a successful build and a newly created executable', t => {
  const options = fixture(t);
  const result = nodeBuild(options, `import fs from 'node:fs'; fs.copyFileSync('/bin/true', ${JSON.stringify(options.artifact)});`);
  assert.equal(result.passed, true, JSON.stringify(result.failures));
  assert.ok(result.executions.length);
});

test('a successful build that invokes a nested rustc process fails the audit', t => {
  const options = fixture(t);
  const compiler = path.join(options.directory, 'rustc');
  fs.copyFileSync('/bin/true', compiler);
  const result = nodeBuild(options, `import fs from 'node:fs'; import { spawnSync } from 'node:child_process';
    spawnSync(${JSON.stringify(compiler)}, ['-vV']); fs.copyFileSync('/bin/true', ${JSON.stringify(options.artifact)});`);
  assert.equal(result.status, 0);
  assert.equal(result.passed, false);
  assert.ok(result.failures.includes('rustc command requested'));
});

test('a check-only success cannot count as a successful build', t => {
  const result = nodeBuild(fixture(t), '');
  assert.equal(result.status, 0);
  assert.equal(result.passed, false);
  assert.match(result.failures.join('\n'), /did not produce/);
});

test('an artifact from a failed compiler cannot count as success', t => {
  const options = fixture(t);
  const result = nodeBuild(options, `import fs from 'node:fs'; fs.copyFileSync('/bin/true', ${JSON.stringify(options.artifact)}); process.exit(1);`);
  assert.equal(result.passed, false);
  assert.ok(result.failures.includes('Build command did not finish successfully'));
});

test('an old executable cannot mask a build that produced nothing', t => {
  const options = fixture(t);
  fs.copyFileSync('/bin/true', options.artifact);
  assert.throws(() => nodeBuild(options, ''), /pre-existing/);
});

test('a hung build is killed and cannot count as success', t => {
  const options = fixture(t);
  const result = auditBuild({ ...options, command: process.execPath,
    args: ['-e', 'setInterval(() => {}, 1000)'], timeout: 200 });
  assert.equal(result.passed, false);
  assert.match(result.error, /ETIMEDOUT/);
});

test('trace decoding catches execveat and compiler argv0, and fails closed', () => {
  assert.equal(executions('12 execveat(3, "", ["rustc", "-vV"], 0x0, AT_EMPTY_PATH) = 0')[0].rustc, true);
  assert.equal(executions('12 execve("/opt/toolchain/rustc", ["compiler"], 0x0) = -1 ENOENT')[0].rustc, true);
  assert.equal(executions('12 execve("/usr/bin/cargo", ["cargo"], 0x0) = 0')[0].rustc, false);
  assert.throws(() => executions(''), /no executions/);
  assert.throws(() => executions('12 execve(unreadable)'), /Cannot decode/);
});
