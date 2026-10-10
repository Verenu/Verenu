import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync, spawnSync } from 'node:child_process';
import YAML from 'yaml';
import { root } from '../../scripts/verification/identity.mjs';
import { requiredCiRulesetPayload } from '../../scripts/ci/ruleset-payload.mjs';

const workflow = name => YAML.parse(fs.readFileSync(path.join(root, '.github/workflows', name), 'utf8'));

test('required CI aggregate cannot pass when a job fails, skips or is cancelled', () => {
  const gate = workflow('pr-checks.yml').jobs.required;
  assert.equal(gate.if, 'always()');
  for (const job of ['frontend', 'dependency-audits', 'rust', 'all-in-one-fast', 'workflow-tests', 'android', 'native']) assert.ok(gate.needs.includes(job));
  const step = gate.steps[0];
  // Execute the gate's actual command against adverse job results.
  const source = step.run.match(/node -e '([\s\S]+)'/)?.[1];
  assert.ok(source);
  const evaluate = new Function('process', 'console', source);
  for (const result of ['failure', 'skipped', 'cancelled']) {
    let code;
    evaluate({ env: { RESULTS: JSON.stringify({ test: { result } }) }, exit: value => { code = value; } }, { error() {} });
    assert.equal(code, 1);
  }
});

test('nightly publication requires exact-source regression verification', () => {
  const nightly = workflow('morning-release.yml');
  assert.ok(nightly.jobs.publish.needs.includes('verify'));
  assert.equal(nightly.jobs.verify.with['source-ref'], '${{ needs.prepare.outputs.release_sha }}');
  const release = workflow('release-quality.yml');
  for (const job of Object.values(release.jobs)) {
    assert.equal(job.steps.find(step => step.uses?.startsWith('actions/checkout')).with.ref, '${{ inputs.source-ref }}');
  }
});

test('nightly version snapshot preserves CRLF files and still rejects trailing spaces', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-nightly-crlf-'));
  const checkout = path.join(directory, 'checkout');
  const remote = path.join(directory, 'remote.git');
  const git = (...args) => execFileSync('git', args, { cwd: checkout, encoding: 'utf8', stdio: 'pipe' }).trim();
  try {
    fs.mkdirSync(checkout);
    execFileSync('git', ['init', '--bare', remote], { stdio: 'pipe' });
    git('init');
    git('config', 'user.name', 'Nightly Test');
    git('config', 'user.email', 'nightly-test@example.invalid');
    git('config', 'core.autocrlf', 'false');
    git('remote', 'add', 'origin', remote);
    fs.mkdirSync(path.join(checkout, 'src-tauri'));
    const originals = {
      'package.json': '{\r\n  "version": "0.20.0"\r\n}\r\n',
      'src-tauri/tauri.conf.json': '{\r\n  "version": "0.20.0"\r\n}\r\n',
      'src-tauri/Cargo.toml': '[package]\nname = "verenu"\nversion = "0.20.0"\n',
    };
    for (const [file, contents] of Object.entries(originals)) fs.writeFileSync(path.join(checkout, file), contents);
    git('add', 'package.json', 'src-tauri');
    git('commit', '-m', 'Synthetic master snapshot');
    const masterSha = git('rev-parse', 'HEAD');
    const version = '0.20.0-nightly.20261009';
    const tag = `Verenu-${version}`;
    const step = workflow('morning-release.yml').jobs.prepare.steps.find(step => step.id === 'tag');
    const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', step.run], {
      cwd: checkout,
      encoding: 'utf8',
      timeout: 30_000,
      env: { ...process.env, MASTER_SHA: masterSha, APP_VERSION: version, RELEASE_VERSION: version, RELEASE_TAG: tag, GITHUB_OUTPUT: path.join(directory, 'output') },
    });
    assert.equal(result.status, 0, result.stderr);
    const releaseSha = git('rev-parse', `${tag}^{commit}`);
    assert.equal(git('rev-parse', `${releaseSha}^`), masterSha);
    assert.equal(JSON.parse(git('show', `${tag}:package.json`)).version, version);
    const config = JSON.parse(git('show', `${tag}:src-tauri/tauri.conf.json`));
    assert.equal(config.version, version);
    assert.equal(config.bundle.windows.wix.version, '0.20.0.0');
    assert.match(git('show', `${tag}:src-tauri/Cargo.toml`), /version = "0.20.0-nightly.20261009"/);
    assert.match(git('ls-remote', 'origin', `refs/tags/${tag}^{}`), new RegExp(releaseSha));
    assert.match(fs.readFileSync(path.join(directory, 'output'), 'utf8'), new RegExp(`release_sha=${releaseSha}`));
    for (const [file, contents] of Object.entries(originals)) assert.equal(fs.readFileSync(path.join(checkout, file), 'utf8'), contents);

    // Exercise the exact workflow check again with a real trailing-space defect.
    const check = step.run.split('\n').find(line => line.includes('diff --check')).trim();
    fs.writeFileSync(path.join(checkout, 'package.json'), originals['package.json'].replace('0.20.0"', '0.20.0" '));
    const invalid = spawnSync('bash', ['-c', check], { cwd: checkout, encoding: 'utf8', timeout: 10_000 });
    assert.notEqual(invalid.status, 0);
    assert.match(invalid.stdout + invalid.stderr, /trailing whitespace/);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test('native and live workflows publish only safe reports and fail on missing live credentials', () => {
  const extended = workflow('extended-test-profiles.yml');
  const absent = extended.jobs['live-profile'].steps.find(step => step.name === 'Skip live profile when secrets are absent');
  assert.match(absent.run, /status:"incomplete"/);
  assert.match(absent.run, /exit 2/);
  for (const job of Object.values(extended.jobs)) {
    for (const step of job.steps.filter(step => step.uses?.startsWith('actions/upload-artifact'))) {
      assert.equal(step.with.path, 'tests/reports/');
      assert.equal(step.with['if-no-files-found'], 'error');
    }
  }
});

