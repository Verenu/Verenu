import { afterEach, expect, it, vi } from 'vitest';
import { devInvoke } from './tauri.dev';

afterEach(() => vi.unstubAllGlobals());

it('preserves Android pill defaults in individual and full browser settings reads', async () => {
  vi.stubGlobal('localStorage', { getItem: () => null });
  expect(await devInvoke('get_setting', { key: 'android_pill_position' })).toBe('keyboard-center');
  expect(await devInvoke('get_setting', { key: 'android_pill_cover_keyboard_mic' })).toBe(false);
  expect(await devInvoke('get_setting', { key: 'android_pill_hide_offline' })).toBe(true);
  expect(await devInvoke('get_all_settings')).toMatchObject({
    android_pill_position: 'keyboard-center',
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
