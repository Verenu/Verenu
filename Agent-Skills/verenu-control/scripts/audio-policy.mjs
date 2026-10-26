#!/usr/bin/env node
// PipeWire's supported PulseAudio compatibility API supplies the active port.
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { invoke } from './verenu-agent.mjs';

export function muteForRoute({ output, activePort }, { muteOutput, unmuteOutput } = {}) {
  const route = `${activePort} ${output}`.toLowerCase();
  if (unmuteOutput?.trim() && route.includes(unmuteOutput.trim().toLowerCase())) return false;
  if (muteOutput?.trim() && route.includes(muteOutput.trim().toLowerCase())) return true;
  return null;
}

function inventory(kind) {
  const command = (args) => execFileSync('pactl', args, { encoding: 'utf8', timeout: 5000, maxBuffer: 2 * 1024 * 1024 });
  const name = command([`get-default-${kind === 'sources' ? 'source' : 'sink'}`]).trim();
  const device = JSON.parse(command(['--format=json', 'list', kind])).find((row) => row.name === name);
  if (!device) throw new Error(`Default ${kind} route unavailable; leaving mute_audio unchanged`);
  return { name: [device.description, device.properties?.['device.product.name'], device.name].filter(Boolean).join(' '), port: device.active_port ?? '' };
}

async function main() {
  if (process.platform !== 'linux') throw new Error('This example uses Linux PipeWire/PulseAudio pactl; use the native audio inventory on other platforms');
  const args = process.argv.slice(2);
  const policy = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--apply') continue;
    const key = { '--mute-output': 'muteOutput', '--unmute-output': 'unmuteOutput' }[args[i]];
    if (!key || !args[i + 1]?.trim() || args[i + 1].startsWith('--')) throw new Error('Usage: node audio-policy.mjs [--mute-output TEXT] [--unmute-output TEXT] [--apply] (default: dry run; unmute match takes priority)');
    policy[key] = args[++i];
  }
  if (!policy.muteOutput && !policy.unmuteOutput) throw new Error('Specify --mute-output or --unmute-output; no device rules are assumed');
  const output = inventory('sinks');
  const mute = muteForRoute({ output: output.name, activePort: output.port }, policy);
  if (mute === null) { console.log(JSON.stringify({ applied: false, reason: 'No matching route; mute_audio unchanged' })); return; }
  if (process.argv.includes('--apply')) {
    await invoke('save_setting', { key: 'mute_audio', value: mute });
    if (await invoke('get_setting', { key: 'mute_audio' }) !== mute) throw new Error('mute_audio readback mismatch');
  }
  console.log(JSON.stringify({ mute_audio: mute, applied: process.argv.includes('--apply') }));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
