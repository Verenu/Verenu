import { afterEach, expect, it, vi } from 'vitest';
import { credentialStatusWithin } from './credentialStatus';

afterEach(() => vi.useRealTimers());

it('returns a completed credential status', async () => {
  await expect(credentialStatusWithin(Promise.resolve({ groq: true }), 2000)).resolves.toEqual({ groq: true });
});

it('continues when credential status does not complete', async () => {
  vi.useFakeTimers();
  const result = credentialStatusWithin(new Promise<never>(() => {}), 2000);
  await vi.advanceTimersByTimeAsync(2000);
  await expect(result).resolves.toBeNull();
});

it('treats an unavailable credential store as unknown', async () => {
  await expect(credentialStatusWithin(Promise.reject(new Error('unavailable')), 2000)).resolves.toBeNull();
});
