import { describe, expect, it, vi } from 'vitest';
import {
  createForwardLimiter,
  formatForwardedArgs,
  formatForwardedValue,
  installFrontendLogForwarding,
} from './frontendLog';

describe('formatForwardedValue', () => {
  it('names an Error by its type and message, without the stack', () => {
    expect(formatForwardedValue(new TypeError('bad input'))).toBe('TypeError: bad input');
  });

  it('keeps strings and serializes objects', () => {
    expect(formatForwardedValue('plain')).toBe('plain');
    expect(formatForwardedValue({ code: 7 })).toBe('{"code":7}');
  });

  it('survives values JSON cannot encode', () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expect(formatForwardedValue(cyclic)).toBe('[object Object]');
  });

  it('joins console arguments with spaces', () => {
    expect(formatForwardedArgs(['refresh failed:', new Error('offline')])).toBe('refresh failed: Error: offline');
  });
});

describe('createForwardLimiter', () => {
  it('allows the limit per window, then resets when the window passes', () => {
    let now = 0;
    const allow = createForwardLimiter(2, 1000, () => now);
    expect([allow(), allow(), allow()]).toEqual([true, true, false]);
    now = 1500;
    expect(allow()).toBe(true);
  });

  it('slides, so a burst across a window boundary still counts against the limit', () => {
    let now = 0;
    const allow = createForwardLimiter(2, 1000, () => now);
    expect(allow()).toBe(true);
    now = 600;
    expect(allow()).toBe(true);
    now = 999;
    expect(allow()).toBe(false);
    now = 1000;
    expect(allow()).toBe(true);
    now = 1599;
    expect(allow()).toBe(false);
  });
});

function fakeTarget() {
  const listeners: Record<string, (event: unknown) => void> = {};
  const errorSpy = vi.fn();
  const warnSpy = vi.fn();
  const console = { error: errorSpy, warn: warnSpy } as unknown as Console;
  return {
    listeners,
    console,
    errorSpy,
    addEventListener: (name: string, handler: (event: unknown) => void) => {
      listeners[name] = handler;
    },
  };
}

describe('installFrontendLogForwarding', () => {
  it('forwards console errors and keeps printing them', () => {
    const target = fakeTarget();
    const send = vi.fn().mockResolvedValue(undefined);
    installFrontendLogForwarding(send, target as never);
    target.console.error('refresh failed:', new Error('offline'));
    expect(target.errorSpy).toHaveBeenCalledWith('refresh failed:', expect.any(Error));
    expect(send).toHaveBeenCalledWith('error', 'console.error', 'refresh failed: Error: offline');
  });

  it('forwards window errors with their location and unhandled rejections', () => {
    const target = fakeTarget();
    const send = vi.fn().mockResolvedValue(undefined);
    installFrontendLogForwarding(send, target as never);
    target.listeners.error({ message: 'x is undefined', filename: 'app.js', lineno: 4, colno: 9 });
    target.listeners.unhandledrejection({ reason: new Error('lost') });
    expect(send).toHaveBeenNthCalledWith(1, 'error', 'window.error', 'x is undefined at app.js:4:9');
    expect(send).toHaveBeenNthCalledWith(2, 'error', 'unhandledrejection', 'Error: lost');
  });

  it('does not let a failing forward break the console call', () => {
    const target = fakeTarget();
    const send = vi.fn().mockRejectedValue(new Error('no command'));
    installFrontendLogForwarding(send, target as never);
    expect(() => target.console.error('still printed')).not.toThrow();
    expect(target.errorSpy).toHaveBeenCalledWith('still printed');
  });
});
