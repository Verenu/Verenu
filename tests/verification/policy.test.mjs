import test from 'node:test';
import assert from 'node:assert/strict';
import { requirements, nativeRequirements, evaluate, incompleteUnlessFailed } from '../../scripts/verification/policy.mjs';
const identity = { fingerprint: 'current', worktree: '/owned' };
const passed = { category: 'session', status: 'passed', ...identity };
test('backend setting writes require Rust and real session checks', () => {
  assert.ok(requirements(['src-tauri/src/commands/settings.rs']).includes('session'));
  assert.ok(requirements(['src-tauri/src/commands/settings.rs']).includes('rust'));
});
test('native source cannot be verified by renderer checks', () => {
  assert.ok(requirements(['src-tauri/src/core/hotkey/linux.rs']).includes('native'));
  assert.equal(evaluate(['native'], [{ ...passed, category: 'renderer' }], identity).status, 'incomplete');
});
test('missing, skipped, stale, foreign and flaky evidence is incomplete', () => {
  for (const records of [[], [{ ...passed, status: 'skipped' }], [{ ...passed, fingerprint: 'old' }], [{ ...passed, worktree: '/other' }], [{ ...passed, flaky: true }]]) {
    assert.equal(evaluate(['session'], records, identity).status, 'incomplete');
  }
});
test('any task-required failure defeats later passing evidence', () => {
  assert.equal(evaluate(['session'], [{ ...passed, status: 'failed' }, passed], identity).status, 'failed');
});
test('acceptance evidence needs an observed outcome bound to current code', () => {
  const criteria = [{ id: 'saved', expected: 'Context survives restart' }];
  assert.equal(evaluate(['session'], [passed], identity, criteria).status, 'incomplete');
  const outcome = { ...passed, category: 'acceptance', criterion: 'saved', observed: 'Context remained after restart', artifacts: [{ path: '/evidence', sha256: 'hash' }] };
  assert.equal(evaluate(['session'], [passed, { ...outcome, artifacts: [] }], identity, criteria).status, 'incomplete');
  assert.equal(evaluate(['session'], [passed, outcome], identity, criteria).status, 'verified');
  assert.equal(evaluate(['session'], [passed, outcome, { ...outcome, status: 'failed' }], identity, criteria).status, 'failed');
});
test('requirements can only be extended and unknown categories are rejected', () => {
  assert.ok(requirements(['src/App.svelte'], ['native']).includes('inspection'));
  assert.ok(requirements(['src/App.svelte'], ['native']).includes('native'));
  assert.throws(() => requirements([], ['made-up']));
});
test('incomplete outcomes never downgrade a failed verification', () => {
  assert.equal(incompleteUnlessFailed('failed'), 'failed');
  assert.equal(incompleteUnlessFailed('verified'), 'incomplete');
  assert.equal(incompleteUnlessFailed('incomplete'), 'incomplete');
});

test('CI, HTML, native pill and Android edits select their relevant checks', () => {
  for (const [file, expected] of [
    ['.github/workflows/pr-checks.yml', ['ci', 'unit']],
    ['pill.html', ['renderer', 'session', 'inspection', 'native']],
    ['src-tauri/src/pipeline/pill.rs', ['rust', 'pipeline', 'native']],
    ['src-tauri/android/kotlin/com/verenu/app/VerenuBridge.kt', ['android']],
    ['tests/browser/session.spec.mjs', ['session']],
  ]) for (const category of expected) assert.ok(requirements([file]).includes(category), `${file}: ${category}`);
});

test('native evidence cannot substitute another platform or capability', () => {
  const native = nativeRequirements(['src-tauri/src/core/injection/linux.rs']);
  assert.deepEqual(native, ['insertion', 'clipboard', 'focus'].map(scope => ({ platform: 'linux', scope })));
  const row = { ...passed, category: 'native-integration', platform: 'linux', scope: ['focused-text'] };
  assert.equal(evaluate(['native-integration'], [row], identity, [], native).status, 'incomplete');
  assert.equal(evaluate(['native-integration'], [{ ...row, platform: 'darwin', scope: ['insertion', 'clipboard', 'focus'] }], identity, [], native).status, 'incomplete');
  assert.equal(evaluate(['native-integration'], [{ ...row, scope: ['insertion', 'clipboard', 'focus'] }], identity, [], native).status, 'verified');
  assert.equal(evaluate(['native-integration'], [{ ...passed, category: 'native-integration' }], identity).status, 'incomplete');
  assert.throws(() => nativeRequirements([], [{ platform: 'unknown', scope: 'clipboard' }]));
});

test('flaky or skipped acceptance cannot be erased by a later passing record', () => {
  const row = { ...passed, category: 'acceptance', criterion: 'saved', observed: 'Saved', artifacts: [{ path: '/evidence' }] };
  for (const invalid of [{ ...row, flaky: true }, { ...row, status: 'skipped' }]) {
    assert.equal(evaluate([], [invalid, row], identity, [{ id: 'saved', expected: 'Saved' }]).status, 'incomplete');
  }
});
