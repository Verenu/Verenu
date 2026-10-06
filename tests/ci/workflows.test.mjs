import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import YAML from 'yaml';
import { root } from '../../scripts/verification/identity.mjs';

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
