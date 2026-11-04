import { cp, lstat, mkdir, rename } from 'node:fs/promises';
import { join } from 'node:path';
export async function fix(dataDir, id) {
  if (id !== 'clear-caches') throw new Error('Unknown fix');
  const backup = join(dataDir, 'triage-backups', `${Date.now()}-${process.pid}`);
  await mkdir(backup, { recursive: true, mode: 0o700 });
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