test('PR checks compile and run Android native regressions and native WebViews', () => {
  const jobs = workflow('pr-checks.yml').jobs;
  assert.ok(jobs.android.steps.some(step => step.run === 'npm run test:android'));
  assert.equal(jobs.android.steps.find(step => step.uses === 'android-actions/setup-android@v3').with.packages, 'platform-tools');
  assert.equal(jobs.android.steps.find(step => step.with?.name === 'android-unit-reports').with['if-no-files-found'], 'ignore');
  assert.deepEqual(jobs.native.strategy.matrix.os, ['windows-latest', 'macos-latest', 'ubuntu-latest']);
  assert.match(jobs.native.steps.find(step => step.name === 'Run Linux native WebView').run, /XDG_RUNTIME_DIR/);
  const shared = YAML.parse(fs.readFileSync(path.join(root, '.github/actions/regression-checks/action.yml'), 'utf8'));
  assert.match(shared.runs.steps.find(step => step.run?.includes('test:session:owned')).run, /XDG_RUNTIME_DIR/);
  assert.ok(shared.runs.steps.some(step => step.run?.includes('cargo clippy')));
});

test('master required-check rules preserve other rules and add no bypass actors', () => {
  const rules = fs.readFileSync(path.join(root, 'scripts/ci/merge-rules.mjs'), 'utf8');
  assert.match(rules, /'CI required'/);
  const existingRule = { type: 'required_linear_history' };
  const payload = requiredCiRulesetPayload({ rules: [existingRule, { type: 'required_status_checks' }], bypass_actors: [{ actor_type: 'OrganizationAdmin' }] }, ['CI required']);
  assert.deepEqual(payload.bypass_actors, []);
  assert.deepEqual(payload.rules, [existingRule, {
    type: 'required_status_checks',
    parameters: {
      strict_required_status_checks_policy: true,
      required_status_checks: [{ context: 'CI required', integration_id: 15368 }],
    },
  }]);
});
