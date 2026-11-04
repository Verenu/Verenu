export interface VirtualWindowState<T> {
  start: number;
  end: number;
  items: readonly T[] | null;
  totalHeight: number;
}

/**
 * The rendered slice may only be reused when the index range, the backing item
 * list, and the measured height are all unchanged. Comparing indices alone
 * keeps a deleted row on screen, because removing an item in the rendered range
 * leaves the same start/end pair.
 */
export function virtualWindowUnchanged<T>(
  prev: VirtualWindowState<T>,
  next: VirtualWindowState<T>
): boolean {
  return (
    prev.start === next.start &&
    prev.end === next.end &&
    prev.items === next.items &&
    prev.totalHeight === next.totalHeight
  );
}
