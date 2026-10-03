import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export function git(args, cwd = root) {
  return execFileSync('git', args, { cwd, encoding: 'utf8', timeout: 10_000 });
}
export function sourceIdentity(cwd = root) {
  const files = [...new Set(git(['ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd).split('\0').filter(Boolean))].sort();
  const hash = createHash('sha256');
  for (const name of files) {
    const file = path.join(cwd, name);
    hash.update(name).update('\0');
    try {
      const stat = fs.lstatSync(file);
      hash.update(String(stat.mode)).update('\0');
      if (stat.isSymbolicLink()) hash.update(fs.readlinkSync(file));
      else if (stat.isFile()) hash.update(fs.readFileSync(file));
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      hash.update('[deleted]');
    }
    hash.update('\0');
  }
  return { worktree: fs.realpathSync(cwd), commit: git(['rev-parse', 'HEAD'], cwd).trim(), branch: git(['branch', '--show-current'], cwd).trim(), fingerprint: hash.digest('hex') };
}
export function changedFiles(base = 'master', cwd = root) {
  const ancestor = git(['merge-base', 'HEAD', base], cwd).trim();
  return [...new Set([...git(['diff', '--name-only', '-z', ancestor], cwd).split('\0'), ...git(['ls-files', '-z', '--others', '--exclude-standard'], cwd).split('\0')].filter(Boolean))].sort();
}
export function artifact(file) {
  return { path: path.resolve(file), sha256: createHash('sha256').update(fs.readFileSync(file)).digest('hex') };
}
