import os from 'node:os';
import path from 'node:path';
import { readFile, writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline/promises';
import { checks } from './checks/index.js';
import { applyFix } from './fixes/index.js';
import { bundle } from './bundle.js';
import { report } from './report.js';

export const usage = `Usage: verenu triage [options]

Options:
  --data-dir PATH     Verenu data directory (default: the platform app data directory)
  --json              Print a machine-readable report
  --yes               Approve cache and settings fixes without prompting
  --bundle            Write verenu-triage-bundle.json with check names and statuses only
  --check-network     Run the network check without recorded Terms consent
  -h, --help          Show this help and exit
  -v, --version       Show the CLI version and exit`;

async function version() {
  return JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8')).version;
}

export async function main(args) {
  if (args[0] === '--help' || args[0] === '-h') { console.log(usage); return; }
  if (args[0] === '--version' || args[0] === '-v') { console.log(await version()); return; }
  if (args.shift() !== 'triage') { console.error(usage); process.exitCode = 2; return; }
  let dataDir = process.env.VERENU_DATA_DIR || (process.platform === 'win32' ? path.join(process.env.APPDATA || os.homedir(), 'Verenu') : process.platform === 'darwin' ? path.join(os.homedir(), 'Library/Application Support/Verenu') : path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config'), 'Verenu'));
  let yes = false, json = false, makeBundle = false, checkNetwork = false;
  while (args.length) {
    const arg = args.shift();
    if (arg === '--data-dir' && args.length) dataDir = path.resolve(args.shift());
    else if (arg === '--yes') yes = true;
    else if (arg === '--json') json = true;
    else if (arg === '--bundle') makeBundle = true;
    else if (arg === '--check-network') checkNetwork = true;
    else if (arg === '--help' || arg === '-h') { console.log(usage); return; }
    else { console.error(`Unknown or incomplete option: ${arg}\nRun \`verenu triage --help\` for the options.`); process.exitCode = 2; return; }
  }
  const results = await checks({ dataDir, checkNetwork });
  const actions = [];
  if (!json) console.log(report(results));
  for (const id of [...new Set(results.map(r => r.fix).filter(Boolean))]) {
    const transcriptAction = ['checkpoint-db', 'recover-db'].includes(id);
    let approved = yes && !transcriptAction;
    if ((!yes || transcriptAction) && !json && process.stdin.isTTY && process.stdout.isTTY) {
      const rl = createInterface({ input: process.stdin, output: process.stdout });
      try { const answer = (await rl.question(transcriptAction ? `Back up and apply ${id}, which touches transcript data? Type yes to confirm: ` : `Back up and apply ${id}? [y/N] `)).trim(); approved = transcriptAction ? answer === 'yes' : answer.toLowerCase() === 'y'; } finally { rl.close(); }
    }
    if (approved) { try { actions.push({ fix: id, backup: await applyFix(dataDir, id, { approved, typedYes: transcriptAction }) }); } catch (error) { actions.push({ fix: id, error: error.message }); process.exitCode = 1; } }
  }
  if (makeBundle) { const target = path.resolve('verenu-triage-bundle.json'); await writeFile(target, bundle(results), { flag: 'wx', mode: 0o600 }); actions.push({ bundle: target }); }
  if (json) console.log(JSON.stringify({ results, actions }, null, 2));
  else for (const action of actions) console.log(JSON.stringify(action));
}
