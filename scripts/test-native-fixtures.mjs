#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { run, stopOwned } from './verification/process.mjs';
const args = process.argv.slice(2);
const index = args.indexOf('--report');
const directory = path.join(root, 'test-results', `native-fixture-${randomUUID()}`);
const reportPath = index < 0 ? path.join(directory, 'verification.json') : path.resolve(args[index + 1]);
const report = { schemaVersion: 1, identity: sourceIdentity(), status: 'incomplete', scope: ['focused-text'], platform: process.platform, checks: [], artifacts: [] };
let fixture;
try {
  if (process.platform !== 'linux') { report.reason = 'This automated disposable target currently verifies Linux AT-SPI only'; }
  else if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) { report.reason = 'Linux native fixture needs a desktop display'; }
  else {
    await fs.mkdir(directory, { recursive: true });
    const env = { ...process.env }; delete env.NO_AT_BRIDGE;
    fixture = spawn('python3', ['scripts/test/linux-format-fixture.py'], { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Disposable GTK fixture did not become ready')), 15_000);
      fixture.once('error', (error) => { clearTimeout(timer); reject(error); });
      fixture.once('exit', () => { clearTimeout(timer); reject(new Error('Disposable GTK fixture exited before testing')); });
      fixture.stdout.on('data', (chunk) => { if (String(chunk).includes(`Fixture PID: ${fixture.pid}`)) { clearTimeout(timer); resolve(); } });
    });
    const checked = await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', 'atspi_live_formats_disposable_entry', '--lib', '--', '--ignored'], { directory, name: 'focused-text', env: { ...env, VERENU_FORMAT_FIXTURE_PID: String(fixture.pid) }, timeout: 180_000 });
    report.checks.push({ name: 'AT-SPI cursor formatting in owned GTK entry', status: checked.status });
    report.artifacts.push(artifact(checked.log));
    report.status = checked.status === 'passed' ? 'verified' : 'failed';
  }
} catch (error) { report.status = 'failed'; report.reason = error.message; }
finally {
  if (fixture) stopOwned(fixture);
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Native fixture verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
