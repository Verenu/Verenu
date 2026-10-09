#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { root, sourceIdentity, artifact } from './verification/identity.mjs';
import { run } from './verification/process.mjs';
import { formattingResult, stopFixture } from './verification/native-fixture.mjs';
const args = process.argv.slice(2);
const index = args.indexOf('--report');
const directory = path.join(root, 'test-results', `native-fixture-${randomUUID()}`);
const reportArgument = index >= 0 ? args[index + 1] : undefined;
if (index >= 0 && (!reportArgument || reportArgument.startsWith('--'))) {
  throw new Error('--report requires a file path');
}
const reportPath = index < 0 ? path.join(directory, 'verification.json') : path.resolve(reportArgument);
const report = { schemaVersion: 1, identity: sourceIdentity(), status: 'incomplete', scope: ['focused-text'], platform: process.platform, checks: [], artifacts: [] };
let fixture;
try {
  if (process.platform !== 'linux') { report.reason = 'This automated disposable target currently verifies Linux AT-SPI only'; }
  else if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) { report.reason = 'Linux native fixture needs a desktop display'; }
  else {
    await fs.mkdir(directory, { recursive: true });
    const env = { ...process.env }; delete env.NO_AT_BRIDGE;
    const build = await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', 'atspi_live_formats_disposable_entry', '--lib', '--no-run'], { directory, name: 'focused-text-build', env, timeout: 180_000 });
    report.artifacts.push(artifact(build.log));
    if (build.status !== 'passed') throw new Error('Focused native fixture build failed');
    fixture = spawn('python3', ['scripts/test/linux-format-fixture.py'], { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    await new Promise((resolve, reject) => {
      fixture.once('error', reject);
      fixture.once('spawn', resolve);
    });
    fixture.stdout.resume();
    fixture.stderr.resume();
    // Rust discovers the expected PID, window and focused entry before editing.
    const checked = await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', 'atspi_live_formats_disposable_entry', '--lib', '--', '--ignored', '--nocapture'], { directory, name: 'focused-text', env: { ...env, VERENU_FORMAT_FIXTURE_PID: String(fixture.pid) }, timeout: 30_000 });
    const result = formattingResult(checked, await fs.readFile(checked.log, 'utf8'));
    report.checks.push({ name: 'AT-SPI cursor formatting in owned GTK entry', status: result.status === 'verified' ? 'passed' : result.status, cases: result.cases });
    report.artifacts.push(artifact(checked.log));
    report.status = result.status;
    report.reason = result.reason;
  }
  if (sourceIdentity().fingerprint !== report.identity.fingerprint && report.status !== 'failed') {
    report.status = 'incomplete'; report.reason = 'Source changed during native fixture verification';
  }
} catch (error) { report.status = error.code === 'ENOENT' ? 'incomplete' : 'failed'; report.reason = error.code === 'ENOENT' ? 'Disposable GTK fixture requires python3' : error.message; }
finally {
  try { if (fixture) await stopFixture(fixture); }
  catch (error) { report.status = 'failed'; report.reason = error.message; }
  await fs.mkdir(path.dirname(reportPath), { recursive: true });
  await fs.writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
}
console.log(`Native fixture verification: ${report.status}. Report: ${reportPath}`);
process.exitCode = report.status === 'verified' ? 0 : report.status === 'failed' ? 1 : 2;
