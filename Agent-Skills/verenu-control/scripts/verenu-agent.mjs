#!/usr/bin/env node
import fs from 'node:fs/promises';
import { constants } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline/promises';
import { randomBytes } from 'node:crypto';

const skillRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const harnesses = [
  { id: 'codex', config: '.codex', skills: '.agents/skills' },
  { id: 'claude', config: '.claude', skills: '.claude/skills' },
  { id: 'cursor', config: '.cursor', skills: '.cursor/skills' },
  { id: 'grok', config: '.grok', skills: '.grok/skills' },
  { id: 'opencode', config: '.config/opencode', skills: '.config/opencode/skills' },
];
const exists = async (name) => fs.stat(name).then(() => true, () => false);

export async function detectedHarnesses(home = os.homedir()) {
  const rows = await Promise.all(harnesses.map(async (h) => ({ ...h, detected: await exists(path.join(home, h.config)), destination: path.join(home, h.skills, 'verenu-control') })));
  return rows.filter((h) => h.detected);
}

export async function installSkill(selected, { home = os.homedir(), permanent = false, replace = false } = {}) {
  if (!permanent) throw new Error('Permanent installation requires explicit consent');
  const detected = await detectedHarnesses(home);
  if (!selected.length || new Set(selected).size !== selected.length) throw new Error('Choose one or more distinct detected harnesses');
  const destinations = selected.map((id) => {
    const harness = detected.find((h) => h.id === id);
    if (!harness) throw new Error(`Harness not detected: ${id}`);
    return harness.destination;
  });
  // Preflight the entire selection before writing any harness.
  const source = await fs.realpath(skillRoot);
  const copies = [];
  for (const destination of destinations) {
    const stat = await fs.lstat(destination).catch((error) => { if (error.code !== 'ENOENT') throw error; });
    if (stat?.isSymbolicLink()) throw new Error('Refusing to replace a linked skill');
    if (stat && await fs.realpath(destination) === source) continue;
    if (stat && !replace) throw new Error('Skill already exists; inspect it and use --replace to update');
    copies.push(destination);
  }
  for (const destination of copies) {
    await fs.mkdir(path.dirname(destination), { recursive: true });
    await fs.cp(skillRoot, destination, { recursive: true, force: replace, errorOnExist: !replace });
  }
  return destinations;
}

function appDataDir(env = process.env) {
  if (env.VERENU_APP_DATA_DIR_OVERRIDE) return env.VERENU_APP_DATA_DIR_OVERRIDE;
  if (process.platform === 'win32') return path.join(env.APPDATA || '.', 'Verenu');
  if (process.platform === 'darwin') return path.join(os.homedir(), 'Library/Application Support/Verenu');
  return path.join(env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config'), 'Verenu');
}

async function grant(env = process.env) {
  if (env.VERENU_AGENT_TOKEN || env.VERENU_AGENT_PORT) return { token: env.VERENU_AGENT_TOKEN, port: Number(env.VERENU_AGENT_PORT) };
  if (!env.VERENU_AGENT_GRANT_FILE) throw new Error('No agent grant. Set VERENU_AGENT_GRANT_FILE to a private session path and run connect.');
  const file = await fs.open(env.VERENU_AGENT_GRANT_FILE, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > 1024 || (process.platform !== 'win32' && ((stat.mode & 0o077) || stat.uid !== process.getuid()))) throw new Error('Agent grant file must be a private regular file');
    return JSON.parse(await file.readFile('utf8'));
  } finally { await file.close(); }
}

async function authorizationRequest(port, endpoint, token) {
  if (!/^[a-f\d]{64}$/i.test(token ?? '') || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid agent grant');
  const response = await fetch(`http://127.0.0.1:${port}/v1/${endpoint}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ token }), redirect: 'error', signal: AbortSignal.timeout(5000) });
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `Agent API HTTP ${response.status}`);
  return body;
}

export async function connect(env = process.env) {
  if (env.VERENU_AGENT_TOKEN || env.VERENU_AGENT_PORT) throw new Error('Unset the environment grant before requesting an approval grant');
  if (!env.VERENU_AGENT_GRANT_FILE) throw new Error('Set VERENU_AGENT_GRANT_FILE to a private session path first');
  if (!path.isAbsolute(env.VERENU_AGENT_GRANT_FILE)) throw new Error('Agent grant file path must be absolute');
  const parent = await fs.stat(path.dirname(env.VERENU_AGENT_GRANT_FILE));
  if (!parent.isDirectory() || (process.platform !== 'win32' && ((parent.mode & 0o077) || parent.uid !== process.getuid()))) throw new Error('Agent grant directory must be private and owned by this user');
  const port = Number((await fs.readFile(path.join(appDataDir(env), 'agent-port'), 'utf8')).trim());
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid Verenu discovery file');
  const token = randomBytes(32).toString('hex');
  const file = await fs.open(env.VERENU_AGENT_GRANT_FILE, 'wx', 0o600);
  try {
    await file.writeFile(JSON.stringify({ token, port }));
    const result = await authorizationRequest(port, 'authorize', token);
    return result;
  } catch (error) {
    await fs.unlink(env.VERENU_AGENT_GRANT_FILE).catch(() => {});
    throw error;
  } finally { await file.close(); }
}

export async function invoke(command, args = {}, env = process.env) {
  const { token, port } = await grant(env);
  if (!/^[a-f\d]{64}$/i.test(token ?? '') || !Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error('Set VERENU_AGENT_TOKEN and VERENU_AGENT_PORT to the grant used to launch Verenu. This process must inherit the same grant; a separate terminal does not share its environment. See --help.');
  }
  const response = await fetch(`http://127.0.0.1:${port}/v1/invoke`, {
    method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(65000),
    redirect: 'error',
  });
  let result;
  try { result = await response.json(); } catch {
    if (!response.ok) throw new Error(`Agent API HTTP ${response.status}`);
    throw new Error('Agent API returned invalid JSON');
  }
  if (!response.ok) throw new Error(`Agent API HTTP ${response.status}: ${typeof result?.error === 'string' ? result.error : 'Request failed'}`);
  if (!result || typeof result !== 'object' || result.version !== 1 || !Object.hasOwn(result, 'result')) throw new Error('Unsupported agent API response');
  return result.result;
}

