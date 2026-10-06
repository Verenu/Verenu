import { beforeEach, describe, expect, it, vi } from 'vitest';
import { booleanSettingHandler, loadSettingsSnapshot } from '../../settings';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../../tauri', () => ({ invoke }));

describe('settings snapshot loading', () => {
  beforeEach(() => invoke.mockReset());

  it('reads one snapshot without replacing null or false values with UI defaults', async () => {
    const snapshot = { cleanup_enabled: false, analytics_enabled: false, mic_gain: null, hotkey: ['Ctrl', 'Super'] };
    invoke.mockResolvedValue(snapshot);
    expect(await loadSettingsSnapshot()).toEqual(snapshot);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('get_all_settings');
  });

  it('reads fresh values after a save and propagates backend failures', async () => {
    invoke.mockResolvedValueOnce({ noise_reduction: true }).mockResolvedValueOnce({ noise_reduction: false });
    expect((await loadSettingsSnapshot()).noise_reduction).toBe(true);
    expect((await loadSettingsSnapshot()).noise_reduction).toBe(false);
    invoke.mockRejectedValueOnce(new Error('settings unavailable'));
    await expect(loadSettingsSnapshot()).rejects.toThrow('settings unavailable');
    expect(invoke).toHaveBeenCalledTimes(3);
  });
});

describe('boolean setting saves', () => {
  it.each([true, false])('shows %s immediately, then rolls back and marks failure on rejection', async value => {
    const update = vi.fn(), failed = vi.fn();
    const error = new Error('settings unavailable');
    const log = vi.spyOn(console, 'error').mockImplementation(() => {});
    invoke.mockRejectedValueOnce(error);
    const pending = booleanSettingHandler('noise_reduction', update, failed)(value);
    expect(update).toHaveBeenNthCalledWith(1, value);
    await pending;
    expect(update).toHaveBeenNthCalledWith(2, !value);
    expect(failed).toHaveBeenCalledOnce();
    expect(log).toHaveBeenCalledWith('save noise_reduction failed:', error);
    log.mockRestore();
  });

  it('keeps the optimistic value when persistence succeeds', async () => {
    invoke.mockResolvedValueOnce(undefined);
    const update = vi.fn(), failed = vi.fn();
    await booleanSettingHandler('mute_audio', update, failed)(true);
    expect(update).toHaveBeenCalledExactlyOnceWith(true);
    expect(failed).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenLastCalledWith('save_setting', {key:'mute_audio',value:true});
  });
});
