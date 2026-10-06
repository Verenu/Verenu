import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { root } from '../verification/identity.mjs';

// Pin and verify the official binary, including local runs outside a Go setup.
const version = '1.7.12';
const platform = { linux: 'linux', darwin: 'darwin', win32: 'windows' }[process.platform];
const arch = { x64: 'amd64', arm64: 'arm64' }[process.arch];
if (!platform || !arch) {
  console.error('Workflow linting incomplete: unsupported actionlint platform.');
  process.exit(2);
}
const directory = path.join(root, 'test-results', `actionlint-${platform}-${arch}-${version}`);
const executable = path.join(directory, process.platform === 'win32' ? 'actionlint.exe' : 'actionlint');
await fs.mkdir(directory, { recursive: true });
const archive = `actionlint_${version}_${platform}_${arch}.${process.platform === 'win32' ? 'zip' : 'tar.gz'}`;
const base = `https://github.com/rhysd/actionlint/releases/download/v${version}`;
async function download(name) {
  const response = await fetch(`${base}/${name}`, { signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error(`actionlint download failed: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}
// Verify the archive on every invocation so an interrupted download is never used.
const [data, checksums] = await Promise.all([download(archive), download(`actionlint_${version}_checksums.txt`)]);
const expected = checksums.toString().split('\n').find(line => line.endsWith(`  ${archive}`))?.split(/\s+/)[0];
if (!expected || createHash('sha256').update(data).digest('hex') !== expected) throw new Error('actionlint checksum mismatch');
const archivePath = path.join(directory, archive);
await fs.writeFile(archivePath, data);
const extraction = spawnSync('tar', ['-xf', archivePath, '-C', directory], { timeout: 30_000, encoding: 'utf8' });
if (extraction.status !== 0) throw new Error('Could not extract verified actionlint archive');
const checked = spawnSync(executable, ['-shellcheck=', '-pyflakes='], { cwd: root, stdio: 'inherit', timeout: 60_000 });
if (checked.error) throw checked.error;
process.exitCode = checked.status || (checked.signal ? 1 : 0);
