import { test as base, expect } from '@playwright/test';
import fs from 'node:fs/promises';
import { sourceIdentity } from '../../scripts/verification/identity.mjs';
export const test = base.extend({
  cachedCatalogs: async ({ session }, use) => {
    const original = await session.invoke('get_setting', { key: 'provider_model_cache' });
    const now = Date.now();
    const cache = Object.fromEntries(['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai'].map((provider) => [provider, {
      ids: [], everSeen: [], lastSuccessAt: now, lastAttemptAt: now,
      lastError: null, missing: {}, metadata: {}, warning: null,
    }]));
    // Native persistence keeps these UI checks independent of public networks
    // and provider credentials. No HTTP or IPC response is intercepted.
    await session.invoke('save_setting', { key: 'provider_model_cache', value: cache });
    try { await use(cache); }
    finally { await session.invoke('save_setting', { key: 'provider_model_cache', value: original ?? {} }); }
  },
  session: async ({}, use, testInfo) => {
    const access = JSON.parse(await fs.readFile(process.env.VERENU_SESSION_ACCESS_FILE, 'utf8'));
    const url = new URL(access.localAccessUrl).origin;
    const invoke = async (command, args = {}) => {
      const response = await fetch(`${url}/__verenu_dev/invoke`, { method: 'POST', headers: { Authorization: `Bearer ${access.token}`, 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args }), signal: AbortSignal.timeout(30_000) });
      expect(response.ok, `Real IPC command ${command}`).toBeTruthy();
      return response.json();
    };
    const metadata = await (await fetch(`${url}/__verenu_dev/session`, { headers: { Authorization: `Bearer ${access.token}` } })).json();
    // Baseline generation intentionally changes source during the run. The
    // runner supplies its initial identity only for explicit update mode and
    // still reports that run incomplete; ordinary verification stays strict.
    const expected = testInfo.config.updateSnapshots === 'all' && process.env.VERENU_SNAPSHOT_SOURCE_FINGERPRINT
      ? process.env.VERENU_SNAPSHOT_SOURCE_FINGERPRINT : sourceIdentity().fingerprint;
    expect(metadata.fingerprint).toBe(expected);
    await use({ access, invoke });
  },
  page: async ({ page, session }, use) => {
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(session.access.localAccessUrl);
    await expect(page.getByRole('button', { name: /^Dev tests/ })).toBeVisible();
    await use(page);
    expect(errors, 'No uncaught application errors').toEqual([]);
  },
});
export { expect };
