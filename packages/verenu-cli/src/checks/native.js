import { access, lstat, readdir, readFile } from 'node:fs/promises';
import { constants, createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, delimiter, basename } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { stalePidFiles } from '../fixes/index.js';

const result = (name, status, message) => ({ name, status, message });
const exists = p => access(p, constants.X_OK).then(() => true, () => false);
const names = p => readdir(p).catch(() => []);

export async function nativeChecks({ dataDir, settings, env = process.env, platform = process.platform, run = spawnSync, networkAllowed = false }) {
  const paths = (env.PATH || '').split(delimiter).filter(Boolean);
  const executable = env.VERENU_BINARY || (platform === 'win32' ? 'verenu.exe' : 'verenu');
  const binary = executable.includes('/') || executable.includes('\\') ? await exists(executable) : (await Promise.all(paths.map(p => exists(join(p, executable))))).some(Boolean);
  let crash = false, keyringError = false;
  for (const name of (await names(join(dataDir, 'logs'))).slice(0, 20)) {
    try {
      const p = join(dataDir, 'logs', name), info = await lstat(p);
      if (!info.isFile() || info.size > 5_000_000) continue;
      const content = await readFile(p, 'utf8');
      crash ||= /(?:panic(?:ked)? at|fatal startup|frontend watchdog|segmentation fault)/i.test(content);
      keyringError ||= /(?:secret service|keyring|keychain|credential manager).{0,100}(?:unavailable|locked|failed|error)/i.test(content);
    } catch { /* A non-readable log cannot establish an error. */ }
  }
  const launch = result('launch', !binary ? 'warn' : crash ? 'fail' : 'pass', !binary ? 'Executable not found on PATH. Set VERENU_BINARY to the installed binary.' : crash ? 'Executable found; logs contain a crash marker. Inspect local logs.' : 'Executable is present and executable; no crash markers found in inspected logs.');
  let keyring;
  if (platform !== 'linux') keyring = result('keyring', 'warn', 'Native secure-storage probe is not supported on this OS.');
  else {
    const probe = run('secret-tool', ['search', '--all', 'service', 'com.verenu.app', 'username', '__verenu_triage_probe__'], { stdio: ['ignore', 'ignore', 'ignore'], timeout: 3000 });
    const ping = run('dbus-send', ['--session', '--print-reply', '--dest=org.freedesktop.secrets', '/org/freedesktop/secrets', 'org.freedesktop.DBus.Peer.Ping'], { stdio: 'ignore', timeout: 3000 });
    const available = !probe.error && !ping.error && ping.status === 0;
    keyring = result('keyring', keyringError ? 'fail' : available ? 'pass' : 'warn', keyringError ? 'Logs show a secure-storage error. Unlock the keyring and retry.' : available ? 'Secret Service probe responded.' : 'Secret Service probe unavailable. Check secret-tool and unlock the keyring.');
  }
  const providers = [...new Set([settings?.transcription_provider, settings?.cleanup_provider].filter(p => typeof p === 'string' && !['local', 'apple_intelligence', ''].includes(p)))];
  let missing = false, unknown = false;
  for (const provider of providers) {
    if (!/^(groq|openai|google|assemblyai|openrouter|xai)$/.test(provider)) { unknown = true; continue; }
    if (typeof settings[`api_key_${provider}`] === 'string' && settings[`api_key_${provider}`].trim()) continue;
    if (platform !== 'linux' || keyring.status !== 'pass') { unknown = true; continue; }
    const probe = run('secret-tool', ['lookup', 'service', 'com.verenu.app', 'username', `api_key_${provider}`], { stdio: ['ignore', 'ignore', 'ignore'], timeout: 3000 });
    if (probe.error || ![0, 1].includes(probe.status)) unknown = true;
    else if (probe.status !== 0) missing = true;
  }
  const keys = result('provider-keys', missing ? 'fail' : unknown || !settings ? 'warn' : 'pass', missing ? 'A configured provider has no saved key. Add it in Settings > Providers.' : unknown || !settings ? 'Some provider credentials could not be checked.' : 'Configured provider credentials are present or not required.');
  const builtIn = JSON.parse(await readFile(fileURLToPath(new URL('../model-manifest.json', import.meta.url)), 'utf8'));
  const expectedHashes = new Map(builtIn.files.map(file => [file.name, file.sha256]));
  for (const model of builtIn.speech || []) if (model.archive && model.sha256) expectedHashes.set(model.archive, model.sha256);
  let partial = false, inspected = 0, verified = 0, missingMarker = false;
  async function walk(dir, depth) {
    if (depth > 8) return;
    for (const entry of await names(dir)) {
      const p = join(dir, entry);
      try {
        const info = await lstat(p);
        if (info.isSymbolicLink()) continue;
        if (/\.(partial|tmp)$/i.test(entry) || (info.isFile() && info.size === 0)) partial = true;
        if (info.isFile()) inspected++;
        if (info.isFile() && expectedHashes.has(entry)) {
          const hash = createHash('sha256');
          for await (const chunk of createReadStream(p)) hash.update(chunk);
          if (hash.digest('hex') !== expectedHashes.get(entry)) partial = true;
          else verified++;
        }
        if (info.isFile() && entry === 'manifest.json' && info.size < 1_000_000) {
          const manifest = JSON.parse(await readFile(p, 'utf8'));
          if (Array.isArray(manifest.files)) for (const file of manifest.files) {
            if (typeof file.name !== 'string' || basename(file.name) !== file.name || !/^[a-f0-9]{64}$/i.test(file.sha256)) { partial = true; continue; }
            const artifact = join(dir, file.name), artifactInfo = await lstat(artifact);
            if (!artifactInfo.isFile() || artifactInfo.isSymbolicLink() || (Number.isSafeInteger(file.size) && artifactInfo.size !== file.size)) { partial = true; continue; }
            const hash = createHash('sha256');
            for await (const chunk of createReadStream(artifact)) hash.update(chunk);
            if (hash.digest('hex') !== file.sha256.toLowerCase()) partial = true;
            else verified++;
          }
        }
        if (info.isDirectory()) await walk(p, depth + 1);
      } catch { partial = true; }
    }
  }
  await walk(join(dataDir, 'models'), 0);
  for (const model of builtIn.speech || []) {
    if (!model.directory) continue;
    const root = join(dataDir, 'models', 'stt', model.install);
    try {
      const info = await lstat(root);
      if (!info.isDirectory() || info.isSymbolicLink()) { partial = true; continue; }
      try { if (await readFile(join(root, '.verenu-extracted'), 'utf8') !== model.id) missingMarker = true; } catch { missingMarker = true; }
      const modelFiles = name => [`${name}.int8.onnx`, `${name}.onnx`];
      const streaming = name => [`${name}.int8.ort`, `${name}.ort`, `${name}.int8.onnx`, `${name}.onnx`];
      const required = {
        Parakeet: [modelFiles('encoder-model'), modelFiles('decoder_joint-model'), ['nemo128.onnx'], ['vocab.txt']],
        Moonshine: [modelFiles('encoder_model'), modelFiles('decoder_model_merged'), ['tokenizer.json']],
        MoonshineStreaming: [['streaming_config.json'], ['tokenizer.bin'], ...['frontend', 'encoder', 'adapter', 'cross_kv', 'decoder_kv'].map(streaming)],
        SenseVoice: [modelFiles('model'), ['tokens.txt']],
        GigaAm: [modelFiles('model'), ['vocab.txt']],
        Canary: [modelFiles('encoder-model'), modelFiles('decoder-model'), ['nemo128.onnx'], ['vocab.txt']],
        Cohere: [['cohere-encoder.int8.onnx', 'encoder_model.int8.onnx', 'onnx/cohere-encoder.int8.onnx', 'onnx/encoder_model.int8.onnx'], ['cohere-decoder.int8.onnx', 'decoder_model_merged.int8.onnx', 'onnx/cohere-decoder.int8.onnx', 'onnx/decoder_model_merged.int8.onnx'], ['tokens.txt', 'vocabulary.txt', 'onnx/tokens.txt', 'onnx/vocabulary.txt']],
      }[model.engine];
      if (!required) { missingMarker = true; continue; }
      for (const alternatives of required) {
        let found = false;
        for (const name of alternatives) { try { const entry = await lstat(join(root, name)); if (entry.isFile() && !entry.isSymbolicLink() && entry.size > 0) found = true; } catch {} }
        if (!found) partial = true;
      }
    } catch (error) { if (error.code !== 'ENOENT') partial = true; }
  }
  for (const bundle of builtIn.bundles || []) {
    const root = join(dataDir, 'models', 'stt', bundle.id);
    let info;
    try { info = await lstat(root); } catch (error) { if (error.code === 'ENOENT') continue; partial = true; continue; }
    if (!info.isDirectory() || info.isSymbolicLink()) { partial = true; continue; }
    try {
      if (await readFile(join(root, '.verenu-integrity'), 'utf8') !== bundle.revision) partial = true;
      for (const file of bundle.files) {
        const path = join(root, file.path);
        const artifactInfo = await lstat(path);
        if (!artifactInfo.isFile() || artifactInfo.isSymbolicLink() || artifactInfo.size !== file.size) { partial = true; continue; }
        const hash = createHash('sha256');
        for await (const chunk of createReadStream(path)) hash.update(chunk);
        if (hash.digest('hex') !== file.sha256) partial = true;
        else verified++;
      }
    } catch { partial = true; }
  }
  const models = result('models', partial ? 'fail' : inspected || missingMarker ? 'warn' : 'pass', partial ? 'Partial, empty, or invalid model artifact found. Retry the download in Settings.' : missingMarker ? 'An extracted model has no matching marker; older installations may predate markers.' : inspected ? `${verified} manifest artifacts verified; other installed files may not be verified.` : 'No local model files found.');
  if (partial) models.fix = 'clear-partial-models';
  let port;
  try { port = JSON.parse(await readFile(join(dataDir, 'agent-session.json'), 'utf8')).port; } catch {}
  let processCheck = result('process', 'pass', platform === 'win32' ? 'Single-instance mutex is OS-managed.' : 'No persisted agent port found.');
  if (Number.isInteger(port) && port > 0 && port < 65536) {
    if (!['linux', 'darwin'].includes(platform)) processCheck = result('process', 'warn', 'Port ownership probe unsupported on this OS.');
    else {
      const probe = run('lsof', ['-nP', '-t', `-iTCP:${port}`, '-sTCP:LISTEN'], {encoding: 'utf8', timeout: 3000});
      const pids = (probe.stdout || '').trim().split(/\s+/).filter(s => /^\d+$/.test(s));
      processCheck = probe.error || ![0, 1].includes(probe.status) || (probe.stderr || '').trim() ? result('process', 'warn', 'Could not establish agent port ownership.') : pids.length ? result('process', 'pass', `Agent port is listening; owner PID: ${pids.join(', ')}.`) : result('process', 'warn', 'Agent port is not listening. Restart Verenu to refresh it.');
    }
  }
  if (platform !== 'win32' && (await stalePidFiles(dataDir)).length) processCheck = { name: 'process', status: 'warn', message: 'A PID file names a process that no longer exists. Clear it after closing Verenu.', fix: 'clear-stale-pids' };
  return [launch, keyring, keys, models, processCheck];
}
