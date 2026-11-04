export interface LatestGate {
  next(): number;
  isLatest(stamp: number): boolean;
}

export function createLatestGate(): LatestGate {
  let current = 0;
  return {
    next: () => ++current,
    isLatest: (stamp) => stamp === current,
  };
}

export function createLatestRefresher<T>(
  fetch: () => Promise<T>,
  apply: (value: T) => void,
  gate: LatestGate = createLatestGate(),
): () => Promise<void> {
  return async () => {
    const stamp = gate.next();
    try {
      const value = await fetch();
      if (gate.isLatest(stamp)) apply(value);
    } catch {
      // A failed refresh keeps the last good value.
    }
  };
}
