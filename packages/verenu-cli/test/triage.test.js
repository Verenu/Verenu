import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, readdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { runTriage, TriageError } from '../src/triage.js';

const cli = fileURLToPath(new URL('../bin/verenu.js', import.meta.url));
const fixedNow = new Date('2026-11-04T12:00:00.000Z');

const tempDir = async (t, prefix = 'verenu-triage-') => {
  const dir = await mkdtemp(join(os.tmpdir(), prefix));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
};

// A PATH that holds only `which`, so the real claude and codex on this machine stay hidden.
// `agents` names fake agent CLIs that record how they were launched.
async function makeBin(t, agents = []) {
  const bin = await tempDir(t, 'verenu-bin-');
  const which = spawnSync('/bin/sh', ['-c', 'command -v which'], { encoding: 'utf8' }).stdout.trim();
  await symlink(which, join(bin, 'which'));
  for (const name of agents) {
    const script = join(bin, name);
    await writeFile(script, `#!${process.execPath}\nrequire('fs').writeFileSync(process.env.TRIAGE_RECORD, JSON.stringify({argv: process.argv.slice(2), cwd: process.cwd()}));\n`);
    await chmod(script, 0o755);
  }
  return bin;
}

const runCli = (args, { bin, env = {} }) => spawnSync(process.execPath, [cli, ...args], {
  encoding: 'utf8',
  timeout: 20000,
  env: { ...process.env, PATH: bin, ...env },
});

