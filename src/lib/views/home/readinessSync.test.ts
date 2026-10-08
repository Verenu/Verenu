import { describe, expect, it, vi } from 'vitest';
import {
  HOME_READINESS_SYNC_EVENT,
  listenForHomeReadinessSyncChanges,
  type ReadinessSyncEvent,
} from './readinessSync';

describe('Home readiness sync updates', () => {
  it('refreshes only for synced settings or Contexts and unregisters on disposal', async () => {
    let eventName = '';
    let receive: ((event: ReadinessSyncEvent) => void) | undefined;
    const unlisten = vi.fn();
    const listen = vi.fn(async (name: typeof HOME_READINESS_SYNC_EVENT, handler: (event: ReadinessSyncEvent) => void) => {
      eventName = name;
      receive = handler;
      return unlisten;
    });
    const refresh = vi.fn();
    const stop = listenForHomeReadinessSyncChanges(listen, refresh);

    await Promise.resolve();
    expect(eventName).toBe('verenu:sync-data-changed');
    receive?.({ payload: { tables: ['dictionary', 'snippets'] } });
    expect(refresh).not.toHaveBeenCalled();

    receive?.({ payload: { tables: ['settings'] } });
    receive?.({ payload: { tables: ['contexts'] } });
    expect(refresh).toHaveBeenCalledTimes(2);

    stop();
    receive?.({ payload: { tables: ['settings'] } });
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it('unregisters when the component is disposed before async registration finishes', async () => {
    let finishRegistration!: (unlisten: () => void) => void;
    let receive: ((event: ReadinessSyncEvent) => void) | undefined;
    const unlisten = vi.fn();
    const listen = (_name: typeof HOME_READINESS_SYNC_EVENT, handler: (event: ReadinessSyncEvent) => void) => {
      receive = handler;
      return new Promise<() => void>((resolve) => { finishRegistration = resolve; });
    };
    const refresh = vi.fn();
    const stop = listenForHomeReadinessSyncChanges(listen, refresh);

    stop();
    finishRegistration(unlisten);
    await Promise.resolve();
    receive?.({ payload: { tables: ['contexts'] } });

    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(refresh).not.toHaveBeenCalled();
  });
});
