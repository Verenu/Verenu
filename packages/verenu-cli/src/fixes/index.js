import { cp, lstat, mkdir, rename, readFile, writeFile, mkdtemp, readdir } from 'node:fs/promises';
import { delimiter, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';

async function hashFile(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

export function holders(paths, platform = process.platform, run = spawnSync) {
  if (!['linux', 'darwin'].includes(platform)) throw new Error('Holder detection is unavailable on this platform');
  const probe = run('lsof', ['-t', '--', ...paths], { encoding: 'utf8', timeout: 5000 });
  if (probe.error || ![0, 1].includes(probe.status) || (probe.stderr || '').trim()) throw new Error('Unable to establish database holders');
  return [...new Set((probe.stdout || '').trim().split(/\s+/).filter(p => /^\d+$/.test(p)))];
}

function recoverySqlite() {
  for (const dir of (process.env.PATH || '').split(delimiter).filter(Boolean)) {
    const binary = join(dir, process.platform === 'win32' ? 'sqlite3.exe' : 'sqlite3');
    const probe = spawnSync(binary, [':memory:', '.help recover'], { encoding: 'utf8', timeout: 3000 });
    if (!probe.error && probe.status === 0 && /\.recover/.test(probe.stdout)) return binary;
  }
  throw new Error('No sqlite3 executable with .recover support found');
}

async function backupDatabase(dataDir, backup) {
  const files = [];
  for (const name of ['verenu.db', 'verenu.db-wal', 'verenu.db-shm']) {
    const source = join(dataDir, name);
    try {
      const info = await lstat(source);
      if (!info.isFile() || info.isSymbolicLink()) throw new Error('Database files must be regular files');
      files.push(source);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  if (!files.includes(join(dataDir, 'verenu.db'))) throw new Error('Database missing');
  const active = holders(files);
  if (active.length) throw new Error(`Database has active holders: ${active.join(', ')}`);
  for (const source of files) await cp(source, join(backup, source.slice(dataDir.length + 1)), { errorOnExist: true, force: false });
  return files;
}
export async function fix(dataDir, id, { typedYes = false } = {}) {
  if (!['clear-caches', 'reset-settings', 'checkpoint-db', 'recover-db'].includes(id)) throw new Error('Unknown fix');
  if (['checkpoint-db', 'recover-db'].includes(id) && !typedYes) throw new Error('Typed yes required');
  const backupRoot = join(dataDir, 'triage-backups');
  await mkdir(backupRoot, { recursive: true, mode: 0o700 });
  const backup = await mkdtemp(join(backupRoot, `${Date.now()}-`));
  if (id === 'checkpoint-db') {
    const files = await backupDatabase(dataDir, backup);
    const active = holders(files);
    if (active.length) throw new Error(`Database has active holders: ${active.join(', ')}`);
    const checkpoint = spawnSync('sqlite3', [join(dataDir, 'verenu.db'), 'PRAGMA wal_checkpoint(TRUNCATE);'], { encoding: 'utf8', timeout: 10000 });
    if (checkpoint.error || checkpoint.status !== 0 || !/^0\|/.test(checkpoint.stdout.trim())) throw new Error('SQLite checkpoint failed or was busy');
    return backup;
  }
  if (id === 'recover-db') {
    const sqlite = recoverySqlite();
    const files = await backupDatabase(dataDir, backup);
    if (files.length !== 1) throw new Error('Database sidecars present; checkpoint or inspect them before recovery');
    const recovered = join(dataDir, 'verenu.recovered.db');
    try { await lstat(recovered); throw new Error('Recovery target already exists'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const dump = spawnSync(sqlite, ['-readonly', files[0], '.recover'], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, timeout: 120000 });
    if (dump.error || dump.status !== 0) throw new Error('SQLite recovery failed');
    await writeFile(recovered, '', { flag: 'wx', mode: 0o600 });
    const restore = spawnSync(sqlite, [recovered], { input: dump.stdout, encoding: 'utf8', timeout: 120000 });
    if (restore.error || restore.status !== 0) throw new Error('Recovered database could not be created');
    const integrity = spawnSync(sqlite, ['-readonly', recovered, 'PRAGMA integrity_check;'], { encoding: 'utf8', timeout: 30000 });
    if (integrity.error || integrity.status !== 0 || integrity.stdout.trim() !== 'ok') throw new Error('Recovered database failed integrity check');
    if (holders(files).length) throw new Error('Database acquired a holder during recovery');
    for (const suffix of ['-wal', '-shm']) {
      try { await lstat(`${files[0]}${suffix}`); throw new Error('Database sidecar appeared during recovery'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    if (await hashFile(files[0]) !== await hashFile(join(backup, 'verenu.db'))) throw new Error('Database changed during recovery');
    await rename(recovered, files[0]);
    return backup;
  }
  if (id === 'reset-settings') {
    const source = join(dataDir, 'settings.json');
    const info = await lstat(source);
    if (!info.isFile() || info.isSymbolicLink()) throw new Error('Settings must be a regular file');
    const original = await readFile(source);
    try { const parsed = JSON.parse(original); if (parsed && !Array.isArray(parsed) && typeof parsed === 'object') throw new Error('Settings are valid'); } catch (error) { if (error.message === 'Settings are valid') throw error; }
    await writeFile(join(backup, 'settings.json'), original, { flag: 'wx', mode: 0o600 });
    if (!(await readFile(source)).equals(original)) throw new Error('Settings changed during backup');
    let restored = '{}\n';
    const candidates = (await readdir(backupRoot)).filter(name => join(backupRoot, name) !== backup).sort().reverse();
    for (const candidate of candidates) {
      try { const bytes = await readFile(join(backupRoot, candidate, 'settings.json'), 'utf8'); const parsed = JSON.parse(bytes); if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) { restored = bytes; break; } } catch {}
    }
    const replacement = join(dataDir, `.triage-settings-${process.pid}-${Date.now()}.tmp`);
    await writeFile(replacement, restored, { flag: 'wx', mode: 0o600 });
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
