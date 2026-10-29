import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('./tauri', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { setSkillsExcluded, t3State, updateT3 } from './t3Store.svelte';

const status = (excludedSkills: string[]) => ({
  minimumVersion: '0.46',
  skills: [{ name: 'a' }, { name: 'b' }],
  connection: { id: 'c', label: 'T3', version: '1', expiresAt: 1, fetchedAt: 1, selectedCatalog: null, catalogs: [], error: null, excludedSkills },
});

describe('T3 skill exclusions', () => {
  beforeEach(() => {
    invoke.mockReset();
    t3State.status = status([]);
    t3State.error = '';
  });

  it('keeps a pending selection when an older pull result arrives', async () => {
    let finishPull!: (value: unknown) => void;
    invoke.mockImplementation((command: string) =>
      command === 'pull_t3_skills' ? new Promise((resolve) => (finishPull = resolve)) : Promise.resolve(status(['a'])),
    );
    const pull = updateT3('pull_t3_skills', { force: true });
    const saved = setSkillsExcluded(['a'], true);
    finishPull(status([]));
    await pull;
    expect(t3State.status?.connection?.excludedSkills).toEqual(['a']);
    await saved;
  });

  it('restores the saved selection but keeps the error when saving fails', async () => {
    invoke.mockImplementation((command: string) =>
      command === 'set_t3_excluded_skills' ? Promise.reject(new Error('disk full')) : Promise.resolve(status([])),
    );
    await setSkillsExcluded(['a'], true);
    expect(t3State.status?.connection?.excludedSkills).toEqual([]);
    expect(t3State.error).not.toBe('');
  });
});
