#!/usr/bin/env node
import { readFile, readdir } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';

const usage = 'Usage: node scripts/measure-linux-memory.mjs --pid PID [--samples COUNT] [--interval-ms MILLISECONDS]';

function options(args) {
  if (args.length === 1 && args[0] === '--help') return null;
  const values = new Map();
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i];
    const value = args[i + 1];
    if (!['--pid', '--samples', '--interval-ms'].includes(key) || values.has(key) || !/^\d+$/.test(value ?? '')) {
      throw new Error(`Invalid arguments. ${usage}`);
    }
    values.set(key, Number(value));
  }
  const pid = values.get('--pid');
  const samples = values.get('--samples') ?? 12;
  const intervalMs = values.get('--interval-ms') ?? 5000;
  if (!Number.isSafeInteger(pid) || pid < 1 || pid > 2147483647 ||
      !Number.isSafeInteger(samples) || samples < 1 || samples > 100000 ||
      !Number.isSafeInteger(intervalMs) || intervalMs < 1 || intervalMs > 2147483647) {
    throw new Error(`PID, sample count, or interval is out of range. ${usage}`);
  }
  return { pid, samples, intervalMs };
}

async function processIdentity(pid) {
  let stat;
  try {
    stat = await readFile(`/proc/${pid}/stat`, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT' || error.code === 'ESRCH') throw new Error(`Process ${pid} exited during sampling`);
    throw new Error(`Cannot read process ${pid} status (${error.code || 'unknown error'})`);
  }
  const end = stat.lastIndexOf(')');
  const fields = stat.slice(end + 2).trim().split(/\s+/);
  if (end < 0 || fields.length < 20 || !/^\d+$/.test(fields[19])) throw new Error(`Invalid process ${pid} status`);
  if (fields[0] === 'Z' || fields[0] === 'X') throw new Error(`Process ${pid} exited during sampling`);
  return fields[19];
}

async function snapshot(root, rootStart) {
  const pending = [root];
  const seen = new Set();
  const identities = new Map();
  const processes = [];
  while (pending.length) {
    const pid = pending.pop();
    if (seen.has(pid)) continue;
    seen.add(pid);
    const start = await processIdentity(pid);
    identities.set(pid, start);
    if (pid === root && start !== rootStart) throw new Error(`Process ${root} exited or was replaced during sampling`);
    let rollup, tasks;
    try {
      [rollup, tasks] = await Promise.all([
        readFile(`/proc/${pid}/smaps_rollup`, 'utf8'),
        readdir(`/proc/${pid}/task`),
      ]);
    } catch (error) {
      throw new Error(`Cannot read process ${pid} memory or tasks (${error.code || 'unknown error'})`);
    }
    const pss = /^Pss:\s+(\d+) kB$/m.exec(rollup);
    if (!pss) throw new Error(`PSS is unavailable for process ${pid}`);
    processes.push({ pid, pssKiB: Number(pss[1]) });
    for (const tid of tasks) {
      try {
        const children = await readFile(`/proc/${pid}/task/${tid}/children`, 'utf8');
        for (const child of children.trim().split(/\s+/).filter(Boolean)) pending.push(Number(child));
      } catch (error) {
        throw new Error(`Cannot read process ${pid} children (${error.code || 'unknown error'}); the process tree may have changed`);
      }
    }
    if (await processIdentity(pid) !== start) throw new Error(`Process ${pid} exited or was replaced during sampling`);
  }
  for (const [pid, start] of identities) {
    if (await processIdentity(pid) !== start) throw new Error(`Process ${pid} exited or was replaced during sampling`);
  }
  processes.sort((a, b) => a.pid - b.pid);
  return { timestamp: new Date().toISOString(), totalPssKiB: processes.reduce((sum, process) => sum + process.pssKiB, 0), processes };
}

async function main() {
  const config = options(process.argv.slice(2));
  if (!config) { console.log(usage); return; }
  if (process.platform !== 'linux') throw new Error('This command requires Linux /proc');
  const start = await processIdentity(config.pid);
  const samples = [];
  for (let i = 0; i < config.samples; i++) {
    if (i) await delay(config.intervalMs);
    samples.push(await snapshot(config.pid, start));
  }
  const totals = samples.map(sample => sample.totalPssKiB);
  console.log(JSON.stringify({
    rootPid: config.pid,
    intervalMs: config.intervalMs,
    unit: 'KiB',
    samples,
    summary: {
      averagePssKiB: totals.reduce((sum, value) => sum + value, 0) / totals.length,
      minPssKiB: Math.min(...totals),
      maxPssKiB: Math.max(...totals),
    },
  }, null, 2));
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
