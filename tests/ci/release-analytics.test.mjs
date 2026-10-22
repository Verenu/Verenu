import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import YAML from 'yaml';
import { root } from '../../scripts/verification/identity.mjs';

const preflightPath = path.join(root, 'scripts/check-release-analytics.mjs');
const workflow = name => YAML.parse(fs.readFileSync(path.join(root, '.github/workflows', name), 'utf8'));

function runPreflight(overrides) {
  return spawnSync(process.execPath, [preflightPath], {
    cwd: root,
    encoding: 'utf8',
    env: {
      ...process.env,
      POSTHOG_PROJECT_TOKEN: '',
      POSTHOG_HOST: 'https://us.i.posthog.com',
      ...overrides,
    },
  });
}

test('release analytics preflight fails on missing token without printing credentials', () => {
  const result = runPreflight({ POSTHOG_PROJECT_TOKEN: '' });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /POSTHOG_PROJECT_TOKEN must contain a project ingestion token/);
  assert.doesNotMatch(result.stdout + result.stderr, /test-ingestion-token/);
});

test('release analytics preflight accepts an ingestion token and HTTPS host without printing the token', () => {
  const result = runPreflight({ POSTHOG_PROJECT_TOKEN: 'test-ingestion-token' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Release analytics configuration is valid/);
  assert.doesNotMatch(result.stdout + result.stderr, /test-ingestion-token/);
});

test('release analytics preflight rejects invalid hosts', () => {
  const result = runPreflight({
    POSTHOG_PROJECT_TOKEN: 'test-ingestion-token',
    POSTHOG_HOST: 'http://us.i.posthog.com',
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /POSTHOG_HOST must be an HTTPS ingestion base URL/);
  assert.doesNotMatch(result.stdout + result.stderr, /test-ingestion-token/);

  for (const host of [' https://us.i.posthog.com', 'https://us.i.posthog.com/path']) {
    const invalidHost = runPreflight({
      POSTHOG_PROJECT_TOKEN: 'test-ingestion-token',
      POSTHOG_HOST: host,
    });
    assert.equal(invalidHost.status, 1);
    assert.match(invalidHost.stderr, /POSTHOG_HOST must be an HTTPS ingestion base URL/);
    assert.doesNotMatch(invalidHost.stdout + invalidHost.stderr, /test-ingestion-token/);
  }
});

test('manual installer builds are gated by PostHog validation and require analytics at compile time', () => {
  const manual = workflow('build-installers.yml');
  const validation = manual.jobs['validate-release-analytics'];
  assert.equal(validation.steps.find(step => step.run)?.run, 'node scripts/check-release-analytics.mjs');
  assert.equal(validation.env.POSTHOG_PROJECT_TOKEN, '${{ secrets.POSTHOG_PROJECT_TOKEN }}');
  assert.equal(validation.env.POSTHOG_HOST, "${{ vars.POSTHOG_HOST || 'https://us.i.posthog.com' }}");
  assert.equal(manual.jobs.build.needs, 'validate-release-analytics');
  assert.deepEqual(manual.jobs.build.strategy.matrix.include.map(entry => entry['os-name']), ['windows', 'macos', 'linux']);
  const build = manual.jobs.build.steps.find(step => step.name === 'Build Tauri release app');
  assert.equal(build.env.VERENU_REQUIRE_POSTHOG, '1');
});

test('nightly release validates analytics before notes and tags, then builds the validated snapshot', () => {
  const nightly = workflow('morning-release.yml');
  const steps = nightly.jobs.prepare.steps;
  const changesIndex = steps.findIndex(step => step.id === 'changes');
  const validationIndex = steps.findIndex(step => step.run === 'node scripts/check-release-analytics.mjs');
  const notesIndex = steps.findIndex(step => step.name === 'Write AI release notes');
  const tagIndex = steps.findIndex(step => step.name === 'Create the versioned nightly tag');
  const validation = steps[validationIndex];
  assert.ok(changesIndex < validationIndex && validationIndex < notesIndex && notesIndex < tagIndex);
  assert.equal(validation.if, "steps.changes.outputs.should_release == 'true'");
  assert.equal(validation.env.POSTHOG_PROJECT_TOKEN, '${{ secrets.POSTHOG_PROJECT_TOKEN }}');
  assert.equal(validation.env.POSTHOG_HOST, "${{ vars.POSTHOG_HOST || 'https://us.i.posthog.com' }}");
  assert.equal(nightly.jobs.build.needs, 'prepare');
  assert.equal(nightly.jobs.build.steps.find(step => step.uses === 'actions/checkout@v6').with.ref, '${{ needs.prepare.outputs.release_sha }}');
  const build = nightly.jobs.build.steps.find(step => step.name === 'Build and name installers');
  assert.equal(build.env.VERENU_REQUIRE_POSTHOG, '1');
  assert.deepEqual(nightly.jobs.build.strategy.matrix.include.map(entry => entry['os-name']), ['windows', 'macos', 'linux']);
});
