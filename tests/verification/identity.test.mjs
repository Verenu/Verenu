import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { sourceIdentity } from '../../scripts/verification/identity.mjs';
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
