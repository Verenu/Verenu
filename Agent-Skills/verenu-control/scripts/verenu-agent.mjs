#!/usr/bin/env node
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline/promises';
import { constants as fsConstants } from 'node:fs';

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

function sessionPaths(env) {
  if (process.platform === 'win32') throw new Error('Automatic agent access is not available on Windows. See --help for the explicit grant.');
  if (env.VERENU_APP_DATA_DIR_OVERRIDE) return [path.join(env.VERENU_APP_DATA_DIR_OVERRIDE, 'agent-session.json')];
  const home = env.HOME || os.homedir();
  if (process.platform === 'darwin') return [path.join(home, 'Library', 'Application Support', 'Verenu', 'agent-session.json')];
  const standard = path.join(home, '.config');
  return [...new Set([env.XDG_CONFIG_HOME || standard, standard].map((base) => path.join(base, 'Verenu', 'agent-session.json')))];
}

async function readDescriptor(filename) {
  let file;
  try {
    file = await fs.open(filename, fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0));
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > 4096 || (process.getuid && (stat.uid !== process.getuid() || (stat.mode & 0o077) !== 0))) {
      throw new Error('The agent session file is not private to this user.');
    }
    let value;
    try { value = JSON.parse(await file.readFile('utf8')); }
    catch { throw new Error('Invalid Verenu agent session file.'); }
    if (!value || typeof value !== 'object' || value.version !== 1 || typeof value.instance !== 'string' || !/^[a-f\d-]{36}$/i.test(value.instance) || !Number.isInteger(value.port) || value.port < 1 || value.port > 65535 ||
        !['idle', 'pending', 'denied', 'approved'].includes(value.status) ||
        (value.status === 'approved' && (typeof value.token !== 'string' || !/^[a-f\d]{64}$/i.test(value.token)))) {
      throw new Error('Invalid Verenu agent session file.');
    }
    return value;
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  } finally {
    await file?.close();
  }
}

async function resolveSession(env) {
  const live = [];
  for (const filename of sessionPaths(env)) {
    const session = await readDescriptor(filename);
    if (!session) continue;
    try {
      const response = await fetch(`http://127.0.0.1:${session.port}/v1/status`, { redirect: 'error', signal: AbortSignal.timeout(1000) });
      if (!response.ok) continue;
      const status = await response.json();
      if (status.version === 1 && status.instance === session.instance) live.push({ filename, session });
    } catch { /* A stale descriptor is not a running profile. */ }
  }
  if (live.length > 1) throw new Error('More than one Verenu profile is running. Specify the intended VERENU_APP_DATA_DIR_OVERRIDE before connecting.');
  if (!live.length) throw new Error('No running Verenu agent session found. Open the installed Verenu app and try again.');
  return live[0];
}

export async function connect(env = process.env, { timeoutMs = 120000, pollMs = 250 } = {}) {
  if (env.VERENU_AGENT_TOKEN !== undefined || env.VERENU_AGENT_PORT !== undefined) {
    await invoke('describe', {}, env);
    return;
  }
  const resolved = await resolveSession(env);
  const filename = resolved.filename;
  let session = resolved.session;
  const instance = session.instance;
  if (session.status === 'approved') {
    try { await invoke('describe', {}, { VERENU_AGENT_TOKEN: session.token, VERENU_AGENT_PORT: String(session.port) }); }
    catch { throw new Error('Could not reach the approved Verenu session. Open Verenu and try again.'); }
    return;
  }
  let response;
  try {
    response = await fetch(`http://127.0.0.1:${session.port}/v1/pair`, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(5000),
    });
  } catch {
    throw new Error('Could not reach the running Verenu app. Open Verenu and try again.');
  }
  if (!response.ok) throw new Error(`Verenu could not start an approval request (HTTP ${response.status}).`);
  if (response.status !== 200) console.error('Approve agent access in the Verenu window. Access lasts until Verenu quits.');
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, pollMs));
    const previousPort = session.port;
    session = await readDescriptor(filename);
    if (!session || session.port !== previousPort || session.instance !== instance) throw new Error('Verenu restarted during approval. Try connecting again.');
    if (session.status === 'approved') {
      try { await invoke('describe', {}, { VERENU_AGENT_TOKEN: session.token, VERENU_AGENT_PORT: String(session.port) }); }
      catch { throw new Error('Could not reach the approved Verenu session. Open Verenu and try again.'); }
      return;
    }
    if (session.status === 'denied') throw new Error('Agent access was declined in Verenu.');
  }
  throw new Error('Agent approval timed out. Try connecting again.');
}

export async function invoke(command, args = {}, env = process.env) {
  let token = env.VERENU_AGENT_TOKEN;
  let port = Number(env.VERENU_AGENT_PORT);
  if (token === undefined && env.VERENU_AGENT_PORT === undefined) {
    const { session } = await resolveSession(env);
    if (session.status !== 'approved') throw new Error('Agent access is not approved yet. Run connect and approve the request in Verenu.');
    ({ token, port } = session);
  }
  if (!/^[a-f\d]{64}$/i.test(token ?? '') || !Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error('Invalid explicit agent grant. Set both VERENU_AGENT_TOKEN and VERENU_AGENT_PORT, or remove both and use connect.');
  }
  let response;
  try {
    response = await fetch(`http://127.0.0.1:${port}/v1/invoke`, {
      method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(65000),
      redirect: 'error',
    });
  } catch { throw new Error('Could not reach the Verenu agent session. Open Verenu and try again.'); }
  const result = await response.json();
  if (!response.ok) throw new Error(result.error ?? `HTTP ${response.status}`);
  if (result.version !== 1 || !Object.hasOwn(result, 'result')) throw new Error('Unsupported agent API response');
  return result.result;
}

async function main(argv) {
  const [command, ...args] = argv;
  if (!command || command === '--help') {
    console.log(`Verenu agent CLI (Node 20+). The desktop app must be running.
Usage: node verenu-agent.mjs <command> '[JSON args]'
       node verenu-agent.mjs <command> --stdin
       node verenu-agent.mjs connect
       node verenu-agent.mjs harnesses
       node verenu-agent.mjs install-skill
       node verenu-agent.mjs install-skill --permanent --harness codex,claude [--replace]

Run connect and approve the request in Verenu. Agents running as your user can
reuse that grant until Verenu quits. It allows private reads and settings writes.
Use describe for the live API and supported settings. Responses are JSON.
An explicit VERENU_AGENT_TOKEN and VERENU_AGENT_PORT grant remains supported.
Never print or share a token. The grant and profile belong to the running app.
On Windows, automatic session access is unavailable; for an explicit grant,
launch Verenu from a private PowerShell session after setting:
  $env:VERENU_AGENT_TOKEN = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
  $env:VERENU_AGENT_PORT = '17421'
Do not paste the token into chat or log it. Quit and relaunch without these
variables to revoke the explicit grant.
Installation writes only selected detected harnesses. It does not enable the API or change global instructions.
Text history requires include_text:true on each history.page request. Output may be private.`);
    return;
  }
  if (command === 'connect') { if (args.length) throw new Error('connect takes no arguments'); await connect(); console.log('Verenu agent access approved for this session.'); return; }
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
