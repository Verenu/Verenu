export type SortKey = 'newest' | 'oldest' | 'alpha' | 'most_used';
export type CreatedRecordMeta = { id: number; created_at: string };

export const TRIGGER_LIMIT = 300;
// Cap auto-grow so a long paste can't blow out the modal layout; the textarea
// scrolls internally past this height.
export const FIELD_GROW_MAX = 220;

export const sortLabels: { key: SortKey; label: string }[] = [
  { key: 'newest', label: 'Newest' },
  { key: 'oldest', label: 'Oldest' },
  { key: 'alpha', label: 'A → Z' },
  { key: 'most_used', label: 'Most used' },
];

export { fmtDate, countCodePoints } from '../sharedListHelpers';

export function normalizeText(value: string): string {
  return value.replace(/\r\n/g, '\n').replace(/\r/g, '\n').trim();
}

export function requireCreatedRecordMeta(value: unknown): CreatedRecordMeta {
  if (typeof value !== 'object' || value === null) {
    throw new Error('Verenu could not confirm that this snippet was saved. Refresh the list before trying again.');
  }
  const meta = value as Partial<CreatedRecordMeta>;
  if (typeof meta.id !== 'number' || !Number.isFinite(meta.id) || typeof meta.created_at !== 'string' || !meta.created_at.trim()) {
    throw new Error('Snippet save returned invalid record metadata. Check the app logs before retrying.');
  }
  return { id: meta.id, created_at: meta.created_at };
}

export function autoGrow(el: HTMLTextAreaElement | null) {
  if (!el) return;
  el.style.height = 'auto';
  const borderDiff = el.offsetHeight - el.clientHeight;
  el.style.height = Math.min(el.scrollHeight + borderDiff, FIELD_GROW_MAX) + 'px';
}
