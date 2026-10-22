import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

// Both steps run inside the parent's process trace. The snapshot preserves
// Tauri's ../dist relationship without touching the installed app or its data.
const [lust, snapshot, artifact] = process.argv.slice(2);
if (!lust || !snapshot || !artifact) throw new Error('Expected compiler, backend snapshot, and output path');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const toolchain = JSON.parse(fs.readFileSync(new URL('./toolchain.json', import.meta.url), 'utf8'));
const version = spawnSync(lust, ['--version'], { encoding: 'utf8' });
if (version.error || version.status !== 0) throw new Error('Cannot read the Lust compiler version');
if (version.stdout.trim().split(/\s+/).slice(0, 2).join(' ') !== `lust ${toolchain.version}`) {
  throw new Error(`Expected Lust ${toolchain.version}`);
}
console.log(version.stdout.trim());
for (const [command, args] of [
  ['npm', ['run', 'build', '--', '--outDir', path.join(snapshot, '../dist'), '--emptyOutDir']],
  [lust, ['build', path.join(snapshot, 'src/main.rs'), '--manifest-path', path.join(snapshot, 'Cargo.toml'), '--bin', 'verenu', '--locked', '-o', artifact]],
]) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.error) { console.error(result.error.message); process.exit(1); }
  if (result.status !== 0) process.exit(result.status || 1);
}
