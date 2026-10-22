import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

// Treat attempted rustc executions as failures too. Cargo's compiler queries
// count, even when a later error prevents compilation.
export function executions(trace) {
  const result = [];
  for (const line of trace.split('\n')) {
    if (!/\bexecve(?:at)?\(/.test(line)) continue;
    const match = line.match(/\bexecve(?:at)?\((?:[^,]+, )?("(?:\\.|[^"\\])*"), \[("(?:\\.|[^"\\])*"|\])/);
    if (!match) throw new Error('Cannot decode an execution in the process trace');
    const executable = JSON.parse(match[1]);
    const argv0 = match[2] === ']' ? '' : JSON.parse(match[2]);
    result.push({ executable, argv0, rustc: [executable, argv0].some(value => /^(?:rustc|rustc-real)(?:\.exe)?$/.test(path.basename(value))) });
  }
  if (!result.length) throw new Error('Process trace contains no executions');
  return result;
}

export function auditBuild({ command, args, directory, artifact, env, timeout = 600_000 }) {
  if (fs.existsSync(artifact)) throw new Error('Refusing to audit a pre-existing build artifact');
  const tracePath = path.join(directory, 'executions.log');
  const result = spawnSync('strace', ['--kill-on-exit', '-f', '-qq', '-s', '4096', '-e', 'trace=execve,execveat', '-o', tracePath, '--', command, ...args], {
    cwd: directory, env, encoding: 'utf8', timeout, killSignal: 'SIGKILL', maxBuffer: 32 * 1024 * 1024,
  });
  const audit = { command, args, status: result.status, signal: result.signal, error: result.error?.message,
    stdout: result.stdout || '', stderr: result.stderr || '', tracePath, artifact, failures: [], executions: [] };
  if (result.error || result.status !== 0) audit.failures.push('Build command did not finish successfully');
  try {
    audit.executions = executions(fs.readFileSync(tracePath, 'utf8'));
    if (audit.executions.some(item => item.rustc)) audit.failures.push('rustc command requested');
  } catch (error) { audit.failures.push(`Process audit unavailable: ${error.message}`); }
  try {
    const fd = fs.openSync(artifact, 'r');
    const magic = Buffer.alloc(4);
    try { fs.readSync(fd, magic, 0, 4, 0); } finally { fs.closeSync(fd); }
    if (!magic.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) || !(fs.statSync(artifact).mode & 0o111)) {
      audit.failures.push('Build did not produce an executable ELF artifact');
    }
  } catch { audit.failures.push('Build did not produce an executable artifact'); }
  audit.passed = audit.failures.length === 0;
  return audit;
}
