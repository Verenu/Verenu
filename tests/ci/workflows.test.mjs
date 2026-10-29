import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync, spawnSync } from 'node:child_process';
import YAML from 'yaml';
import { root } from '../../scripts/verification/identity.mjs';
import { requiredCiContextDiff, requiredCiRulesetPayload } from '../../scripts/ci/ruleset-payload.mjs';
import { sessionPartition, partitionFiles } from '../../scripts/verification/session-partition.mjs';

const workflow = name => YAML.parse(fs.readFileSync(path.join(root, '.github/workflows', name), 'utf8'));

test('master builds seed caches accessible to new PRs without running on task-branch pushes', () => {
  const checks = workflow('pr-checks.yml');
  assert.deepEqual(checks.on.push.branches, ['master']);
  assert.deepEqual(checks.on.pull_request.branches, ['master']);
  const guard = checks.jobs['blob-size-guard'].steps.find(step => step.env?.PR_BASE_SHA);
  assert.equal(guard.env.PR_BASE_SHA, '${{ github.event.pull_request.base.sha || github.event.before }}');
});

test('Linux setup replaces only the slow Azure mirror and installs explicit fixture tools', () => {
  const shared = YAML.parse(fs.readFileSync(path.join(root, '.github/actions/regression-checks/action.yml'), 'utf8'));
  const setup = shared.runs.steps.find(step => step.name === 'Install Linux build and fixture tools').run;
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-apt-'));
  try {
    const source = path.join(directory, 'ubuntu.sources');
    const calls = path.join(directory, 'calls');
    fs.writeFileSync(source, 'URIs: mirror+file:/etc/apt/apt-mirrors.txt\nURIs: http://azure.archive.ubuntu.com/ubuntu\nURIs: http://security.ubuntu.com/ubuntu\nSuites: noble noble-updates\n');
    const result = spawnSync('bash', ['-euo', 'pipefail', '-c', `
      sudo() {
        if [ "$1" = sed ]; then "$@"; else printf '%s\\n' "$*" >> "$APT_CALLS"; fi
      }
      ${setup.replace('/etc/apt/sources.list.d/ubuntu.sources', '"$APT_SOURCE_FIXTURE"')}
    `], { encoding: 'utf8', env: { ...process.env, APT_SOURCE_FIXTURE: source, APT_CALLS: calls } });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(fs.readFileSync(source, 'utf8'), 'URIs: https://archive.ubuntu.com/ubuntu\nURIs: https://archive.ubuntu.com/ubuntu\nURIs: http://security.ubuntu.com/ubuntu\nSuites: noble noble-updates\n');
    const [update, install] = fs.readFileSync(calls, 'utf8').trim().split('\n');
    assert.equal(update, 'apt-get update');
    assert.ok(install.includes('--no-install-recommends'));
    for (const tool of ['xvfb', 'xauth', 'ffmpeg', 'espeak-ng', 'gnome-keyring', 'lua5.4']) assert.ok(install.split(' ').includes(tool));
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test('required CI aggregate cannot pass when a job fails, skips or is cancelled', () => {
  const jobs = workflow('pr-checks.yml').jobs;
  // Existing branch protection requires this exact context name.
  assert.equal(jobs['all-in-one-fast'].name, 'All-in-one fast profile');
  const gate = jobs.required;
  assert.equal(gate.if, 'always()');
  for (const job of ['frontend', 'dependency-audits', 'rust', 'all-in-one-fast', 'owned-sessions', 'workflow-tests', 'android', 'native', 'linux-arm64']) assert.ok(gate.needs.includes(job));
  assert.equal(jobs['linux-arm64']['runs-on'], 'ubuntu-24.04-arm');
  assert.ok(jobs['linux-arm64'].steps.some(step => step.run?.includes('tauri build -- --bundles deb')));
  assert.match(jobs['linux-arm64'].steps.find(step => step.uses === 'actions/upload-artifact@v4').with.path, /_arm64\.deb/);
  for (const aggregate of [gate, jobs['all-in-one-fast']]) {
    assert.equal(aggregate.if, 'always()');
    const source = aggregate.steps[0].run.match(/node -e '([\s\S]+)'/)?.[1];
    assert.ok(source);
    const evaluate = new Function('process', 'console', source);
    for (const result of ['failure', 'skipped', 'cancelled']) {
      let code;
      evaluate({ env: { RESULTS: JSON.stringify({ test: { result } }) }, exit: value => { code = value; } }, { error() {} });
      assert.equal(code, 1);
    }
  }
});

test('free GitHub runners execute every owned file once and both full browser projects independently', () => {
  const jobs = workflow('pr-checks.yml').jobs;
  const owned = jobs['owned-sessions'];
  assert.equal(owned['runs-on'], 'ubuntu-24.04');
  assert.equal(owned.needs, undefined);
  assert.equal(owned.strategy['fail-fast'], false);
  const matrix = owned.strategy.matrix.include;
  const files = fs.readdirSync(path.join(root, 'tests/dev-session')).filter(file => file.endsWith('.test.mjs')).sort();
  const executed = matrix.filter(row => row.suite === 'node').flatMap(row => partitionFiles(files, sessionPartition(['--suite', row.suite, '--shard', row.shard, '--project', row.project])));
  assert.deepEqual(executed.sort(), files);
  for (const project of ['desktop', 'phone']) {
    const rows = matrix.filter(row => row.suite === 'browser' && row.project === project);
    assert.deepEqual(rows.map(row => row.shard).sort(), ['1/4', '2/4', '3/4', '4/4']);
    const list = shard => {
      const args = ['node_modules/@playwright/test/cli.js', 'test', '--config', 'tests/browser/playwright.config.mjs', '--list', `--project=${project}`, '--reporter=json'];
      if (shard) args.push(`--shard=${shard}`);
      const report = JSON.parse(execFileSync(process.execPath, args, { cwd: root, encoding: 'utf8', env: { ...process.env, VERENU_SESSION_ACCESS_FILE: path.join(os.tmpdir(), 'verenu-ci-list', 'access.json') } }));
      const ids = [];
      const visit = suite => {
        for (const spec of suite.specs || []) for (const test of spec.tests || []) ids.push(`${spec.id}:${test.projectName}`);
        for (const child of suite.suites || []) visit(child);
      };
      report.suites.forEach(visit);
      return ids;
    };
    const all = list();
    const shards = rows.flatMap(row => list(row.shard));
    assert.ok(rows.every(row => list(row.shard).length > 0));
    assert.deepEqual(shards.sort(), all.sort());
    assert.equal(new Set(shards).size, shards.length);
  }
  assert.equal(new Set(matrix.map(row => row.label)).size, matrix.length);
  const step = owned.steps.find(step => step.uses === './.github/actions/regression-checks');
  assert.equal(step.with.profile, 'owned');
  for (const key of ['suite', 'shard', 'project']) assert.equal(step.with[key], '${{ matrix.' + key + ' }}');
  assert.equal(step.with['report-prefix'], 'pr-${{ matrix.label }}');
  assert.deepEqual(jobs['all-in-one-fast'].needs, ['fast-partitions', 'linux-regressions']);
  const shared = YAML.parse(fs.readFileSync(path.join(root, '.github/actions/regression-checks/action.yml'), 'utf8'));
  assert.equal(shared.inputs.profile.default, 'all');
  // Some Node transport files also launch Chromium internally.
  assert.equal(shared.runs.steps.find(step => step.name === 'Install browser').if, "inputs.profile != 'linux'");
  assert.deepEqual(workflow('pr-checks.yml').env, { CARGO_PROFILE_DEV_DEBUG: '0', CARGO_PROFILE_TEST_DEBUG: '0' });
  assert.equal(shared.runs.steps.find(step => step.run?.includes('test:session:owned')).if, "inputs.profile == 'all' || inputs.profile == 'owned'");
  assert.equal(shared.runs.steps.find(step => step.run?.includes('npm test -- --strict')).if, "inputs.profile == 'all' || inputs.profile == 'fast'");
  const promptCompile = shared.runs.steps.find(step => step.run?.includes('prompt_regression_fixtures_hold --no-run'));
  assert.match(promptCompile.if, /inputs\.fast-suites == ''/);
  assert.match(promptCompile.if, /,contract,/);
  assert.equal(shared.runs.steps.find(step => step.run?.includes('cargo build --manifest-path src-tauri/Cargo.toml --features dev-session')).if, "inputs.profile == 'all' || inputs.profile == 'owned'");
});

test('Rust partitions retain every platform check and required status fails closed', () => {
  const jobs = workflow('pr-checks.yml').jobs;
  assert.deepEqual(jobs['rust-partitions'].strategy.matrix.os, ['windows-latest', 'macos-latest']);
  assert.deepEqual(jobs['rust-partitions'].strategy.matrix.check, ['clippy', 'test', 'dev-session']);
  for (const check of ['clippy', 'test', 'dev-session']) assert.ok(jobs['rust-partitions'].steps.some(step => step.if === `matrix.check == '${check}'` && step.run?.includes('cargo ')));
  assert.equal(jobs.rust.if, 'always()');
  assert.equal(jobs.rust.needs, 'rust-partitions');
  assert.match(jobs.rust.steps[0].run, /test "\$RESULT" = success/);
  assert.deepEqual(workflow('release-quality.yml').jobs.rust.strategy.matrix.check, ['test', 'clippy']);
});

test('PR and release partitions cover the complete fast profile and owned sessions exactly once', () => {
  const pr = workflow('pr-checks.yml').jobs;
  const release = workflow('release-quality.yml').jobs;
  const expected = JSON.parse(execFileSync('python3', ['-c', "import json, runpy; print(json.dumps(runpy.run_path('tests/OnePyFone.py')['PROFILE_SUITES']['fast']))"], { cwd: root, encoding: 'utf8' }));
  for (const job of [pr['fast-partitions'], release.fast]) {
    const suites = job.strategy.matrix.include.flatMap(row => row.suites.split(','));
    assert.deepEqual(suites.sort(), expected.sort());
    assert.equal(job.strategy['fail-fast'], false);
    const step = job.steps.find(step => step.uses === './.github/actions/regression-checks');
    assert.equal(step.with.profile, 'fast');
    assert.equal(step.with['fast-suites'], '${{ matrix.suites }}');
  }
  assert.deepEqual(release.owned.strategy.matrix.include, pr['owned-sessions'].strategy.matrix.include);
  for (const job of [pr['linux-regressions'], release.linux]) {
    assert.equal(job.steps.find(step => step.uses === './.github/actions/regression-checks').with.profile, 'linux');
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

test('nightly preparation derives a numeric base from stable and beta manifests', () => {
  const step = workflow('morning-release.yml').jobs.prepare.steps.find(step => step.id === 'changes');
  const source = step.run.match(/nightly_base_version="\$\(node <<'NODE'\n([\s\S]*?)\nNODE\n/)[1];
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'verenu-nightly-base-'));
  try {
    fs.mkdirSync(path.join(directory, 'src-tauri'));
    for (const [version, expected] of [['0.20.0', '0.20.0'], ['0.21.0-beta', '0.21.0'], ['0.21.0-beta.1', '0.21.0'], ['0.21.0-private', null], ['0.21.0-beta.', null]]) {
      for (const file of ['package.json', 'src-tauri/tauri.conf.json']) fs.writeFileSync(path.join(directory, file), JSON.stringify({version}));
      fs.writeFileSync(path.join(directory, 'src-tauri/Cargo.toml'), `[package]\nversion = "${version}"\n`);
      const result = spawnSync(process.execPath, ['-e', source], {cwd:directory, encoding:'utf8', timeout:10_000});
      if (expected) {
        assert.equal(result.status, 0, result.stderr);
        assert.equal(result.stdout, expected);
      } else assert.notEqual(result.status, 0);
    }
  } finally { fs.rmSync(directory, {recursive:true, force:true}); }
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
  assert.match(jobs.native.steps.find(step => step.if === "runner.os == 'Linux'" && step.run?.includes('test:native:webview')).run, /XDG_RUNTIME_DIR/);
  const shared = YAML.parse(fs.readFileSync(path.join(root, '.github/actions/regression-checks/action.yml'), 'utf8'));
  assert.match(shared.runs.steps.find(step => step.run?.includes('test:session:owned')).run, /XDG_RUNTIME_DIR/);
  assert.ok(shared.runs.steps.some(step => step.run?.includes('cargo clippy')));
  const deletedReviewTestCommand = ['node --test scripts/verenu', 'ai', 'review-*.test.mjs'].join('-');
  assert.ok(!jobs['workflow-tests'].steps.some(step => step.run === deletedReviewTestCommand));
});

test('master required-check rules preserve other rules and add no bypass actors', () => {
  const rules = fs.readFileSync(path.join(root, 'scripts/ci/merge-rules.mjs'), 'utf8');
  assert.match(rules, /'CI required'/);
  assert.match(rules, /configuredDiff\.unexpected\.length/);
  assert.doesNotMatch(rules, /effectiveDiff\.unexpected\.length/);
  const contexts = rules.match(/const contexts = \[(.*?)\];/s)?.[1] || '';
  assert.ok(contexts);
  assert.doesNotMatch(contexts, /['"]review['"]/);
  assert.deepEqual(requiredCiContextDiff([
    { context: 'CI required', integration_id: 15368 },
    { context: 'review', integration_id: 15368 },
  ], ['CI required']), { missing: [], unexpected: ['review'] });
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
