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

export function countCodePoints(value: string): number {
  let count = 0;
  for (const _ of value) count++;
  return count;
}
