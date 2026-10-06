import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { sourceFileChanges, sourceIdentity, sourceSnapshot } from '../../scripts/verification/identity.mjs';
test('uncommitted edits, additions and deletions invalidate source identity', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-identity-'));
  const git = (...args) => execFileSync('git', args, { cwd: directory, stdio: 'ignore' });
  try {
    git('init'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@invalid');
    fs.writeFileSync(path.join(directory, 'source.txt'), 'before'); git('add', 'source.txt'); git('commit', '-m', 'fixture');
    const initial = sourceIdentity(directory);
    fs.writeFileSync(path.join(directory, 'source.txt'), 'after');
    assert.notEqual(sourceIdentity(directory).fingerprint, initial.fingerprint);
    fs.writeFileSync(path.join(directory, 'source.txt'), 'before');
    assert.equal(sourceIdentity(directory).fingerprint, initial.fingerprint);
    fs.writeFileSync(path.join(directory, 'new.txt'), 'new');
    assert.notEqual(sourceIdentity(directory).fingerprint, initial.fingerprint);
    fs.unlinkSync(path.join(directory, 'new.txt')); fs.unlinkSync(path.join(directory, 'source.txt'));
    assert.notEqual(sourceIdentity(directory).fingerprint, initial.fingerprint);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test('source diagnostics show changed repository paths and digests without source contents', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-identity-diagnostic-'));
  const git = (...args) => execFileSync('git', args, { cwd: directory, stdio: 'ignore' });
  try {
    git('init'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@invalid');
    const secretSource = 'PRIVATE_SOURCE_TEXT_FOR_DIAGNOSTIC_TEST';
    fs.writeFileSync(path.join(directory, 'source.txt'), secretSource); git('add', 'source.txt'); git('commit', '-m', 'fixture');
    const before = sourceSnapshot(directory);
    assert.equal(before.identity.fingerprint, sourceIdentity(directory).fingerprint);
    fs.writeFileSync(path.join(directory, 'source.txt'), 'changed source');
    fs.writeFileSync(path.join(directory, 'generated.txt'), 'generated source');
    const after = sourceSnapshot(directory);
    assert.equal(after.identity.fingerprint, sourceIdentity(directory).fingerprint);
    const changes = sourceFileChanges(before.files, after.files);
    const summarize = file => ({ state: file.state, mode: file.mode, type: file.type, sha256: file.sha256 });
    const previousSource = before.files.find(file => file.path === 'source.txt');
    const currentSource = after.files.find(file => file.path === 'source.txt');
    const generatedSource = after.files.find(file => file.path === 'generated.txt');

    assert.deepEqual(changes, {
      count: 2,
      files: [
        { path: 'generated.txt', before: null, after: summarize(generatedSource) },
        { path: 'source.txt', before: summarize(previousSource), after: summarize(currentSource) },
      ],
      truncated: false,
    });
    assert.equal(JSON.stringify(changes).includes(secretSource), false);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
