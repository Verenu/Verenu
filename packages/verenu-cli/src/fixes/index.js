import { cp, lstat, mkdir, rename, readFile, writeFile, mkdtemp } from 'node:fs/promises';
import { join } from 'node:path';
export async function fix(dataDir, id) {
  if (!['clear-caches', 'reset-settings'].includes(id)) throw new Error('Unknown fix');
  const backupRoot = join(dataDir, 'triage-backups');
  await mkdir(backupRoot, { recursive: true, mode: 0o700 });
  const backup = await mkdtemp(join(backupRoot, `${Date.now()}-`));
  if (id === 'reset-settings') {
    const source = join(dataDir, 'settings.json');
    const info = await lstat(source);
    if (!info.isFile() || info.isSymbolicLink()) throw new Error('Settings must be a regular file');
    const original = await readFile(source);
    try { const parsed = JSON.parse(original); if (parsed && !Array.isArray(parsed) && typeof parsed === 'object') throw new Error('Settings are valid'); } catch (error) { if (error.message === 'Settings are valid') throw error; }
    await writeFile(join(backup, 'settings.json'), original, { flag: 'wx', mode: 0o600 });
    if (!(await readFile(source)).equals(original)) throw new Error('Settings changed during backup');
    const replacement = join(dataDir, `.triage-settings-${process.pid}-${Date.now()}.tmp`);
    await writeFile(replacement, '{}\n', { flag: 'wx', mode: 0o600 });
    await rename(replacement, source);
    return backup;
  }
  for (const name of ['icon-cache', 'favicon-cache']) {
    const source = join(dataDir, name);
    let info;
    try { info = await lstat(source); } catch (error) { if (error.code === 'ENOENT') continue; throw error; }
    if (!info.isDirectory() || info.isSymbolicLink()) throw new Error('Cache is not a directory');
    await cp(source, join(backup, name), { recursive: true, errorOnExist: true, force: false, dereference: false });
    await rename(source, join(backup, `${name}.original`));
  }
  return backup;
}
