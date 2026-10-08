import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SyncMutingSetting } from './syncMutingSetting.svelte';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('SyncMutingSetting', () => {
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('keeps the switch disabled until the saved value has loaded', async () => {
    const load = deferred<boolean>();
    const setting = new SyncMutingSetting({ load: () => load.promise, save: vi.fn() });

    const pending = setting.load();
    expect(setting.disabled).toBe(true);

    load.resolve(true);
    await pending;
    expect(setting.enabled).toBe(true);
    expect(setting.disabled).toBe(false);
  });

  it('treats a missing saved value as off', async () => {
    const setting = new SyncMutingSetting({ load: async () => null, save: vi.fn() });
    await setting.load();
    expect(setting.enabled).toBe(false);
    expect(setting.disabled).toBe(false);
  });

  it('ignores a second change while a save is in flight', async () => {
    const save = deferred<void>();
    const saveFn = vi.fn(() => save.promise);
    const setting = new SyncMutingSetting({ load: async () => false, save: saveFn });
    await setting.load();

    const first = setting.setEnabled(true);
    expect(setting.disabled).toBe(true);
    await setting.setEnabled(false);

    expect(saveFn).toHaveBeenCalledOnce();
    expect(saveFn).toHaveBeenCalledWith(true);
    expect(setting.enabled).toBe(true);

    save.resolve();
    await first;
    expect(setting.saving).toBe(false);
    expect(setting.disabled).toBe(false);
  });

  it('rolls back the optimistic value and reports a failed save', async () => {
    const setting = new SyncMutingSetting({
      load: async () => false,
      save: async () => {
        throw new Error('write failed');
      },
    });
    await setting.load();

    await setting.setEnabled(true);

    expect(setting.enabled).toBe(false);
    expect(setting.saveFailed).toBe(true);
    expect(setting.flashError).toBe(true);
    expect(setting.disabled).toBe(false);
  });

  it('clears the save error after a later save succeeds', async () => {
    const save = vi.fn()
      .mockRejectedValueOnce(new Error('write failed'))
      .mockResolvedValueOnce(undefined);
    const setting = new SyncMutingSetting({ load: async () => false, save });
    await setting.load();

    await setting.setEnabled(true);
    expect(setting.saveFailed).toBe(true);

    await setting.setEnabled(true);
    expect(setting.saveFailed).toBe(false);
    expect(setting.enabled).toBe(true);
  });

  it('keeps the switch disabled after a failed load until retry succeeds', async () => {
    const load = vi.fn()
      .mockRejectedValueOnce(new Error('ipc unavailable'))
      .mockResolvedValueOnce(true);
    const setting = new SyncMutingSetting({ load, save: vi.fn() });

    await setting.load();
    expect(setting.loadFailed).toBe(true);
    expect(setting.disabled).toBe(true);

    await setting.load();
    expect(load).toHaveBeenCalledTimes(2);
    expect(setting.loadFailed).toBe(false);
    expect(setting.enabled).toBe(true);
    expect(setting.disabled).toBe(false);
  });

  it('does not reload a value that already loaded', async () => {
    const load = vi.fn(async () => true);
    const setting = new SyncMutingSetting({ load, save: vi.fn() });

    await setting.load();
    await setting.load();

    expect(load).toHaveBeenCalledOnce();
  });
});
