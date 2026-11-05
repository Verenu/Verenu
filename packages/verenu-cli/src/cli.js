import path from 'node:path';
import { cliVersion, defaultDataDir, runTriage, TRIAGE_AGENTS } from './triage.js';

export const usage = `Usage: verenu triage [options]

Investigate a Verenu problem on this machine with Claude Code or Codex, and help file a good issue.

Options:
  --agent claude|codex   Agent CLI to use. Default: ask when both are installed.
  --model MODEL          Model passed through to the agent CLI. Default: the agent's default.
  --data-dir PATH        Verenu data directory. Default: the platform app data directory.
  -h, --help             Show this help and exit
  -v, --version          Show the CLI version and exit`;

const agentIds = TRIAGE_AGENTS.map(agent => agent.id);

export async function main(args) {
  if (args[0] === '--help' || args[0] === '-h') { console.log(usage); return; }
  if (args[0] === '--version' || args[0] === '-v') { console.log(await cliVersion()); return; }
  if (args.shift() !== 'triage') { console.error(usage); process.exitCode = 2; return; }

  let dataDir;
  let agent;
  let model;
  while (args.length) {
    const arg = args.shift();
    if (arg === '--help' || arg === '-h') { console.log(usage); return; }
    if (arg === '--agent' && args.length) {
      agent = args.shift();
      if (!agentIds.includes(agent)) { console.error(`--agent must be one of: ${agentIds.join(', ')}`); process.exitCode = 2; return; }
    } else if (arg === '--model' && args.length) {
      model = args.shift();
    } else if (arg === '--data-dir' && args.length) {
      dataDir = path.resolve(args.shift());
    } else {
      console.error(`Unknown or incomplete option: ${arg}\nRun \`verenu triage --help\` for the options.`);
      process.exitCode = 2;
      return;
    }
  }

  try {
    const { exitCode } = await runTriage({ dataDir: dataDir ?? defaultDataDir(), agent, model });
    if (exitCode !== 0) process.exitCode = exitCode;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
