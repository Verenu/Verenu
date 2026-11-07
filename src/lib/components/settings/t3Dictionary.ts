import { timeAgo } from './t3SkillNames';

export type T3DictionarySync = { syncedTerms: string[]; syncedAt: number; attemptedAt: number; skipped: number; error: string | null };
type Shareable = { scopes?: string[]; dictionary?: T3DictionarySync };

export const T3_DICTIONARY_LIMIT = 100;

/** One status sentence for the T3 Code row, and whether it needs attention. */
export function dictionaryStatus(connection: Shareable, nowMs = Date.now()): { text: string; warn: boolean } {
  if (!connection.scopes?.includes('settings:write')) {
    return { text: 'Reconnect to share your dictionary. This pairing cannot change T3 Code settings.', warn: true };
  }
  const state = connection.dictionary;
  if (state?.error) return { text: state.error, warn: true };
  if (!state?.syncedAt) return { text: 'Sharing your dictionary with T3 Code…', warn: false };
  const count = state.syncedTerms.length;
  const shared = `${count} dictionary ${count === 1 ? 'term' : 'terms'} shared with T3 Code · ${timeAgo(state.syncedAt, nowMs)}`;
  return state.skipped > 0
    ? { text: `${shared}. ${state.skipped} did not fit T3's ${T3_DICTIONARY_LIMIT}-entry limit.`, warn: true }
    : { text: shared, warn: false };
}
