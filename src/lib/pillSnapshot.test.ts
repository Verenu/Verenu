import { describe, expect, it } from 'vitest';
import { createPillSnapshotReceiver, type PillSnapshot } from './pillSnapshot';

const initial: PillSnapshot = {
  state: '', context: null, message: null, stage: null,
  revision: 0, state_revision: 0, context_revision: 0,
  message_revision: 0, stage_revision: 0,
};

describe('pill snapshot delivery', () => {
  it('uses the newest complete state when an older replay arrives later', () => {
    const events: string[] = [];
    const receive = createPillSnapshotReceiver({
      state: (value) => events.push(`state:${value}`),
      context: (value) => events.push(`context:${value}`),
      message: (value) => events.push(`message:${value}`),
      stage: (value) => events.push(`stage:${value}`),
    });
    receive({ ...initial, state: 'processing', stage: 'cleaning', revision: 2, state_revision: 1, stage_revision: 2 });
    receive({ ...initial, revision: 0 });
    expect(events).toEqual(['message:null', 'state:processing', 'context:null', 'stage:cleaning']);
  });

  it('repeats state transitions but not for unrelated stage or context updates', () => {
    const states: string[] = [];
    const receive = createPillSnapshotReceiver({
      state: (value) => states.push(value), context: () => {}, message: () => {}, stage: () => {},
    });
    const first = { ...initial, state: 'processing', revision: 1, state_revision: 1 };
    receive(first);
    receive({ ...first, context: 'Synthetic', revision: 2, context_revision: 2 });
    receive({ ...first, context: 'Synthetic', stage: 'cleaning', revision: 3, context_revision: 2, stage_revision: 3 });
    receive({ ...first, context: 'Synthetic', stage: 'cleaning', revision: 4, state_revision: 4, context_revision: 2, stage_revision: 3 });
    expect(states).toEqual(['processing', 'processing']);
  });

  it('rejects malformed and stale snapshots without affecting later valid delivery', () => {
    const states: string[] = [];
    const receive = createPillSnapshotReceiver({ state: (value) => states.push(value), context: () => {}, message: () => {}, stage: () => {} });
    receive({ ...initial, revision: 10, state_revision: 11 });
    receive({ ...initial, state: 'recording', revision: 1, state_revision: 1 });
    receive(initial);
    expect(states).toEqual(['recording']);
  });

  it('clears old messages and stages when intermediate transitions were missed', () => {
    const messages: (string | null)[] = [];
    const stages: (string | null)[] = [];
    const receive = createPillSnapshotReceiver({
      state: () => {}, context: () => {},
      message: (value) => messages.push(value), stage: (value) => stages.push(value),
    });
    receive({ ...initial, state: 'processing', message: 'Old', stage: 'cleaning', revision: 2, state_revision: 1, message_revision: 1, stage_revision: 2 });
    receive({ ...initial, state: 'processing', revision: 4, state_revision: 4, message_revision: 3, stage_revision: 3 });
    expect(messages).toEqual(['Old', null]);
    expect(stages).toEqual(['cleaning', null]);
  });
});