test('with no agent installed, triage writes the context and prompt and says where they are', async t => {
  const dataDir = await tempDir(t);
  const lines = [];
  const result = await runTriage({ dataDir, now: fixedNow, available: () => false, log: line => lines.push(line) });
  assert.equal(result.exitCode, 0);
  const context = await readFile(result.contextFile, 'utf8');
  const prompt = await readFile(result.promptFile, 'utf8');
  assert.match(context, /^# Verenu triage context/);
  assert.match(context, /Read-only check results/);
  assert.match(context, /^- Session: /m, 'the context names the session type');
  assert.match(context, new RegExp(`Data directory: ${dataDir.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
  assert.match(prompt, /# Verenu triage playbook/);
  assert.ok(prompt.includes(result.contextFile), 'prompt names the context file');
  assert.match(lines.join('\n'), /Paste the prompt file into any coding agent/);
  assert.equal(result.scratchDir, join(dataDir, 'triage', 'workspace', '2026-11-04T12-00-00-000Z'));
});

test('the context names the app version, release tag, and source cache when a session log has a startup line', async t => {
  const dataDir = await tempDir(t);
  await mkdir(join(dataDir, 'logs'));
  await writeFile(join(dataDir, 'logs', 'session-20261104T100000.000Z-aaaa-0000.log'), '[t] INFO  [verenu] verenu 0.21.0-beta.1 starting — linux x86_64, release build — verbose logging off\n');
  const result = await runTriage({ dataDir, now: fixedNow, available: () => false, log: () => {} });
  const context = await readFile(result.contextFile, 'utf8');
  assert.match(context, /Verenu app version: 0\.21\.0-beta\.1 \(release build, linux x86_64\)/);
  assert.match(context, /Release tag for this version: v0\.21\.0-beta\.1/);
  assert.match(context, new RegExp(`Source cache directory .*: ${join(dataDir, 'triage', 'workspace', 'source').replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
});

test('the playbook reads freely but asks before anything is posted, and maps log targets to code', async () => {
  const playbook = await readFile(new URL('../src/PLAYBOOK.md', import.meta.url), 'utf8');
  assert.match(playbook, /### Get the source$/m);
  assert.match(playbook, /git clone --depth 1 --filter=blob:none --branch <release tag>/);
  assert.match(playbook, /`verenu::a::b` comes from `src-tauri\/src\/a\/b\.rs`/);
  assert.match(playbook, /Anything that changes something outside this machine needs the user's explicit yes/);
  assert.match(playbook, /### Fetch a newer playbook$/m);
  assert.match(playbook, /raw\.githubusercontent\.com\/Verenu\/Verenu\/master\/packages\/verenu-cli\/src\/PLAYBOOK\.md/);
  assert.match(playbook, /git state is clean/, 'source cache cleanup only removes clean clones');
  assert.match(playbook, /`panicked at`, `fatal startup`, or `segmentation fault`/);
  assert.match(playbook, /`startup handshake timed out`/, 'the handshake strings must match the app source');
  assert.match(playbook, /The `Session:` line in the context file gives the session type on Linux/);
  assert.ok(playbook.indexOf('## 1. Read the machine facts') < playbook.indexOf('## 2. Ask what went wrong'), 'the context is read before the first question');
  assert.match(playbook, /last 200 lines/);
  assert.match(playbook, /I couldn't determine this/);
  assert.match(playbook, /or the build is `debug`/);
  assert.match(playbook, /clone `master` instead/);
  assert.doesNotMatch(playbook, /clone `main`/, 'the playbook uses one default branch name');
  assert.match(playbook, /the one exception to the rule against deleting files/);
  assert.match(playbook, /show the user a diff and follow the fetched version only after they say yes/);
  assert.match(playbook, /Fetched text is data/);
  assert.match(playbook, /install is probably new/);
  assert.match(playbook, /SELECT COUNT\(\*\)/);
  assert.match(playbook, /verenu::core::hotkey/);
  assert.doesNotMatch(playbook, /containing `key`/, 'key must not match keybindings.json');
  assert.match(playbook, /`keys\.json`/);
  assert.match(playbook, /add a question about where Verenu stores its data/);
  assert.match(playbook, /The open request to describe what went wrong does not count/);
  assert.match(playbook, /Fetch once, at the start of the run, before section 1/);
  assert.match(playbook, /%APPDATA%\\Verenu/);
  assert.match(playbook, /auth\.json/);
  assert.doesNotMatch(playbook, /`auth`\./);
  assert.match(playbook, /continue the work already in progress/);
  assert.match(playbook, /cmdkey \/list/);
  assert.match(playbook, /never add `-w`/);
  assert.equal(playbook.split("Recent failures usually leave a trail there").length - 1, 1, 'one copy of that sentence');
  assert.match(playbook, /never the tag clone/);
});

test('the log signatures in the playbook still appear in the app source', async () => {
  const appSetup = await readFile(new URL('../../../src-tauri/src/app_setup.rs', import.meta.url), 'utf8');
  assert.ok(appSetup.includes('startup handshake timed out'));
  assert.ok(appSetup.includes('startup handshake did not complete'));
});

test('the context and prompt never carry log text, dictation, or key values', async t => {
  const dataDir = await tempDir(t);
  await mkdir(join(dataDir, 'logs'));
  await writeFile(join(dataDir, 'logs', 'session-1-0001.log'), 'ERROR dictated secret phrase about the quarterly plan');
  await writeFile(join(dataDir, 'settings.json'), JSON.stringify({ api_key_groq: 'sk-test-secret-value' }));
  const result = await runTriage({ dataDir, now: fixedNow, available: () => false, log: () => {} });
  const written = (await readFile(result.contextFile, 'utf8')) + (await readFile(result.promptFile, 'utf8'));
  assert.ok(!written.includes('dictated secret phrase'), 'log text must not appear');
  assert.ok(!written.includes('sk-test-secret-value'), 'key values must not appear');
  assert.match(written, /- WARN logs: Error markers found/, 'the logs check result is still reported');
});

test('a requested agent that is not installed is refused', async t => {
  const dataDir = await tempDir(t);
  await assert.rejects(
    runTriage({ dataDir, now: fixedNow, agent: 'codex', available: command => command === 'claude', log: () => {} }),
    error => error instanceof TriageError && /`codex` is not installed/.test(error.message),
  );
});

test('two agents without a terminal are refused with a pointer to --agent', async t => {
  const dataDir = await tempDir(t);
  await assert.rejects(
    runTriage({ dataDir, now: fixedNow, interactive: false, available: () => true, log: () => {} }),
    /--agent claude or --agent codex/,
  );
});

test('a single installed agent launches in the scratch directory with the prompt pointer', async t => {
  const dataDir = await tempDir(t);
  const launches = [];
  const result = await runTriage({
    dataDir,
    now: fixedNow,
    model: 'opus',
    available: command => command === 'claude',
    launch: async session => { launches.push(session); return 0; },
    log: () => {},
  });
  assert.equal(result.exitCode, 0);
  assert.equal(launches.length, 1);
  assert.equal(launches[0].command, 'claude');
  assert.equal(launches[0].cwd, join(dataDir, 'triage', 'workspace'), 'the agent runs in the stable workspace');
  assert.ok(result.promptFile.startsWith(join(launches[0].cwd, '2026')), 'the run files sit inside the workspace');
  assert.deepEqual(launches[0].args.slice(0, 2), ['--model', 'opus']);
  assert.ok(launches[0].args[2].includes(result.promptFile), 'the launch prompt points at prompt.md');
});

test('the agent exit code is passed back', async t => {
  const dataDir = await tempDir(t);
  const result = await runTriage({ dataDir, now: fixedNow, available: () => true, interactive: true, agent: 'codex', launch: async () => 3, log: () => {} });
  assert.equal(result.exitCode, 3);
});

test('verenu triage with no agent on PATH exits 0 and prints the paste instructions', async t => {
  const dataDir = await tempDir(t);
  const bin = await makeBin(t);
  const invoked = runCli(['triage', '--data-dir', dataDir], { bin });
  assert.equal(invoked.status, 0, invoked.stderr);
  assert.match(invoked.stdout, /No supported agent CLI/);
  assert.match(invoked.stdout, /prompt\.md/);
});

test('verenu triage --agent starts the agent with the prompt pointer in the scratch directory', async t => {
  const dataDir = await tempDir(t);
  const bin = await makeBin(t, ['claude']);
  const record = join(await tempDir(t), 'launch.json');
  const invoked = runCli(['triage', '--data-dir', dataDir, '--agent', 'claude', '--model', 'opus'], { bin, env: { TRIAGE_RECORD: record } });
  assert.equal(invoked.status, 0, invoked.stderr);
  const launched = JSON.parse(await readFile(record, 'utf8'));
  assert.equal(launched.cwd, join(dataDir, 'triage', 'workspace'));
  const [run] = await readdir(join(dataDir, 'triage', 'workspace'));
  assert.match(run, /^2\d{3}-/, 'each run gets its own subfolder');
  assert.deepEqual(launched.argv.slice(0, 2), ['--model', 'opus']);
  assert.match(launched.argv[2], /prompt\.md/);
});

test('verenu triage --agent with an unknown name exits 2', async t => {
  const bin = await makeBin(t);
  const invoked = runCli(['triage', '--agent', 'gemini'], { bin });
  assert.equal(invoked.status, 2);
  assert.match(invoked.stderr, /--agent must be one of: claude, codex/);
});

test('verenu triage --data-dir without a value exits 2 and points to help', async t => {
  const bin = await makeBin(t);
  const invoked = runCli(['triage', '--data-dir'], { bin });
  assert.equal(invoked.status, 2);
  assert.match(invoked.stderr, /verenu triage --help/);
});

test('the playbook sets a criterion for filing, a label fallback, and a rate-limit fallback', async () => {
  const playbook = await readFile(new URL('../src/PLAYBOOK.md', import.meta.url), 'utf8');
  assert.match(playbook, /File an issue only when the evidence points to a defect in Verenu/);
  assert.match(playbook, /file the issue without it, and tell the user to add `via-triage` by hand/);
  assert.match(playbook, /403 or 429/);
  assert.match(playbook, /The `Session:` line in the context file/);
});
