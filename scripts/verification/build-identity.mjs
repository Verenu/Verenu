import fs from 'node:fs/promises';
import path from 'node:path';

export async function syncBuildIdentityFile(file, fingerprint) {
  if (typeof fingerprint !== 'string' || !/^[a-f0-9]{64}$/.test(fingerprint)) {
    throw new TypeError('Build identity needs a lowercase SHA-256 fingerprint');
  }
  const contents = `${fingerprint}\n`;
  const current = await fs.readFile(file, 'utf8').catch(error => {
    if (error.code === 'ENOENT') return null;
    throw error;
  });
  if (current === contents) return false;
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, contents, { mode: 0o600 });
  return true;
}
