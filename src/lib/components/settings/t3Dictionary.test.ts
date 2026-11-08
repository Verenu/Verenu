import { describe, expect, it } from 'vitest';
import { dictionaryStatus } from './t3Dictionary';

const write = ['orchestration:read', 'settings:write'];
const synced = { syncedTerms: ['Verenu', 'Omarchy'], syncedAt: 1_000, attemptedAt: 1_000, skipped: 0, error: null };

describe('dictionaryStatus', () => {
  it('asks old read-only pairings to reconnect', () => {
    expect(dictionaryStatus({ scopes: ['orchestration:read'] })).toMatchObject({ warn: true, text: expect.stringContaining('Reconnect') });
    expect(dictionaryStatus({})).toMatchObject({ warn: true });
  });
  it('reports counts and age after a sync', () => {
    expect(dictionaryStatus({ scopes: write, dictionary: synced }, 1_000_000 + 60_000)).toEqual({ text: '2 dictionary terms shared with T3 Code · 1 min ago', warn: false });
    expect(dictionaryStatus({ scopes: write, dictionary: { ...synced, syncedTerms: ['One'] } }, 1_000_000).text).toContain('1 dictionary term shared');
  });
  it('warns about terms that did not fit and about failures', () => {
    expect(dictionaryStatus({ scopes: write, dictionary: { ...synced, skipped: 3 } }, 1_000_000)).toMatchObject({ warn: true, text: expect.stringContaining('3 did not fit') });
    expect(dictionaryStatus({ scopes: write, dictionary: { ...synced, error: 'Cannot reach T3.' } })).toEqual({ text: 'Cannot reach T3.', warn: true });
  });
  it('shows progress before the first sync', () => {
    expect(dictionaryStatus({ scopes: write, dictionary: { ...synced, syncedAt: 0, syncedTerms: [] } })).toEqual({ text: 'Sharing your dictionary with T3 Code…', warn: false });
  });
});
