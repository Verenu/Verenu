import { describe, expect, it, vi } from 'vitest';

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('./tauri', () => ipc);

import {
  dictionaryEntryCorrectionId,
  dictionaryEntryId,
  dictionaryUpdateContextId,
  editContextDictionaryEntry,
  moveContextDictionaryEntry,
  normalizeContextDictionary,
  removeContextDictionaryEntry,
  shouldRefreshContextDictionary,
} from './contextDictionary';

describe('Context dictionary DTO compatibility', () => {
  it('keeps canonical and Context-owned mapping ids distinct', () => {
    const [entry] = normalizeContextDictionary([{
      id: 41,
      term: 'Kubernetes',
      corrections: [{
        id: 902,
        mistake: 'Koobernetes',
        auto_learned: true,
        correction_count: 1,
        confidence_tier: 'high',
        last_seen_at: '2026-09-09T12:00:00Z',
        created_at: '2026-09-09T11:59:00Z',
      }, {
        id: 903,
        mistake: 'Koobernettis',
        auto_learned: true,
        correction_count: 0,
        confidence_tier: 'medium',
        last_seen_at: null,
        created_at: '2026-09-09T11:58:00Z',
      }],
      created_at: '2026-01-01T00:00:00Z',
    }], 7);

    expect(entry).toMatchObject({
      id: 41,
      dictionary_id: 41,
      context_id: 7,
      correction_id: 902,
      correction_ids: [902, 903],
      mistake: 'Koobernetes, Koobernettis',
      auto_learned: true,
      confidence_tier: 'high',
    });
    expect(dictionaryEntryId(entry)).toBe(41);
    expect(dictionaryEntryCorrectionId(entry)).toBe(902);
  });

  it('accepts the pre-migration flat row without inventing a mapping id', () => {
    const [entry] = normalizeContextDictionary([{
      id: 4,
      term: 'Acme',
      mistake: 'Ackme',
      auto_learned: false,
      correction_count: 0,
      confidence_tier: 'manual',
      last_seen_at: null,
      created_at: '2026-01-01T00:00:00Z',
    }], 1);

    expect(entry.dictionary_id).toBe(4);
    expect(entry.correction_id).toBeNull();
    expect(entry.mistake).toBe('Ackme');
  });
});

describe('Context dictionary update events', () => {
  it('refreshes only the originating Context for scoped events', () => {
    expect(dictionaryUpdateContextId({ context_id: 7 })).toBe(7);
    expect(shouldRefreshContextDictionary({ context_id: 7 }, 7)).toBe(true);
    expect(shouldRefreshContextDictionary({ context_id: 7 }, 8)).toBe(false);
  });

  it('refreshes on legacy unscoped and explicit global updates', () => {
    expect(dictionaryUpdateContextId(undefined)).toBeUndefined();
    expect(shouldRefreshContextDictionary(undefined, 8)).toBe(true);
    expect(dictionaryUpdateContextId({ context_id: null })).toBeNull();
    expect(shouldRefreshContextDictionary({ context_id: null }, 8)).toBe(true);
  });
});

describe('Context dictionary mutations', () => {
  it('passes canonical, mapping, and Context identities to IPC', async () => {
    ipc.invoke.mockReset().mockResolvedValue(undefined);
    const entry = {
      id: 41,
      dictionary_id: 41,
      correction_id: 902,
      correction_ids: [902, 903],
    };

    await editContextDictionaryEntry(entry, 7, 'Kubernetes', 'Koobernetes');
    expect(ipc.invoke).toHaveBeenLastCalledWith('edit_dictionary_entry', expect.objectContaining({
      id: 41,
      dictionaryId: 41,
      contextId: 7,
      correctionId: 902,
      correctionIds: [902, 903],
    }));

    await removeContextDictionaryEntry(entry, 7);
    expect(ipc.invoke).toHaveBeenLastCalledWith('remove_dictionary_entry', expect.objectContaining({
      id: 41,
      contextId: 7,
      correctionIds: [902, 903],
    }));

    await moveContextDictionaryEntry(entry, 7, 8);
    expect(ipc.invoke).toHaveBeenLastCalledWith('move_dictionary_entry_to_context', {
      dictionaryId: 41,
      sourceContextId: 7,
      targetContextId: 8,
      correctionId: 902,
      correctionIds: [902, 903],
    });
  });
});
