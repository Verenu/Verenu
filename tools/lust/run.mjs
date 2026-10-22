#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { performance } from 'node:perf_hooks';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const output = path.join(root, 'test-results/lust');
const toolchain = JSON.parse(fs.readFileSync(path.join(here, 'toolchain.json'), 'utf8'));
const pin = toolchain.revision;
const checkout = path.join(output, `toolchain-${pin}`);
const targets = JSON.parse(fs.readFileSync(path.join(here, 'targets.json'), 'utf8'));
const mode = process.argv[2];
const env = { ...process.env, LUST_NO_DAEMON: '1' };
fs.mkdirSync(output, { recursive: true });
const directory = fs.mkdtempSync(path.join(output, `${mode || 'help'}-`));
env.LUST_CACHE_DIR = path.join(directory, 'cache');
const socketDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-lust-'));
env.LUST_DAEMON_SOCKET = path.join(socketDirectory, 'daemon.sock');
env.LUST_DAEMON_IDLE = '15';
const report = { mode, date: new Date().toISOString(), platform: `${os.platform()} ${os.arch()}`, cpu: os.cpus()[0]?.model,
  worktree: root, load: os.loadavg(), daemon: false, toolchain, compilerOverride: process.env.LUST_BIN || null,
  sources: [], blocked: [], commands: [] };

function execute(command, args, { allowFailure = false, timeout = 120_000, cwd = directory } = {}) {
  const start = performance.now();
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8', timeout, maxBuffer: 32 * 1024 * 1024 });
  const record = { command, args, elapsedMs: performance.now() - start, status: result.status,
    stdout: result.stdout || '', stderr: result.stderr || '', error: result.error?.message };
  report.commands.push(record);
  if (result.error || (result.status !== 0 && !allowFailure)) {
    throw new Error(`${command} ${args.join(' ')} failed (${result.status}):\n${record.error || ''}\n${record.stdout}\n${record.stderr}`);
  }
  return record;
}

