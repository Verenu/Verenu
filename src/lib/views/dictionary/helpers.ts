export type SortKey = 'newest' | 'oldest' | 'alpha' | 'most_corrected';
export type CreatedRecordMeta = {
  /** Canonical dictionary id; `id` remains the legacy response field. */
  id: number;
  dictionary_id?: number;
  correction_id?: number | null;
  context_id?: number | null;
  created_at: string;
};

export const TERM_LIMIT = 120;
export const MISTAKE_LIMIT = 120;

export const sortLabels: { key: SortKey; label: string }[] = [
  { key: 'newest', label: 'Newest' },
  { key: 'oldest', label: 'Oldest' },
  { key: 'alpha', label: 'A → Z' },
  { key: 'most_corrected', label: 'Most corrected' },
];

export function fmtDate(iso: string): string {
  try {
    const MS_PER_DAY = 86_400_000;
    const d = new Date(/[Z+]/.test(iso) ? iso : iso + 'Z');
    const diffDays = Math.floor((Date.now() - d.getTime()) / MS_PER_DAY);
    if (diffDays === 0) return 'Today';
    if (diffDays === 1) return 'Yesterday';
    if (diffDays < 7) return `${diffDays}d ago`;
    return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch {
    return iso.slice(0, 10);
  }
}

export function confidenceLabel(tier?: string | null): string {
  if (tier === 'high') return 'High confidence';
  if (tier === 'medium') return 'Medium confidence';
  if (tier === 'low') return 'Low confidence';
  if (tier === 'manual') return 'Manual';
  return 'Unknown confidence';
}

export const countCodePoints = (value: string): number => [...value].length;

export function requireCreatedRecordMeta(value: unknown): CreatedRecordMeta {
  if (typeof value !== 'object' || value === null) {
    throw new Error('Save returned no record metadata. Relaunch the Tauri app and try again.');
  }
  const meta = value as Partial<CreatedRecordMeta>;
  const canonicalId = typeof meta.dictionary_id === 'number' && Number.isFinite(meta.dictionary_id)
    ? meta.dictionary_id
    : meta.id;
  if (typeof canonicalId !== 'number' || !Number.isFinite(canonicalId) || typeof meta.created_at !== 'string' || !meta.created_at.trim()) {
    throw new Error('Save returned invalid record metadata. Check the app logs before retrying.');
  }
  return {
    id: canonicalId,
    dictionary_id: canonicalId,
    correction_id: typeof meta.correction_id === 'number' && Number.isFinite(meta.correction_id)
      ? meta.correction_id
      : null,
    context_id: typeof meta.context_id === 'number' && Number.isFinite(meta.context_id)
      ? meta.context_id
      : meta.context_id === null ? null : undefined,
    created_at: meta.created_at,
  };
}
