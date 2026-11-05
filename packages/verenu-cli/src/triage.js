/**
 * `verenu triage` hands a misbehaving install to the user's own coding agent.
 *
 * The command writes a `context.md` with machine facts and the results of
 * read-only checks, then launches Claude Code or Codex interactively, seeded
 * with the playbook in PLAYBOOK.md. The agent asks the user what went wrong,
 * investigates, and files the issue. The agent's own permission prompts gate
 * anything it wants to run. With no agent CLI installed, the prompt and context
 * are written to disk for the user to paste into an agent they do have.
 *
 * This command makes no network requests itself. The playbook lets the agent
 * read from GitHub freely, and requires the user's yes before anything is posted.
 */
import os from 'node:os';
import path from 'node:path';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline/promises';
import { checks } from './checks/index.js';
import { readAppInfo } from './appInfo.js';
import { buildTriageContext, buildTriageLaunchPrompt, buildTriageSeedPrompt, loadPlaybook } from './triagePrompt.js';

export const TRIAGE_AGENTS = Object.freeze([
  { id: 'claude', command: 'claude', label: 'Claude Code' },
  { id: 'codex', command: 'codex', label: 'Codex' },
]);

/** Default data directory, matching the app's own resolution. */
export function defaultDataDir({ env = process.env, platform = process.platform, home = os.homedir() } = {}) {
  if (env.VERENU_APP_DATA_DIR_OVERRIDE) return env.VERENU_APP_DATA_DIR_OVERRIDE;
  if (platform === 'win32') return path.join(env.APPDATA || home, 'Verenu');
  if (platform === 'darwin') return path.join(home, 'Library/Application Support/Verenu');
  return path.join(env.XDG_CONFIG_HOME || path.join(home, '.config'), 'Verenu');
}

export function isCommandAvailable(command, { platform = process.platform, run = spawnSync } = {}) {
  const probe = run(platform === 'win32' ? 'where' : 'which', [command], { stdio: 'ignore' });
  return !probe.error && probe.status === 0;
}

export class TriageError extends Error {}

export async function cliVersion() {
  return JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8')).version;
}

/** Run the agent CLI as a normal interactive session. Resolves with its exit code. */
export function runInteractiveSession({ command, args, cwd }) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit', shell: process.platform === 'win32' });
    child.once('error', reject);
    // Signal death has no exit code, so report failure rather than success.
    child.once('exit', (code, signal) => resolve(code ?? (signal === null ? 0 : 1)));
  });
}

async function pickAgent(agents) {
  const readline = createInterface({ input: process.stdin, output: process.stdout });
  try {
    const menu = agents.map((agent, index) => `  [${index + 1}] ${agent.label}`).join('\n');
    for (;;) {
      const answer = (await readline.question(`Run triage with:\n${menu}\n> `)).trim();
      const byNumber = agents[Number.parseInt(answer, 10) - 1];
      if (byNumber !== undefined) return byNumber;
      const byId = agents.find(agent => agent.id === answer.toLowerCase());
      if (byId !== undefined) return byId;
    }
  } finally {
    readline.close();
  }
}

/**
 * @param {object} options
 * @param {string} options.dataDir Verenu data directory.
 * @param {string} [options.agent] `claude` or `codex`. Default: ask or pick the one installed.
 * @param {string} [options.model] Model passed through to the agent CLI.
 * @param {Date} [options.now]
 * @param {boolean} [options.interactive] Both stdin and stdout are terminals.
 * @param {(command: string) => boolean} [options.available]
 * @param {(session: object) => Promise<number>} [options.launch]
 * @param {(line: string) => void} [options.log]
 * @returns {Promise<{exitCode: number, scratchDir: string, contextFile: string, promptFile: string}>}
 */
export async function runTriage({
  dataDir,
  agent: requested,
  model,
  now = new Date(),
  interactive = Boolean(process.stdin.isTTY && process.stdout.isTTY),
  available = command => isCommandAvailable(command),
  launch = runInteractiveSession,
  log = console.log,
}) {
  // The agent runs in one stable workspace, so the one-time "trust this folder"
  // answer persists across runs. Each run writes its files to its own subfolder.
  // The instant format matches T3 Code, made safe for Windows paths.
  const workspaceDir = path.join(dataDir, 'triage', 'workspace');
  const scratchDir = path.join(workspaceDir, now.toISOString().replaceAll(':', '-').replace('.', '-'));
  await mkdir(scratchDir, { recursive: true });

  const contextFile = path.join(scratchDir, 'context.md');
  const promptFile = path.join(scratchDir, 'prompt.md');
  const results = await checks({ dataDir });
  const app = await readAppInfo(dataDir);
  await writeFile(contextFile, buildTriageContext({
    generatedAt: now.toISOString(),
    version: await cliVersion(),
    os: `${process.platform} ${process.arch} (${os.release()})`,
    nodeVersion: process.version,
    launchedAs: `${process.argv[1] ?? 'verenu'} triage`,
    session: process.platform === 'linux'
      ? `${process.env.XDG_SESSION_TYPE || 'unknown'} session, desktop ${process.env.XDG_CURRENT_DESKTOP || 'unknown'}`
      : `not Linux (${process.platform})`,
    app,
    releaseTag: app ? `v${app.version}` : null,
    sourceCacheDir: path.join(dataDir, 'triage', 'workspace', 'source'),
    checks: results,
    paths: {
      dataDir,
      settingsPath: path.join(dataDir, 'settings.json'),
      dbPath: path.join(dataDir, 'verenu.db'),
      logsDir: path.join(dataDir, 'logs'),
      agentSessionPath: path.join(dataDir, 'agent-session.json'),
      modelsDir: path.join(dataDir, 'models'),
      backupsDir: path.join(dataDir, 'triage-backups'),
    },
  }));
  await writeFile(promptFile, buildTriageSeedPrompt(contextFile, await loadPlaybook()));

  const installed = TRIAGE_AGENTS.filter(agent => available(agent.command));
  let selected;
  if (requested !== undefined) {
    selected = installed.find(agent => agent.id === requested);
    if (selected === undefined) throw new TriageError(`\`${requested}\` is not installed or was not found on PATH.`);
  } else if (installed.length === 1) {
    [selected] = installed;
  } else if (installed.length > 1) {
    // Both streams must be terminals, or the picker prompt is invisible and the command hangs.
    if (!interactive) throw new TriageError('Both claude and codex are installed and there is no terminal to ask which to use. Re-run with --agent claude or --agent codex.');
    selected = await pickAgent(installed);
  }

  if (selected === undefined) {
    log([
      'No supported agent CLI (claude, codex) was found on this machine.',
      '',
      'The triage prompt and machine context were written to:',
      `  ${promptFile}`,
      `  ${contextFile}`,
      '',
      'Paste the prompt file into any coding agent to run triage by hand.',
    ].join('\n'));
    return { exitCode: 0, scratchDir, contextFile, promptFile };
  }

  log(`Starting ${selected.label}. It will ask what went wrong.\n`);
  const exitCode = await launch({
    command: selected.command,
    args: [...(model === undefined ? [] : ['--model', model]), buildTriageLaunchPrompt(promptFile)],
    cwd: workspaceDir,
  });
  return { exitCode, scratchDir, contextFile, promptFile };
}
