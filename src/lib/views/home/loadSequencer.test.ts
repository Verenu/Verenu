import { describe, expect, it } from 'vitest';
import { createLatestGate, createLatestRefresher } from './latestRequest';
import { createLoadSequencer } from './loadSequencer';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

type Stats = { words: number };

function setup() {
  const pages: ReturnType<typeof deferred<string[]>>[] = [];
  const statsCalls: ReturnType<typeof deferred<Stats>>[] = [];
  const state = { stats: { words: 0 } as Stats, rows: [] as string[], error: '', settled: 0 };
  const refreshStats = createLatestRefresher(
    () => { const d = deferred<Stats>(); statsCalls.push(d); return d.promise; },
    (next) => { state.stats = next; },
    createLatestGate(),
  );
  const sequencer = createLoadSequencer<string[]>({
    fetchPage: () => { const d = deferred<string[]>(); pages.push(d); return d.promise; },
    refreshStats,
    onSuccess: (rows) => { state.rows = rows; state.error = ''; },
    onFailure: (err, reset) => { if (reset) { state.error = String(err); state.rows = []; } },
    onSettled: () => { state.settled++; },
  });
  return { pages, statsCalls, state, sequencer };
}

describe('load sequencer', () => {
  it('applies stats from a stats-bearing load even when a history-only load supersedes its page', async () => {
    const { pages, statsCalls, state, sequencer } = setup();
    const withStats = sequencer.load(true, true);
    const searching = sequencer.load(true, false);

    statsCalls[0].resolve({ words: 42 });
    pages[0].resolve(['stale']);
    pages[1].resolve(['search-hit']);
    await Promise.all([withStats, searching]);
    await Promise.resolve();

    expect(state.stats).toEqual({ words: 42 });
    expect(state.rows).toEqual(['search-hit']);
  });

  it('keeps only the newest stats when a delete refresh overlaps a stats-bearing load', async () => {
    const { pages, statsCalls, state, sequencer } = setup();
    const first = sequencer.load(true, true);
    const second = sequencer.load(true, true);
    statsCalls[1].resolve({ words: 2 });
    await Promise.resolve();
    statsCalls[0].resolve({ words: 1 });
    pages[0].resolve([]);
    pages[1].resolve([]);
    await Promise.all([first, second]);
    await Promise.resolve();
    expect(state.stats).toEqual({ words: 2 });
  });

  it('preserves the last good stats when a reset history load fails', async () => {
    const { pages, statsCalls, state, sequencer } = setup();
    const ok = sequencer.load(true, true);
    statsCalls[0].resolve({ words: 7 });
    pages[0].resolve(['a']);
    await ok;
    await Promise.resolve();

    const failing = sequencer.load(true, false);
    pages[1].reject(new Error('db locked'));
    await failing;

    expect(state.stats).toEqual({ words: 7 });
    expect(state.error).toContain('db locked');
    expect(state.rows).toEqual([]);
  });

  it('does not let a failed stats fetch erase stats or fail the history load', async () => {
    const { pages, statsCalls, state, sequencer } = setup();
    const run = sequencer.load(true, true);
    statsCalls[0].reject(new Error('stats down'));
    pages[0].resolve(['a']);
    await run;
    expect(state.stats).toEqual({ words: 0 });
    expect(state.rows).toEqual(['a']);
    expect(state.error).toBe('');
  });

  it('interrupt drops the in-flight page and reports whether it was a reset', async () => {
    const { pages, state, sequencer } = setup();
    const run = sequencer.load(true);
    expect(sequencer.interrupt()).toEqual({ reset: true });
    expect(sequencer.interrupt()).toBeNull();
    pages[0].resolve(['deleted-row']);
    await run;
    expect(state.rows).toEqual([]);
    expect(state.settled).toBe(0);
  });
});
