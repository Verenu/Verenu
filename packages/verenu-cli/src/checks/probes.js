import { lstat, readFile } from 'node:fs/promises';
import { join } from 'node:path';

/** PID files whose recorded process no longer exists. Read-only. */
export async function stalePidFiles(dataDir) {
  if (!['linux', 'darwin'].includes(process.platform)) return [];
  const stale = [];
  for (const name of ['verenu.pid', 'verenu.lock']) {
    const path = join(dataDir, name);
    try {
      const info = await lstat(path);
      if (!info.isFile() || info.isSymbolicLink() || info.size > 32) continue;
      const contents = (await readFile(path, 'utf8')).trim();
      if (!/^[1-9]\d{0,9}$/.test(contents)) continue;
      const pid = Number(contents);
      if (!Number.isSafeInteger(pid) || pid <= 1 || pid > 2147483647) continue;
      try { process.kill(pid, 0); } catch (error) { if (error.code === 'ESRCH') stale.push({ name, contents }); }
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  return stale;
}
