import assert from 'node:assert/strict';
import test from 'node:test';
import { nativeNavigationState, nativePageIdentity, waitForNativeNavigation } from '../../scripts/verification/native-navigation.mjs';

const identity = 'VERENU_NATIVE_PAGE pid=12 start=34 page=56 kind=main port=1234\n';
const active = [{ pid: 12, start: '34' }];
const options = { kind: 'main', port: 1234, active };
const start = 'VERENU_NAV page=56 event=0\n';
const finish = 'VERENU_NAV page=56 event=3\n';

test('native navigation accepts fast start and finish after the baseline', () => {
  const before = identity + start + finish;
  const expectedPage = nativePageIdentity(before, options);
  assert.equal(nativeNavigationState(before + start + finish, { ...options, expectedPage, after: before.length }).page, '56');
  assert.equal(nativeNavigationState(before, { ...options, expectedPage, after: before.length }), false);
  assert.equal(nativeNavigationState(before + start + finish, options).page, '56');
  assert.equal(nativeNavigationState(before + start, options), false);
});

test('initial pill load can finish before its identity is reported', () => {
  const log = 'VERENU_NAV page=56 event=0\nVERENU_NAV page=56 event=3\nVERENU_NATIVE_PAGE pid=12 start=34 page=56 kind=pill port=1234\n';
  assert.equal(nativeNavigationState(log, { ...options, kind: 'pill' }).page, '56');
});

test('wrong pages, missing starts, and missing finishes do not satisfy the barrier', () => {
  assert.equal(nativeNavigationState(identity + 'VERENU_NAV page=99 event=0\nVERENU_NAV page=99 event=3\n', options), false);
  assert.equal(nativeNavigationState(identity + finish, options), false);
  assert.equal(nativeNavigationState(identity + start, options), false);
});

test('failed or overlapping navigations and ambiguous identities fail closed', () => {
  assert.throws(() => nativeNavigationState(identity + start + 'VERENU_NAV page=56 event=failed\n' + finish, options), /navigation failed/);
  assert.throws(() => nativeNavigationState(identity + start + start + finish, options), /Overlapping/);
  const other = 'VERENU_NATIVE_PAGE pid=13 start=35 page=57 kind=main port=1234\n';
  assert.throws(() => nativePageIdentity(identity + other, { ...options, active: [...active, { pid: 13, start: '35' }] }), /Ambiguous/);
  assert.throws(() => nativeNavigationState(identity + start + finish, { ...options, expectedPage: { pid: 99, start: '1', page: '56' } }), /identity changed/);
});

test('destroyed page reports cannot compete with a live replacement in the same renderer', () => {
  const gone = 'VERENU_NATIVE_PAGE_GONE pid=12 page=56\n';
  const replacement = 'VERENU_NATIVE_PAGE pid=12 start=34 page=57 kind=main port=1234\n';
  const events = 'VERENU_NAV page=57 event=0\nVERENU_NAV page=57 event=3\n';
  assert.equal(nativeNavigationState(identity + gone + replacement + events, options).page, '57');
  assert.equal(nativePageIdentity(identity + gone, options), null);
});

test('native navigation waits for delayed identity and fails on a missing event', async () => {
  const snapshots = [start + finish, start + finish + identity];
  let calls = 0;
  const result = await waitForNativeNavigation(async () => ({ log: snapshots[Math.min(calls++, 1)], active }), options, { sleep: async () => {} });
  assert.equal(result.page, '56');
  assert.equal(calls, 2);
  await assert.rejects(waitForNativeNavigation(async () => ({ log: identity + start, active }), options, { deadlineMs: 1, sleep: async () => {} }), /did not finish/);
});
