import { readdir, readFile, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { nativeChecks } from './native.js';

// Every check here is read-only. Nothing in this file changes a file or sends a request.
const result = (name, status, message) => ({ name, status, message });
const exists = p => stat(p).then(() => true, () => false);
const list = async p => readdir(p).catch(() => []);

export const checkIds = Object.freeze(['settings', 'sqlite', 'cache', 'terms', 'launch', 'keyring', 'provider-keys', 'models', 'process', 'logs']);

export async function checks({ dataDir: dir, env = process.env, platform, run = spawnSync }) {
  const out = [];

  const settings = join(dir, 'settings.json');
  let settingsValue;
  try {
    const value = JSON.parse(await readFile(settings, 'utf8'));
    if (!value || Array.isArray(value) || typeof value !== 'object') throw Error('settings must be an object');
    settingsValue = value;
    out.push(result('settings', 'pass', 'Settings are valid JSON.'));
  } catch {
    out.push(result('settings', await exists(settings) ? 'fail' : 'warn', 'Settings missing or invalid.'));
  }

  const db = join(dir, 'verenu.db');
  if (!await exists(db)) {
    out.push(result('sqlite', 'warn', 'Database not found. Check the data directory.'));
  } else {
    const probe = spawnSync('sqlite3', ['-readonly', '-cmd', '.timeout 1000', db, 'PRAGMA integrity_check;'], { encoding: 'utf8', timeout: 3000 });
    const ok = probe.status === 0 && probe.stdout.trim() === 'ok';
    const busy = /(?:locked|busy)/i.test(probe.stderr || '');
    out.push(ok
      ? result('sqlite', 'pass', 'Database integrity_check passed.')
      : probe.error
        ? result('sqlite', 'warn', 'Database check unavailable. Install sqlite3 and retry.')
        : busy
          ? result('sqlite', 'warn', 'Database is locked. Close Verenu before any database work.')
          : result('sqlite', 'fail', 'Database integrity check failed.'));
  }

  const caches = (await Promise.all(['icon-cache', 'favicon-cache'].map(async n => await exists(join(dir, n)) ? n : null))).filter(Boolean);
  out.push(caches.length
    ? result('cache', 'warn', `Icon caches exist (${caches.join(', ')}). Clear them if icons are stale.`)
    : result('cache', 'pass', 'No icon caches found.'));

  let consent = false;
  try { consent = JSON.parse(await readFile(join(dir, 'terms-consent.json'), 'utf8')).accepted === true; } catch {}
  out.push(consent
    ? result('terms', 'pass', 'Terms acceptance recorded.')
    : result('terms', 'warn', 'Terms acceptance unknown or not accepted. Review Terms in Verenu.'));

  out.push(...await nativeChecks({ dataDir: dir, settings: settingsValue, env, platform, run }));

  const files = await list(join(dir, 'logs'));
  let errors = 0;
  for (const file of files.slice(0, 20)) {
    try {
      const p = join(dir, 'logs', file);
      const info = await stat(p);
      if (info.isFile() && info.size < 5_000_000) errors += ((await readFile(p, 'utf8')).match(/\b(?:error|fatal)\b/gi) || []).length;
    } catch {}
  }
  out.push(errors
    ? result('logs', 'warn', 'Error markers found in logs. Inspect logs locally; they may contain private data.')
    : result('logs', 'pass', 'No error markers found in readable bounded logs.'));

  return out;
}