async function main(argv) {
  const [command, ...args] = argv;
  if (!command || command === '--help') {
    console.log(`Verenu agent CLI (Node 20+). The desktop app must be running.
Usage: node verenu-agent.mjs <command> '[JSON args]'
       node verenu-agent.mjs <command> --stdin
       node verenu-agent.mjs connect | status | disconnect
       node verenu-agent.mjs harnesses
       node verenu-agent.mjs install-skill
       node verenu-agent.mjs install-skill --permanent --harness codex,claude [--replace]

Use describe for the live API and supported settings. All responses are JSON.
Set VERENU_AGENT_GRANT_FILE to a new file in a private session directory, run connect,
and give the user the returned code to compare with the prompt in Verenu. After
approval, run status. Use the same grant-file path for every command. Access lasts
one hour or until revoked; disconnect revokes and deletes the grant file.
Never print or share the grant-file contents.

For scripted launches, a shared environment grant is also supported:
Linux/macOS (same shell for app and CLI):
  export VERENU_AGENT_TOKEN="$(node -e 'process.stdout.write(require("crypto").randomBytes(32).toString("hex"))')"
  export VERENU_AGENT_PORT=17421
  verenu
PowerShell:
  $env:VERENU_AGENT_TOKEN = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
  $env:VERENU_AGENT_PORT = '17421'
  & 'C:\\path\\to\\verenu.exe'

The CLI or agent must inherit these same variables; launching Verenu in a separate terminal does not grant access to an existing agent session.
Restart without these variables to revoke access. Existing app instances must be closed normally first.
Installation writes only selected detected harnesses. It does not enable the API or change global instructions.
Text history requires include_text:true on each history.page request. Output may be private.`);
    return;
  }
  if (command === 'connect') { console.log(JSON.stringify(await connect(), null, 2)); return; }
  if (command === 'status' || command === 'disconnect') {
    if (process.env.VERENU_AGENT_TOKEN || process.env.VERENU_AGENT_PORT) {
      throw new Error('Status and disconnect require an approval grant file. Revoke environment grants from Settings > Privacy or restart Verenu without the environment grant.');
    }
    const { token, port } = await grant();
    const result = await authorizationRequest(port, command === 'status' ? 'authorization-status' : 'revoke', token);
    if (command === 'disconnect' && process.env.VERENU_AGENT_GRANT_FILE) await fs.unlink(process.env.VERENU_AGENT_GRANT_FILE);
    console.log(JSON.stringify(result, null, 2)); return;
  }
  if (command === 'harnesses') { console.log(JSON.stringify(await detectedHarnesses(), null, 2)); return; }
  if (command === 'install-skill') {
    const detected = await detectedHarnesses();
    if (!detected.length) throw new Error('No supported harness configuration directories detected; use this skill from its current location');
    let selected;
    const permanent = args.includes('--permanent');
    if (permanent) {
      if (args.some((v, i) => v !== '--permanent' && v !== '--replace' && v !== '--harness' && args[i - 1] !== '--harness')) throw new Error('Unknown installation option');
      const index = args.indexOf('--harness');
      selected = index < 0 ? [] : (args[index + 1] ?? '').split(',').filter(Boolean);
    } else {
      if (args.length) throw new Error('Use --permanent with --harness for noninteractive installation');
      if (!process.stdin.isTTY) throw new Error('Interactive installation needs a terminal; explicitly choose --permanent --harness <ids> for scripts');
      const rl = createInterface({ input: process.stdin, output: process.stdout });
      try {
        console.log('Detected: ' + detected.map((h) => `${h.id} (${h.destination})`).join(', '));
        if (!/^y(es)?$/i.test((await rl.question('Install the Verenu skill permanently? [y/N] ')).trim())) return;
        selected = (await rl.question('Choose harness IDs, separated by commas (blank cancels): ')).split(',').map((s) => s.trim()).filter(Boolean);
        if (!selected.length) return;
      } finally { rl.close(); }
    }
    console.log(JSON.stringify({ installed: await installSkill(selected, { permanent: true, replace: args.includes('--replace') }) }, null, 2));
    return;
  }
  let input = '{}';
  if (args.length === 1 && args[0] === '--stdin') {
    input = '';
    for await (const chunk of process.stdin) {
      input += chunk;
      if (Buffer.byteLength(input) > 60000) throw new Error('Arguments exceed 60000 bytes');
    }
  } else if (args.length > 1) throw new Error('Expected one JSON argument or --stdin');
  else if (args.length === 1) input = args[0];
  console.log(JSON.stringify(await invoke(command, JSON.parse(input)), null, 2));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
