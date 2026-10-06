import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

test('Windows checkout preserves Cargo.toml bytes used by strict source identity', () => {
  const attributes = execFileSync('git', ['check-attr', 'eol', '--', 'src-tauri/Cargo.toml'], {
    cwd: root,
    encoding: 'utf8',
  });
  assert.match(attributes, /src-tauri\/Cargo\.toml: eol: lf\s*$/);

  const committed = execFileSync('git', ['show', 'HEAD:src-tauri/Cargo.toml'], { cwd: root });
  const windowsCheckout = execFileSync('git', [
    '-c', 'core.autocrlf=true',
    'cat-file', '--filters', '--path=src-tauri/Cargo.toml', 'HEAD:src-tauri/Cargo.toml',
  ], { cwd: root });
  assert.deepEqual(windowsCheckout, committed);
});
