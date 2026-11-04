import os from 'node:os';
import path from 'node:path';
import { writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline/promises';
import { checks } from './checks/index.js';
import { applyFix } from './fixes/index.js';
import { bundle } from './bundle.js';
import { report } from './report.js';

export async function main(args) {
  if (args.shift() !== 'triage') { console.error('Usage: verenu triage [--data-dir PATH] [--yes] [--json] [--bundle] [--check-network]'); process.exitCode = 2; return; }
  let dataDir = process.env.VERENU_DATA_DIR || (process.platform === 'win32' ? path.join(process.env.APPDATA || os.homedir(), 'Verenu') : process.platform === 'darwin' ? path.join(os.homedir(), 'Library/Application Support/Verenu') : path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config'), 'Verenu'));
  let yes = false, json = false, makeBundle = false, checkNetwork = false;
  while (args.length) {
    const arg = args.shift();
    if (arg === '--data-dir' && args.length) dataDir = path.resolve(args.shift());
    else if (arg === '--yes') yes = true;
    else if (arg === '--json') json = true;
    else if (arg === '--bundle') makeBundle = true;
    else if (arg === '--check-network') checkNetwork = true;
    else { console.error(`Unknown or incomplete option: ${arg}`); process.exitCode = 2; return; }
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
