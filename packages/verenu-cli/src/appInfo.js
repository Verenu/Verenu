import { open, readdir } from 'node:fs/promises';
import { join } from 'node:path';

// The app writes this line at startup, in every session log:
//   [timestamp] INFO  [verenu] verenu 0.21.0-beta.1 starting — linux x86_64, release build — verbose logging off
const startupLine = /verenu (\S+) starting — (\S+) (\S+), (\w+) build/;
const HEAD_BYTES = 4096;

/**
 * Read the app version, platform, and build mode from the newest session log.
 * Only those four fields leave this function. No other log text is returned.
 * Returns null when no session log has a startup line.
 */
export async function readAppInfo(dataDir) {
  let names;
  try {
    names = (await readdir(join(dataDir, 'logs'))).filter(n => /^session-.*\.log$/.test(n)).sort().reverse();
  } catch {
    return null;
  }
  for (const name of names.slice(0, 5)) {
    const file = await open(join(dataDir, 'logs', name), 'r').catch(() => null);
    if (!file) continue;
    try {
      const buffer = Buffer.alloc(HEAD_BYTES);
      const { bytesRead } = await file.read(buffer, 0, HEAD_BYTES, 0);
      const match = startupLine.exec(buffer.subarray(0, bytesRead).toString('utf8'));
      if (match) return { version: match[1], os: match[2], arch: match[3], build: match[4], sessionLog: name };
    } finally {
      await file.close();
    }
  }
  return null;
}
