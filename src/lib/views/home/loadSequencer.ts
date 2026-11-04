export interface LoadSequencerDeps<R> {
  /** Starts a history request. Read mutable state synchronously, before the first await. */
  fetchPage(reset: boolean): Promise<R>;
  /** Refreshes stats independently of history. Failures must keep the last good stats. */
  refreshStats(): Promise<void>;
  onSuccess(result: R, reset: boolean): void;
  onFailure(err: unknown, reset: boolean): void;
  onSettled(reset: boolean): void;
}

export interface LoadSequencer {
  load(reset?: boolean, withStats?: boolean): Promise<void>;
  /** Drops the in-flight history load. Returns it so the caller can restart a reset. */
  interrupt(): { reset: boolean } | null;
}

export function createLoadSequencer<R>(deps: LoadSequencerDeps<R>): LoadSequencer {
  let seq = 0;
  let active: { seq: number; reset: boolean } | null = null;

  async function load(reset = true, withStats = false): Promise<void> {
    const mine = ++seq;
    active = { seq: mine, reset };
    // Stats have their own latest-wins gate, so a later history-only load cannot discard them.
    if (withStats) void deps.refreshStats();
    try {
      const result = await deps.fetchPage(reset);
      if (mine !== seq) return;
      active = null;
      deps.onSuccess(result, reset);
    } catch (err) {
      if (mine !== seq) return;
      active = null;
      deps.onFailure(err, reset);
    }
    deps.onSettled(reset);
  }

  function interrupt() {
    if (!active) return null;
    const interrupted = active;
    seq++;
    active = null;
    return { reset: interrupted.reset };
  }

  return { load, interrupt };
}
