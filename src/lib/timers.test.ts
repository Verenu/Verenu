import { afterEach, expect, it, vi } from 'vitest';
import { createTimers } from './timers';

afterEach(() => vi.useRealTimers());

it('replaces stale callbacks and releases the key before rescheduling', () => {
  vi.useFakeTimers();
  const timers = createTimers<'dismiss'>();
  const stale = vi.fn();
  const next = vi.fn();
  timers.set('dismiss', stale, 10);
  timers.set('dismiss', () => {
    expect(timers.has('dismiss')).toBe(false);
    timers.set('dismiss', next, 20);
  }, 10);
  vi.advanceTimersByTime(10);
  expect(stale).not.toHaveBeenCalled();
  expect(timers.has('dismiss')).toBe(true);
  vi.advanceTimersByTime(20);
  expect(next).toHaveBeenCalledOnce();
  expect(timers.has('dismiss')).toBe(false);
});

it('cancels groups and disposes every pending timeout', () => {
  vi.useFakeTimers();
  const timers = createTimers<'audio' | 'dismiss' | 'settle'>();
  const callback = vi.fn();
  timers.set('audio', callback, 600);
  timers.set('dismiss', callback, 10000);
  timers.clear('audio', 'dismiss');
  timers.set('settle', callback, 200);
  timers.clearAll();
  vi.runAllTimers();
  expect(callback).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});
