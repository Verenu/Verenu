import { afterEach, expect, it, vi } from 'vitest';
import { devInvoke } from './tauri.dev';

afterEach(() => vi.unstubAllGlobals());

it('reports NVIDIA RE-USE as unavailable in the browser preview', async () => {
  expect(await devInvoke('nvidia_reuse_status')).toEqual({ state: 'unavailable', message: 'NVIDIA RE-USE is available in the desktop app.', has_files: false });
  await expect(devInvoke('nvidia_reuse_install')).rejects.toThrow('requires the desktop app');
});

function browserStorage() {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
}

it('keeps a first group-only word out of Everywhere and other groups', async () => {
  browserStorage();
  await devInvoke('create_dictionary_entry', { term: 'LocalOnly', mistake: 'local-only', contextId: 2 });
  expect(await devInvoke('get_context_dictionary', { contextId: 1 })).toEqual([]);
  expect(await devInvoke('get_context_dictionary', { contextId: 3 })).toEqual([]);
  expect(await devInvoke('get_context_dictionary', { contextId: 2 })).toMatchObject([{ term: 'LocalOnly', assigned_to_context: true }]);
});

it('persists an inherited word customization locally and preserves its global spelling', async () => {
  browserStorage();
  const created = await devInvoke<{ id: number }>('create_dictionary_entry', { term: 'GlobalWord', mistake: 'global-word', contextId: 1 });
  await expect(devInvoke('edit_dictionary_entry', { id: created.id, term: 'Renamed', contextId: 2 })).rejects.toThrow('Rename inherited terms in Everywhere');
  await devInvoke('edit_dictionary_entry', { id: created.id, term: 'GlobalWord', mistake: 'local-word', contextId: 2, expected: { term: 'GlobalWord', mistake: null } });
  await expect(devInvoke('edit_dictionary_entry', { id: created.id, term: 'GlobalWord', mistake: 'stale', contextId: 2, expected: { term: 'GlobalWord', mistake: null } })).rejects.toThrow('changed while you were editing');
  expect(await devInvoke('get_context_dictionary', { contextId: 2 })).toMatchObject([{
    term: 'GlobalWord', assigned_to_context: true, inherited_from_everywhere: true,
    context_corrections: [{ mistake: 'local-word', context_id: 2 }],
  }]);
  expect(await devInvoke('get_context_dictionary', { contextId: 1 })).toMatchObject([{ term: 'GlobalWord', mistake: 'global-word' }]);
});

it('rejects stale vocabulary and snippet edits without losing newer changes', async () => {
  browserStorage();
  const word = await devInvoke<{ id: number }>('create_dictionary_entry', { term: 'Word', mistake: 'old', contextId: 2 });
  await devInvoke('edit_dictionary_entry', { id: word.id, term: 'Word', mistake: 'new', contextId: 2 });
  await expect(devInvoke('edit_dictionary_entry', { id: word.id, term: 'Renamed', mistake: 'old', contextId: 2, expected: { term: 'Word', mistake: 'old' } })).rejects.toThrow('changed while you were editing');
  expect(await devInvoke('get_context_dictionary', { contextId: 2 })).toMatchObject([{ term: 'Word', mistake: 'new' }]);

  const snippet = await devInvoke<{ id: number }>('create_snippet', { trigger: 'sig', expansion: 'old', instructions: '' });
  await devInvoke('edit_snippet', { id: snippet.id, trigger: 'sig', expansion: 'new', instructions: '' });
  await expect(devInvoke('edit_snippet', { id: snippet.id, trigger: 'signature', expansion: 'old', instructions: '', expected: { trigger: 'sig', expansion: 'old', instructions: '' } })).rejects.toThrow('changed while you were editing');
  expect(await devInvoke('get_snippets')).toMatchObject([{ trigger: 'sig', expansion: 'new' }]);
});

it('preserves Android pill defaults in individual and full browser settings reads', async () => {
  vi.stubGlobal('localStorage', { getItem: () => null });
  expect(await devInvoke('get_setting', { key: 'android_pill_position' })).toBe('keyboard-center');
  expect(await devInvoke('get_setting', { key: 'android_pill_dock_position' })).toBe('screen-bottom');
  expect(await devInvoke('get_setting', { key: 'android_pill_cover_keyboard_mic' })).toBe(false);
  expect(await devInvoke('get_setting', { key: 'android_pill_hide_offline' })).toBe(true);
  expect(await devInvoke('get_all_settings')).toMatchObject({
    android_pill_position: 'keyboard-center',
    android_pill_dock_position: 'screen-bottom',
    android_pill_cover_keyboard_mic: false,
    android_pill_hide_offline: true,
  });
});

it.each([null, '{invalid', 'null', '[]', 'false', '42', '"model"'])('keeps model defaults for missing or malformed storage %s', async (raw) => {
  vi.stubGlobal('localStorage', { getItem: () => raw });
  const defaults = {
    current_model_id: null, is_loaded: false, is_loading: false,
    is_downloading: false, downloading_model_id: null,
  };
  expect(await devInvoke('get_local_transcription_state')).toEqual(defaults);
  expect(await devInvoke('get_local_llm_state')).toEqual({ ...defaults, endpoint: null });
  expect(await devInvoke('get_local_llm_runtime_info')).toMatchObject({ installed: false, is_downloading: false });
});

it('preserves stored model flags, string fields, and download manifests', async () => {
  const saved = {
    current_model_id: 'model', is_loaded: 1, is_loading: '', is_downloading: true,
    downloading_model_id: 123, endpoint: 'http://localhost:7777',
    installed: true, 'parakeet-v3': { downloaded: true, partial_size: 1234 },
  };
  vi.stubGlobal('localStorage', { getItem: () => JSON.stringify(saved) });
  const expected = {
    current_model_id: 'model', is_loaded: true, is_loading: false,
    is_downloading: true, downloading_model_id: null,
  };
  expect(await devInvoke('get_local_transcription_state')).toEqual(expected);
  expect(await devInvoke('get_local_llm_state')).toEqual({ ...expected, endpoint: saved.endpoint });
  expect(await devInvoke('list_local_stt_models')).toEqual(expect.arrayContaining([
    expect.objectContaining({ id: 'parakeet-v3', is_downloaded: true, partial_size: 1234 }),
  ]));
  expect(await devInvoke('get_local_llm_runtime_info')).toMatchObject({ installed: true, is_downloading: true });
});

it('keeps browser model writes nonfatal when persistent storage is blocked', async () => {
  vi.stubGlobal('localStorage', { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } });
  await expect(devInvoke('delete_local_llm_runtime')).resolves.toBeUndefined();
  await expect(devInvoke('delete_local_stt_model', { modelId: 'parakeet-v3' })).resolves.toBeUndefined();
});
