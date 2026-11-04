import { readdir, readFile, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
const result = (name, status, message, fix) => ({ name, status, message, ...(fix ? { fix } : {}) });
const exists = async p => stat(p).then(() => true, () => false);
const list = async p => readdir(p).catch(() => []);
export async function checks({ dataDir: dir, checkNetwork = false, fetcher = globalThis.fetch, env = process.env }) {
  const out = [];
  const present = await exists(dir);
  out.push(result('launch', present ? 'pass' : 'warn', present ? 'Data directory exists. This does not prove the app launches.' : 'Data directory missing. Install or start Verenu and check its location.'));
  const settings = join(dir, 'settings.json');
  try { const value = JSON.parse(await readFile(settings, 'utf8')); if (!value || Array.isArray(value) || typeof value !== 'object') throw Error(); out.push(result('settings', 'pass', 'Settings are valid JSON.')); }
  catch { out.push(result('settings', await exists(settings) ? 'fail' : 'warn', 'Settings missing or invalid. Restore a known good backup.')); }
  const db = join(dir, 'verenu.db');
  if (!await exists(db)) out.push(result('sqlite', 'warn', 'Database not found. Check the data directory.'));
  else {
    const probe = spawnSync('sqlite3', ['-readonly', '-cmd', '.timeout 1000', db, 'PRAGMA quick_check;'], { encoding: 'utf8', timeout: 3000 });
    const ok = probe.status === 0 && probe.stdout.trim() === 'ok';
    out.push(result('sqlite', ok ? 'pass' : probe.error?.code === 'ENOENT' ? 'warn' : 'fail', ok ? 'Database quick_check passed.' : 'Database check unavailable, locked, or corrupt. Close Verenu and inspect a backup with SQLite.'));
  }
  const found = (await Promise.all(['icon-cache', 'favicon-cache'].map(async n => await exists(join(dir, n)) ? n : null))).filter(Boolean);
  out.push(result('cache', found.length ? 'warn' : 'pass', found.length ? 'Icon caches exist. Clear them if icons are stale.' : 'No icon caches found.', found.length ? 'clear-caches' : undefined));
  const models = await list(join(dir, 'models'));
  out.push(result('models', models.length ? 'warn' : 'pass', models.length ? 'Model files exist. Verify downloads in Settings > Models if a model fails.' : 'No local model directory found; cloud setups may not need one.'));
  out.push(result('keyring', 'warn', 'Secure storage cannot be verified without accessing the native credential store. Check the system keyring is unlocked.'));
  out.push(result('provider-keys', 'warn', 'Provider keys are stored in native secure storage. Check their status in Settings > Providers.'));
  let consent = false;
  try { consent = JSON.parse(await readFile(join(dir, 'terms-consent.json'), 'utf8')).accepted === true; } catch {}
  out.push(result('terms', consent ? 'pass' : 'warn', consent ? 'Terms acceptance recorded.' : 'Terms acceptance unknown or not accepted. Review Terms in Verenu.'));
  const url = env.VERENU_CONVEX_URL;
  if (!(consent || checkNetwork)) out.push(result('network', 'warn', 'Skipped: Terms not accepted or unknown. Use --check-network to explicitly opt in.'));
  else if (!url) out.push(result('network', 'warn', 'Skipped: VERENU_CONVEX_URL is not configured.'));
  else {
    try { const parsed = new URL(url); if (parsed.protocol !== 'https:') throw Error(); const response = await fetcher(parsed.origin, { method: 'HEAD', signal: AbortSignal.timeout(5000), redirect: 'error' }); out.push(result('network', response.ok ? 'pass' : 'warn', response.ok ? 'Convex endpoint reachable.' : 'Convex endpoint returned an error. Check network and service status.')); }
    catch { out.push(result('network', 'fail', 'Convex endpoint unreachable or invalid. Check network and configuration.')); }
  }
  out.push(result('process', 'warn', 'Process and port ownership are not established. Check for another Verenu instance before restarting.'));
  const files = await list(join(dir, 'logs'));
  let errors = 0;
  for (const file of files.slice(0, 20)) { try { const p = join(dir, 'logs', file); const info = await stat(p); if (info.isFile() && info.size < 5_000_000) errors += ((await readFile(p, 'utf8')).match(/\b(?:error|fatal)\b/gi) || []).length; } catch {} }
  out.push(result('logs', errors ? 'warn' : 'pass', errors ? 'Error markers found in logs. Inspect logs locally; they may contain private data.' : 'No error markers found in readable bounded logs.'));
  return out;
}
