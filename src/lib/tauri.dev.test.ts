import { afterEach, expect, it, vi } from 'vitest';
import { devInvoke } from './tauri.dev';

afterEach(() => vi.unstubAllGlobals());

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
