import { readFileSync } from 'node:fs';
import { join } from 'node:path';

export function readDemoVersion(repo) {
  const { version } = JSON.parse(readFileSync(join(repo, 'package.json'), 'utf8'));
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+(?:-[\w.-]+)?(?:\+[\w.-]+)?$/.test(version)) {
    throw new Error('Demo requires a valid package.json version');
  }
  return version;
}
