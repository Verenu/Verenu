import { invoke } from './tauri';
import type { DictionaryCorrection, DictionaryEntry } from './stores';

/**
 * The canonical dictionary row is still identified by `dictionary_id` (and
 * exposed as `id` for the legacy Dictionary surface).  A Context row may also
 * carry the id of the Context-owned correction mapping that supplied its
 * effective mistake/confidence fields.
 *
 * The backend should return these fields from `get_context_dictionary`.  The
 * normalizer below also accepts the pre-migration row shape so a frontend
 * update can be deployed alongside an older backend during development.
 */
export type ContextDictionaryEntry = DictionaryEntry & {
  context_id: number;
  dictionary_id: number;
  correction_id: number | null;
  correction_ids: number[];
  corrections: DictionaryCorrection[];
};

export type ContextDictionaryIdentity = Pick<
  DictionaryEntry,
  'id' | 'dictionary_id' | 'correction_id' | 'correction_ids' | 'corrections'
>;

export interface DictionaryUpdatedPayload {
  context_id?: number | null;
  contextId?: number | null;
  dictionary_id?: number | null;
  dictionaryId?: number | null;
  correction_id?: number | null;
  correctionId?: number | null;
}

type JsonRecord = Record<string, unknown>;

function recordOf(value: unknown): JsonRecord | null {
  return typeof value === 'object' && value !== null ? value as JsonRecord : null;
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function firstFinite(...values: unknown[]): number | null {
  for (const value of values) {
    const number = finiteNumber(value);
    if (number !== null) return number;
  }
  return null;
}

function nullableText(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function requiredText(value: unknown, field: string): string {
  if (typeof value === 'string' && value.trim()) return value;
  throw new Error(`Context dictionary row is missing ${field}.`);
}

function booleanValue(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function nonNegativeCount(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? Math.floor(value)
    : 0;
}

function confidenceTier(value: unknown, autoLearned: boolean): DictionaryEntry['confidence_tier'] {
  if (value === 'manual' || value === 'low' || value === 'medium' || value === 'high') return value;
  return autoLearned ? 'low' : 'manual';
}

function normalizeCorrection(value: unknown, dictionaryId: number, contextId: number): DictionaryCorrection | null {
  const row = recordOf(value);
  if (!row) return null;
  const id = firstFinite(row.id);
  const mistake = row.mistake;
  if (id === null || typeof mistake !== 'string' || !mistake.trim()) return null;
  const autoLearned = booleanValue(row.auto_learned, false);
  return {
    id,
    dictionary_id: firstFinite(row.dictionary_id, row.dictionaryId) ?? dictionaryId,
    context_id: firstFinite(row.context_id, row.contextId) ?? contextId,
    mistake,
    auto_learned: autoLearned,
    correction_count: nonNegativeCount(row.correction_count),
    confidence_tier: typeof row.confidence_tier === 'string'
      ? row.confidence_tier
      : confidenceTier(row.confidence_tier, autoLearned) ?? 'manual',
    last_seen_at: nullableText(row.last_seen_at),
    created_at: requiredText(row.created_at, 'correction.created_at'),
  };
}

/**
 * Convert a context vocabulary DTO into the stable frontend row shape.
 * Context identity comes from the request, not from a later process/domain
 * lookup; this prevents a stale or malformed response from changing scope.
 */
export function normalizeContextDictionaryEntry(value: unknown, contextId: number): ContextDictionaryEntry {
  const row = recordOf(value);
  if (!row) throw new Error('Context dictionary returned an invalid row.');

  const dictionaryId = firstFinite(
    row.dictionary_id,
    row.dictionaryId,
    row.canonical_id,
    row.canonicalId,
    row.id,
  );
  if (dictionaryId === null) throw new Error('Context dictionary row is missing its dictionary id.');

  const nestedCorrections = Array.isArray(row.corrections)
    ? row.corrections
    : [row.correction ?? row.mapping].filter((candidate) => candidate != null);
  const corrections = nestedCorrections
    .map((correction) => normalizeCorrection(correction, dictionaryId, contextId))
    .filter((correction): correction is DictionaryCorrection => correction !== null);
  const flatCorrectionIds = Array.isArray(row.correction_ids)
    ? row.correction_ids.filter((id): id is number => finiteNumber(id) !== null)
    : [];
  const correctionId = firstFinite(
    row.correction_id,
    row.correctionId,
    row.mapping_id,
    row.mappingId,
    flatCorrectionIds[0],
    corrections[0]?.id,
  );
  const correctionIds = corrections.length > 0 ? corrections.map((correction) => correction.id) : flatCorrectionIds;
  const correctionMistake = corrections.map((correction) => correction.mistake).join(', ') || null;
  const autoLearned = booleanValue(row.auto_learned, corrections.some((correction) => correction.auto_learned));
  const correctionConfidence = corrections
    .filter((correction) => correction.auto_learned)
    .map((correction) => correction.confidence_tier)
    .find((tier) => tier === 'high')
    ?? corrections.filter((correction) => correction.auto_learned).map((correction) => correction.confidence_tier).find((tier) => tier === 'medium')
    ?? corrections.filter((correction) => correction.auto_learned).map((correction) => correction.confidence_tier).find((tier) => tier === 'low');
  const correctionCount = row.correction_count !== undefined
    ? nonNegativeCount(row.correction_count)
    : corrections.reduce((sum, correction) => sum + correction.correction_count, 0);
  const lastSeenAt = row.last_seen_at !== undefined
    ? nullableText(row.last_seen_at)
    : (() => {
        const dates = corrections
          .map((correction) => correction.last_seen_at)
          .filter((date): date is string => date !== null)
          .sort();
        return dates[dates.length - 1] ?? null;
      })();

  return {
    // Keep `id` canonical so existing Contexts/legacy selectors and assignment
    // commands continue to address the dictionary row, never the mapping row.
    id: dictionaryId,
    dictionary_id: dictionaryId,
    context_id: contextId,
    correction_id: correctionId,
    correction_ids: correctionIds,
    corrections,
    term: requiredText(row.term ?? row.canonical_term, 'term'),
    mistake: nullableText(row.mistake) ?? correctionMistake,
    auto_learned: autoLearned,
    correction_count: correctionCount,
    confidence_tier: confidenceTier(row.confidence_tier ?? correctionConfidence, autoLearned),
    last_seen_at: lastSeenAt,
    created_at: requiredText(row.created_at, 'created_at'),
  };
}

export function normalizeContextDictionary(value: unknown, contextId: number): ContextDictionaryEntry[] {
  if (!Array.isArray(value)) throw new Error('Context dictionary returned an invalid list.');
  return value.map((row) => normalizeContextDictionaryEntry(row, contextId));
}

export function dictionaryEntryId(entry: Pick<DictionaryEntry, 'id' | 'dictionary_id'>): number {
  return entry.dictionary_id ?? entry.id;
}

export function dictionaryEntryCorrectionIds(entry: Pick<DictionaryEntry, 'correction_id' | 'correction_ids' | 'corrections'>): number[] {
  if (entry.correction_ids?.length) return [...entry.correction_ids];
  if (entry.corrections?.length) return entry.corrections.map((correction) => correction.id);
  return entry.correction_id == null ? [] : [entry.correction_id];
}

export function dictionaryEntryCorrectionId(entry: Pick<DictionaryEntry, 'correction_id' | 'correction_ids' | 'corrections'>): number | null {
  return dictionaryEntryCorrectionIds(entry)[0] ?? null;
}

/**
 * `undefined` means the event has no scope and should refresh the selected
 * Context.  `null` is an explicit global/default update and also refreshes the
 * selected Context because it may change a canonical row it displays.
 */
export function dictionaryUpdateContextId(payload: unknown): number | null | undefined {
  const row = recordOf(payload);
  if (!row) return undefined;
  if ('context_id' in row) return finiteNumber(row.context_id);
  if ('contextId' in row) return finiteNumber(row.contextId);
  return undefined;
}

export function shouldRefreshContextDictionary(payload: unknown, contextId: number): boolean {
  const updateContextId = dictionaryUpdateContextId(payload);
  return updateContextId === undefined || updateContextId === null || updateContextId === contextId;
}

export async function editContextDictionaryEntry(
  entry: ContextDictionaryIdentity,
  contextId: number,
  term: string,
  mistake: string | null,
): Promise<void> {
  const dictionaryId = dictionaryEntryId(entry);
  await invoke('edit_dictionary_entry', {
    id: dictionaryId,
    dictionaryId,
    contextId,
    correctionId: dictionaryEntryCorrectionId(entry),
    correctionIds: dictionaryEntryCorrectionIds(entry),
    term,
    mistake,
  });
}

export async function removeContextDictionaryEntry(entry: ContextDictionaryIdentity, contextId: number): Promise<void> {
  const dictionaryId = dictionaryEntryId(entry);
  await invoke('remove_dictionary_entry', {
    id: dictionaryId,
    dictionaryId,
    contextId,
    correctionId: dictionaryEntryCorrectionId(entry),
    correctionIds: dictionaryEntryCorrectionIds(entry),
  });
}

/**
 * Moving is distinct from sharing: the backend transfers the Context-owned
 * mapping atomically, then removes the source assignment.  Sharing continues
 * to use `set_dictionary_context_assignment` and intentionally does not copy a
 * private correction mapping.
 */
export async function moveContextDictionaryEntry(
  entry: ContextDictionaryIdentity,
  sourceContextId: number,
  targetContextId: number,
): Promise<void> {
  const dictionaryId = dictionaryEntryId(entry);
  await invoke('move_dictionary_entry_to_context', {
    dictionaryId,
    sourceContextId,
    targetContextId,
    correctionId: dictionaryEntryCorrectionId(entry),
    correctionIds: dictionaryEntryCorrectionIds(entry),
  });
}