function generate(name, selected, tail, inline = false) {
  const modules = selected.map(target => {
    const source = path.join(root, 'src-tauri/src', target.path);
    const bytes = fs.readFileSync(source); // Missing/renamed sources must fail.
    if (!report.sources.some(item => item.path === target.path)) {
      report.sources.push({ path: target.path, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
    // Both test compilers receive the exact source bytes in the same modules.
    return inline ? `mod ${target.name} {\n${bytes.toString('utf8')}\n}`
      : `#[path = ${JSON.stringify(source)}]\nmod ${target.name};`;
  });
  const filename = path.join(directory, name);
  fs.writeFileSync(filename, `#![allow(dead_code)]\n${modules.join('\n')}\n${tail}\n`);
  return filename;
}

function rustTests(text) {
  return [...text.matchAll(/^test (\S+) \.\.\. (ok|FAILED|ignored)/gm)]
    .map(([, name, status]) => ({ name, status: status === 'ok' ? 'pass' : status === 'ignored' ? 'ignored' : 'fail' }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function setup() {
  if (os.platform() !== 'linux' || os.arch() !== 'x64') throw new Error('Lust 0.2.0 supports Linux x86_64 only.');
  console.log(`Preparing ${toolchain.repository} at ${pin}...`);
  env.CARGO_TARGET_DIR = path.join(checkout, 'target');
  // Each revision has its own checkout and build output, including host CRT paths.
  if (!fs.existsSync(checkout)) execute('git', ['init', checkout]);
  if (!fs.existsSync(path.join(checkout, '.git'))) throw new Error(`Refusing to overwrite non-Git directory ${checkout}`);
  if (execute('git', ['status', '--porcelain', '--untracked-files=no'], { cwd: checkout }).stdout.trim()) {
    throw new Error(`Refusing to overwrite modified toolchain sources in ${checkout}`);
  }
  const head = execute('git', ['rev-parse', '--verify', 'HEAD'], { cwd: checkout, allowFailure: true });
  if (head.status === 0 && head.stdout.trim() !== pin) throw new Error(`Toolchain HEAD differs from pinned commit ${pin}`);
  if (head.status !== 0) {
    execute('git', ['fetch', '--depth', '1', toolchain.repository, pin], { cwd: checkout, timeout: 300_000 });
    execute('git', ['checkout', '--detach', 'FETCH_HEAD'], { cwd: checkout });
  }
  if (!fs.existsSync(path.join(checkout, 'third_party/tcc/lib/libtcc.a'))) {
    console.log('Building pinned TinyCC...');
    execute('bash', ['scripts/build-tcc.sh'], { cwd: checkout, timeout: 600_000 });
  }
  console.log('Building Lust with the release profile...');
  execute('cargo', ['build', '--release', '--locked', '-j', '4'], { cwd: checkout, timeout: 1_200_000 });
  console.log(`Built Lust ${pin}. Toolchain: ${checkout}/target/release/lust`);
}

function run() {
  if (mode === 'setup') return setup();
  if (!['check', 'test', 'bench', 'full-check'].includes(mode)) throw new Error('Usage: node tools/lust/run.mjs setup|check|test|bench|full-check');
  const local = path.join(checkout, 'target/release/lust');
  const lust = process.env.LUST_BIN || local;
  if (!process.env.LUST_BIN && !fs.existsSync(local)) {
    throw new Error('Pinned Lust is not built. Run node tools/lust/run.mjs setup or set LUST_BIN.');
  }
  const version = execute(lust, ['--version']).stdout.trim();
  if (version.split(/\s+/).slice(0, 2).join(' ') !== `lust ${toolchain.version}`) {
    throw new Error(`Expected Lust ${toolchain.version}, got ${version}`);
  }
  report.lust = version;
  report.rustc = execute('rustc', ['--version']).stdout.trim();
  if (mode === 'full-check') return fullCheck(lust);
  for (const target of targets) {
    fs.accessSync(path.join(root, 'src-tauri/src', target.path));
    if (target.blockedCheck) report.blocked.push({ module: target.name, capability: 'check/test/bench', reason: target.blockedCheck });
    else if (!target.test) report.blocked.push({ module: target.name, capability: 'test', reason: target.blockedTest });
  }
  const checkRoot = generate('check.rs', targets.filter(t => !t.blockedCheck), 'fn main() {}');
  execute(lust, ['check', checkRoot]);
  execute('rustc', ['--edition=2024', '--emit=metadata', checkRoot, '-o', path.join(directory, 'check.rmeta')]);
  console.log(`Checked ${report.sources.length} production modules with Lust and rustc.`);
  if (mode === 'test') {
    const testRoot = generate('tests.rs', targets.filter(t => t.test), 'fn main() {}', true);
    const bin = path.join(directory, 'rust-tests');
    execute('rustc', ['--edition=2024', '--test', testRoot, '-o', bin]);
    const baseline = rustTests(execute(bin, ['--test-threads=1']).stdout);
    const actual = execute(lust, ['test', testRoot, '--json']).stdout.split('\n').filter(Boolean)
      .map(line => JSON.parse(line)).filter(row => row.kind === 'test')
      .map(({ name, status }) => ({ name, status }))
      .sort((a, b) => a.name.localeCompare(b.name));
    if (!baseline.length || baseline.some(t => t.status !== 'pass') || JSON.stringify(actual) !== JSON.stringify(baseline)) {
      throw new Error(`Test parity failed:\nrustc ${JSON.stringify(baseline)}\nlust ${JSON.stringify(actual)}`);
    }
    report.tests = actual;
    // Exercise both failure paths so a tool invocation cannot masquerade as checking.
    const bad = generate('invalid.rs', [], 'fn main() { verenu_deliberately_missing(); }');
    for (const [compiler, args] of [[lust, ['check', bad]], ['rustc', ['--edition=2024', '--emit=metadata', bad]]]) {
      const failure = execute(compiler, args, { allowFailure: true });
      if (failure.status === 0 || !`${failure.stdout}${failure.stderr}`.includes('verenu_deliberately_missing')) {
        throw new Error(`${compiler} did not reject the known invalid program with the expected diagnostic`);
      }
    }
    console.log(`${actual.length} real Verenu tests passed with identical names and results under Lust and rustc.`);
  }
  if (mode === 'bench') benchmark(lust, checkRoot);
  for (const blocked of report.blocked) console.log(`BLOCKED ${blocked.module} ${blocked.capability}: ${blocked.reason}`);
}

function fullCheck(lust) {
  const backend = path.join(root, 'src-tauri');
  const snapshot = path.join(directory, 'backend-2024');
  fs.mkdirSync(snapshot);
  // Keep the real dependency graph and source files. Only the copied manifest's
  // edition changes; Cargo's build products and lockfile stay in this run.
  for (const entry of fs.readdirSync(backend)) {
    if (['target', '.lust-cache', 'Cargo.toml', 'Cargo.lock'].includes(entry)) continue;
    fs.cpSync(path.join(backend, entry), path.join(snapshot, entry), { recursive: true });
  }
  const manifest = fs.readFileSync(path.join(backend, 'Cargo.toml'), 'utf8');
  if (!/^edition = "2021"$/m.test(manifest)) throw new Error('Review full-check edition overlay: expected edition 2021');
  const original = path.join(directory, 'backend-2021');
  fs.cpSync(snapshot, original, { recursive: true });
  fs.writeFileSync(path.join(original, 'Cargo.toml'), manifest);
  fs.copyFileSync(path.join(backend, 'Cargo.lock'), path.join(original, 'Cargo.lock'));
  fs.writeFileSync(path.join(snapshot, 'Cargo.toml'), manifest.replace(/^edition = "2021"$/m, 'edition = "2024"'));
  fs.copyFileSync(path.join(backend, 'Cargo.lock'), path.join(snapshot, 'Cargo.lock'));
  env.CARGO_TARGET_DIR = path.join(output, 'full-backend-target');
  env.CARGO_BUILD_JOBS = '4';
  report.fullBackend = { manifestSha256: createHash('sha256').update(manifest).digest('hex'), attempts: [], probes: [] };
  for (const [name, source] of [
    ['async', 'async fn provider_request() -> u32 { 1 }\nfn main() { let _request = provider_request(); }'],
    ['native-ffi', 'unsafe extern "C" { fn getpid() -> i32; }\nfn main() { let _pid = unsafe { getpid() }; }'],
  ]) {
    const filename = path.join(directory, `${name}.rs`);
    fs.writeFileSync(filename, source);
    execute('rustc', ['--edition=2024', '--emit=metadata', filename, '-o', `${filename}.rmeta`]);
    const result = execute(lust, ['check', filename, '--json'], { allowFailure: true });
    report.fullBackend.probes.push({ name, status: result.status, stdout: result.stdout, stderr: result.stderr });
    console.log(`${name} language probe (rustc accepted): ${result.stdout}${result.stderr}`);
  }
  for (const [name, manifestPath, selection] of [
    ['original-library', path.join(original, 'Cargo.toml'), ['--lib']],
    ['edition-2024-library', path.join(snapshot, 'Cargo.toml'), ['--lib']],
    ['edition-2024-desktop', path.join(snapshot, 'Cargo.toml'), ['--bin', 'verenu']],
  ]) {
    console.log(`Checking ${name} with the complete Cargo dependency graph...`);
    const source = path.join(path.dirname(manifestPath), 'src', selection[0] === '--lib' ? 'lib.rs' : 'main.rs');
    const result = execute(lust, ['check', source, '--manifest-path', manifestPath, ...selection, '--locked', '--json'],
      { allowFailure: true, timeout: 600_000 });
    report.fullBackend.attempts.push({ name, status: result.status, stdout: result.stdout, stderr: result.stderr });
    console.log(`${name}: ${result.status === 0 ? 'PASS' : 'FAILED'}\n${result.stdout}${result.stderr}`);
  }
  if (report.fullBackend.attempts.some(attempt => attempt.status !== 0)) {
    throw new Error('Full Verenu compilation is blocked. See fullBackend.attempts in the report; the shipping manifest was not changed.');
  }
}

function benchmark(lust, checkRoot) {
  const workload = generate('workload.rs', targets.filter(t => !t.blockedCheck), fs.readFileSync(path.join(here, 'workload.rs'), 'utf8'));
  const lustBin = path.join(directory, 'lust-workload');
  const rustBin = path.join(directory, 'rust-workload');
  // No --release: Lust forwards that mode to rustc. Both are development builds.
  execute(lust, ['build', workload, '-o', lustBin]);
  execute('rustc', ['--edition=2024', '-C', 'opt-level=0', workload, '-o', rustBin]);
  const rounds = '200000';
  const expected = execute(rustBin, [rounds]).stdout;
  if (execute(lustBin, [rounds]).stdout !== expected) throw new Error('Workload checksum mismatch');
  // Edit a private snapshot of an actual production function, never the live
  // worktree. Both compilers read the identical snapshot after each edit.
  const sourcePath = path.join(root, 'src-tauri/src', targets.find(t => t.name === 'device_match').path);
  const source = fs.readFileSync(sourcePath, 'utf8');
  const needle = 'if candidate == desired {';
  if (source.split(needle).length !== 2) throw new Error('Device matching benchmark edit needs review');
  const snapshot = path.join(directory, 'device_match.rs');
  fs.writeFileSync(snapshot, source);
  fs.writeFileSync(checkRoot, fs.readFileSync(checkRoot, 'utf8').replace(JSON.stringify(sourcePath), JSON.stringify(snapshot)));
  env.LUST_NO_DAEMON = '0';
  report.daemon = true;
  const rustCheck = ['--edition=2024', '--emit=metadata', '-C', `incremental=${directory}/incremental`, checkRoot, '-o', path.join(directory, 'check.rmeta')];
  execute(lust, ['check', checkRoot]);
  execute('rustc', rustCheck);
  execute(lust, ['check', checkRoot]);
  report.benchmark = { scope: `${report.sources.length} source modules; warm device matching body-edit check and dev runtime, not the desktop app`,
    edit: 'Add a different always-true integer comparison to the exact-name branch in a private device_match.rs snapshot',
    iterations: Number(rounds), checksum: expected.trim(), rustOptLevel: 0, rustIncremental: true, samples: [] };
  try { report.governor = fs.readFileSync('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor', 'utf8').trim(); } catch { report.governor = 'unavailable'; }
  for (let sample = 0; sample < 5; sample++) {
    fs.writeFileSync(snapshot, source.replace(needle, `if candidate == desired && ${sample}u8 == ${sample}u8 {`));
    const order = sample % 2 ? ['rustc', 'lust'] : ['lust', 'rustc'];
    for (const compiler of order) {
      const check = compiler === 'lust' ? execute(lust, ['check', checkRoot])
        : execute('rustc', rustCheck);
      const runtime = execute(compiler === 'lust' ? lustBin : rustBin, [rounds]);
      if (runtime.stdout !== expected) throw new Error('Timed workload checksum mismatch');
      report.benchmark.samples.push({ sample, compiler, checkMs: check.elapsedMs, runtimeMs: runtime.elapsedMs });
    }
  }
  for (const compiler of ['lust', 'rustc']) {
    const samples = report.benchmark.samples.filter(s => s.compiler === compiler);
    const stats = key => {
      const times = samples.map(s => s[key]).sort((a, b) => a - b);
      return { median: times[2], min: times[0], max: times[4] };
    };
    console.log(`${compiler}: ${JSON.stringify({ checkMs: stats('checkMs'), runtimeMs: stats('runtimeMs') })}`);
  }
  fs.writeFileSync(snapshot, source.replace(needle, 'if candidate == verenu_deliberately_missing {'));
  for (const [compiler, args] of [[lust, ['check', checkRoot]], ['rustc', rustCheck]]) {
    const failure = execute(compiler, args, { allowFailure: true });
    if (failure.status === 0 || !`${failure.stdout}${failure.stderr}`.includes('verenu_deliberately_missing')) {
      throw new Error(`${compiler} did not invalidate a warmed check after an invalid body edit`);
    }
  }
  fs.writeFileSync(snapshot, source);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { run(); report.status = 'passed'; }
  catch (error) { report.status = 'failed'; report.error = error.message; console.error(error.message); process.exitCode = 1; }
  finally {
    // Removing our unique socket asks only this run's daemon to shut down.
    fs.rmSync(env.LUST_DAEMON_SOCKET, { force: true });
    fs.writeFileSync(path.join(directory, 'report.json'), JSON.stringify(report, null, 2)); console.log(`Report: ${directory}/report.json`);
  }
}
