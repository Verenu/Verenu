import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export function git(args, cwd = root) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', timeout: 10_000 });
}
function readSourceIdentity(cwd, includeFiles) {
  const files = [...new Set(git(['ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd).split('\0').filter(Boolean))].sort();
  const hash = createHash('sha256');
  const entries = includeFiles ? [] : null;
  for (const name of files) {
    const file = path.join(cwd, name);
    hash.update(name).update('\0');
    try {
      const stat = fs.lstatSync(file);
      const mode = String(stat.mode);
      const type = stat.isSymbolicLink() ? 'symlink' : stat.isFile() ? 'file' : 'other';
      const contents = stat.isSymbolicLink()
        ? Buffer.from(fs.readlinkSync(file))
        : stat.isFile() ? fs.readFileSync(file) : null;
      hash.update(mode).update('\0');
      if (contents !== null) hash.update(contents);
      if (entries) entries.push({
        path: name,
        state: 'present',
        mode,
        type,
        sha256: contents === null ? null : createHash('sha256').update(contents).digest('hex'),
      });
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      hash.update('[deleted]');
      if (entries) entries.push({ path: name, state: 'deleted', mode: null, type: null, sha256: null });
    }
    hash.update('\0');
  }
  return {
    identity: { worktree: fs.realpathSync(cwd), commit: git(['rev-parse', 'HEAD'], cwd).trim(), branch: git(['branch', '--show-current'], cwd).trim(), fingerprint: hash.digest('hex') },
    ...(entries ? { files: entries } : {}),
  };
}
export function sourceSnapshot(cwd = root) {
  return readSourceIdentity(cwd, true);
}
export function sourceIdentity(cwd = root) {
  return readSourceIdentity(cwd, false).identity;
}
export function sourceFileChanges(before, after, limit = 200) {
  const earlier = new Map(before.map(file => [file.path, file]));
  const later = new Map(after.map(file => [file.path, file]));
  const changed = [...new Set([...earlier.keys(), ...later.keys()])].sort().flatMap(name => {
    const previous = earlier.get(name) ?? null;
    const current = later.get(name) ?? null;
    if (JSON.stringify(previous) === JSON.stringify(current)) return [];
    const describe = file => file && ({ state: file.state, mode: file.mode, type: file.type, sha256: file.sha256 });
    return [{ path: name, before: describe(previous), after: describe(current) }];
  });
  return { count: changed.length, files: changed.slice(0, limit), truncated: changed.length > limit };
}
export function changedFiles(base = 'master', cwd = root) {
  const ancestor = git(['merge-base', 'HEAD', base], cwd).trim();
  return [...new Set([...git(['diff', '--name-only', '-z', ancestor], cwd).split('\0'), ...git(['ls-files', '-z', '--others', '--exclude-standard'], cwd).split('\0')].filter(Boolean))].sort();
}
export function artifact(file) {
  return { path: path.resolve(file), sha256: createHash('sha256').update(fs.readFileSync(file)).digest('hex') };
}
