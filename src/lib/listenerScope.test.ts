import { describe, expect, it, vi } from 'vitest';
import { createListenerScope } from './listenerScope';

function pendingListener() {
  let resolve!: (unlisten: () => void) => void;
  const promise = new Promise<() => void>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('async listener lifetime', () => {
  it('registers concurrently and waits for every listener before readiness', async () => {
    const scope = createListenerScope();
    const first = pendingListener();
    const second = pendingListener();
    const ready = vi.fn();
    const registrations = Promise.all([scope.track(first.promise), scope.track(second.promise)]).then(ready);
    const firstCleanup = vi.fn();
    const secondCleanup = vi.fn();
    second.resolve(secondCleanup);
    await Promise.resolve();
    expect(ready).not.toHaveBeenCalled();
    first.resolve(firstCleanup);
    await registrations;
    expect(ready).toHaveBeenCalledOnce();
    scope.dispose();
    scope.dispose();
    expect(firstCleanup).toHaveBeenCalledOnce();
    expect(secondCleanup).toHaveBeenCalledOnce();
  });

  it('removes a registration that resolves after unmount', async () => {
    const scope = createListenerScope();
    const pending = pendingListener();
    const tracked = scope.track(pending.promise);
    const cleanup = vi.fn();
    scope.dispose();
    pending.resolve(cleanup);
    await tracked;
    expect(cleanup).toHaveBeenCalledOnce();
  });

  it('cleans both installed and late listeners when a registration fails', async () => {
    const scope = createListenerScope();
    const pending = pendingListener();
    const installedCleanup = vi.fn();
    const lateCleanup = vi.fn();
    const failure = new Error('registration failed');
    const registrations = Promise.all([
      scope.track(Promise.resolve(installedCleanup)),
      scope.track(Promise.reject(failure)),
      scope.track(pending.promise),
    ]).catch((error) => { scope.dispose(); throw error; });
    await expect(registrations).rejects.toBe(failure);
    expect(installedCleanup).toHaveBeenCalledOnce();
    pending.resolve(lateCleanup);
    await Promise.resolve();
    expect(lateCleanup).toHaveBeenCalledOnce();
  });
});
