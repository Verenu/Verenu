import { describe, expect, it } from 'vitest';
import { createLatestGate, createLatestRefresher } from './latestRequest';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

describe('latest-wins refreshes', () => {
  it('keeps the newest result when responses arrive in reverse order', async () => {
    const calls = [deferred<string[]>(), deferred<string[]>()];
    let index = 0;
    let value: string[] = ['initial'];
    const refresh = createLatestRefresher(() => calls[index++].promise, (next) => { value = next; });

    const first = refresh();
    const second = refresh();
    calls[1].resolve(['after-second-delete']);
    await second;
    calls[0].resolve(['after-first-delete']);
    await first;

    expect(value).toEqual(['after-second-delete']);
  });

  it('applies results in normal order and ignores failures', async () => {
    let value = 0;
    let fail = false;
    const refresh = createLatestRefresher(
      async () => { if (fail) throw new Error('boom'); return value + 1; },
      (next) => { value = next; },
    );
    await refresh();
    fail = true;
    await refresh();
    expect(value).toBe(1);
  });

  it('lets a shared-gate load stamp supersede an older refresh', async () => {
    const gate = createLatestGate();
    const old = deferred<number>();
    let value = 0;
    const refresh = createLatestRefresher(() => old.promise, (next) => { value = next; }, gate);
    const pending = refresh();
    const loadStamp = gate.next();
    if (gate.isLatest(loadStamp)) value = 2;
    old.resolve(1);
    await pending;
    expect(value).toBe(2);
  });

  it('gate marks only the newest stamp as current', () => {
    const gate = createLatestGate();
    const a = gate.next();
    const b = gate.next();
    expect(gate.isLatest(a)).toBe(false);
    expect(gate.isLatest(b)).toBe(true);
  });
});
