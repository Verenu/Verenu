#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { root, git, sourceIdentity } from './verification/identity.mjs';
import { stopOwned } from './verification/process.mjs';

const args = process.argv.slice(2);
const tasks = JSON.parse(await fs.readFile(new URL('../tests/agent-evals/tasks.json', import.meta.url), 'utf8')).tasks;
const option = (name, fallback) => { const index = args.indexOf(name); if (index < 0) return fallback; if (!args[index + 1]) throw new Error(`${name} requires a value`); return args[index + 1]; };
if (args.includes('--list') || args.includes('--help')) {
  console.log('npm run test:agent-evals -- --agent-command \'["agent-cli","{prompt}"]\' [--trials 3] [--task ID] [--timeout 900]');
  console.log(tasks.map((task) => `${task.id}: ${task.request}`).join('\n'));
  process.exit(0);
}
const command = JSON.parse(option('--agent-command', 'null'));
if (!Array.isArray(command) || !command.length || !command.every((part) => typeof part === 'string')) throw new Error('Provide a shell-free --agent-command JSON array; {prompt} is replaced by the task prompt');
const trials = Number(option('--trials', '3'));
const timeout = Number(option('--timeout', '900')) * 1000;
if (!Number.isSafeInteger(trials) || trials < 1 || trials > 100 || !Number.isFinite(timeout) || timeout < 1000 || timeout > 3_600_000) throw new Error('Invalid trial count or timeout');
const selected = tasks.filter((task) => !option('--task', '') || task.id === option('--task', ''));
if (!selected.length) throw new Error('No evaluation task matched');
const directory = path.join(root, 'test-results', `agent-evals-${randomUUID()}`);
await fs.mkdir(directory, { recursive: true, mode: 0o700 });
const files = [...new Set(git(['ls-files', '-z', '--cached', '--others', '--exclude-standard']).split('\0').filter(Boolean))];
const results = [];
async function execute(argv, cwd, log) {
  const output = await fs.open(log, 'w', 0o600);
  const started = Date.now();
  try {
    return await new Promise((resolve) => {
      const child = spawn(argv[0], argv.slice(1), { cwd, stdio: ['ignore', output.fd, output.fd], detached: process.platform !== 'win32' });
      let forceTimer;
      let timedOut = false;
      const timer = setTimeout(() => { timedOut = true; stopOwned(child); forceTimer = setTimeout(() => stopOwned(child, 'SIGKILL'), 5000); }, timeout);
      child.once('error', () => { clearTimeout(timer); clearTimeout(forceTimer); resolve({ code: null, durationMs: Date.now() - started }); });
      child.once('exit', (code) => { clearTimeout(timer); clearTimeout(forceTimer); resolve({ code: timedOut ? null : code, durationMs: Date.now() - started, timedOut }); });
    });
  } finally { await output.close(); }
}
for (const task of selected) {
  for (let trial = 1; trial <= trials; trial++) {
    const sandbox = path.join(directory, `${task.id}-${trial}`);
    await fs.mkdir(sandbox);
    for (const file of files.filter((name) => !name.startsWith('tests/agent-evals/'))) {
      const destination = path.join(sandbox, file);
      await fs.mkdir(path.dirname(destination), { recursive: true });
      try { await fs.copyFile(path.join(root, file), destination); } catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    git(['init', '--initial-branch=master'], sandbox);
    git(['config', 'user.name', 'Evaluation fixture'], sandbox);
    git(['config', 'user.email', 'fixture@invalid'], sandbox);
    git(['add', '.'], sandbox);
    git(['commit', '-m', 'Disposable evaluation snapshot'], sandbox);
    const target = path.join(sandbox, task.file);
    const original = await fs.readFile(target, 'utf8');
    if (!original.includes(task.before)) throw new Error(`Evaluation seed drifted: ${task.id}`);
    await fs.writeFile(target, original.replace(task.before, task.after));
    const oracle = path.join(root, 'tests/agent-evals/oracle.py');
    const python = process.platform === 'win32' ? 'python' : 'python3';
    const before = await execute([python, '-B', oracle, sandbox], root, path.join(directory, `${task.id}-${trial}-before.log`));
    if (before.code === 0) throw new Error(`Seed did not reproduce the bug: ${task.id}`);
    const prompt = `${task.request}\nWork only in ${sandbox}. Do not weaken tests. Finish with verified checks and explicit gaps. Write agent-result.json with claimedComplete, checks, skippedChecks, providerCostUsd, and summary. Use public synthetic data only.`;
    const agent = await execute(command.map((part) => part.replaceAll('{prompt}', prompt)), sandbox, path.join(directory, `${task.id}-${trial}-agent.log`));
    const outcome = await execute([python, '-B', oracle, sandbox], root, path.join(directory, `${task.id}-${trial}-after.log`));
    let claim = {};
    try { claim = JSON.parse(await fs.readFile(path.join(sandbox, 'agent-result.json'), 'utf8')); } catch { /* Missing claims remain explicit. */ }
    results.push({ task: task.id, trial, correct: outcome.code === 0, agentExitCode: agent.code, falseDone: claim.claimedComplete === true && outcome.code !== 0, missingReport: typeof claim.claimedComplete !== 'boolean', durationMs: agent.durationMs, reportedChecks: claim.checks || [], reportedSkips: claim.skippedChecks || [], reportedProviderCostUsd: claim.providerCostUsd ?? null });
    console.log(`${task.id} trial ${trial}: ${outcome.code === 0 ? 'passed' : 'failed'}`);
  }
}
await fs.writeFile(path.join(directory, 'results.json'), JSON.stringify({ schemaVersion: 1, identity: sourceIdentity(), agentCommand: command[0], results, summary: { trials: results.length, correct: results.filter((row) => row.correct).length, falseDone: results.filter((row) => row.falseDone).length, missingReports: results.filter((row) => row.missingReport).length } }, null, 2), { mode: 0o600 });
console.log(`Private evaluation results: ${path.join(directory, 'results.json')}`);
process.exitCode = results.every((row) => row.correct && !row.missingReport) ? 0 : 1;
