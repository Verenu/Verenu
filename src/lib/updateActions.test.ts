import { beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ updateInfo: null as null | { version: string; assetName: string; downloadUrl: string; installMode: 'install' | 'download' }, updateInstalling: false, updateInstalled: false, updateProgress: '', updateInstallError: '' }));
const invoke = vi.hoisted(() => vi.fn());
const stop = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn(async () => stop));
vi.mock('./stores', () => ({ appStore: state }));
vi.mock('./tauri', () => ({ invoke, listen }));
import { installAvailableUpdate, progressLabel, reinstallLatestUpdate, updateActionLabel } from './updateActions';

describe('update actions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(state, { updateInfo: { version: '0.21.0', assetName: 'verenu-0.21.0-1-x86_64.pkg.tar.zst', downloadUrl: 'https://example.invalid/update', installMode: 'install' }, updateInstalling: false, updateInstalled: false, updateProgress: '', updateInstallError: '' });
    invoke.mockResolvedValue('installed');
  });
  it('labels Linux installation and download actions accurately', () => {
    expect(updateActionLabel(state.updateInfo!)).toBe('Install & Restart');
    expect(updateActionLabel({ ...state.updateInfo!, assetName: 'Verenu_0.21.0_amd64.AppImage', installMode: 'download' })).toBe('Download AppImage');
    expect(updateActionLabel({ ...state.updateInfo!, assetName: 'Verenu_0.21.0_x64-setup.exe' })).toBe('Install & Restart');
  });
  it('shares one install attempt and starts a restart after installation', async () => {
    let resolve!: (outcome: 'installed') => void;
    invoke.mockReturnValueOnce(new Promise<'installed'>((done) => { resolve = done; }));
    const first = installAvailableUpdate();
    await installAvailableUpdate();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(state.updateInstalling).toBe(true);
    resolve('installed');
    await first;
    expect(state.updateInstalling).toBe(false);
    expect(state.updateInstalled).toBe(true);
    expect(state.updateProgress).toContain('Restarting');
    expect(stop).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenLastCalledWith('restart_app');
    expect(invoke).toHaveBeenCalledTimes(2);
  });
  it('keeps the installed update and offers restart retry after startup fails', async () => {
    invoke.mockResolvedValueOnce('installed').mockRejectedValueOnce(new Error('Replacement startup failed'));
    await expect(installAvailableUpdate()).rejects.toThrow('startup failed');
    expect(state.updateInstalled).toBe(true);
    expect(state.updateProgress).toContain('Quit and reopen');
    expect(state.updateInstallError).toContain('startup failed');
    await installAvailableUpdate();
    expect(invoke).toHaveBeenCalledTimes(3);
    expect(invoke).toHaveBeenLastCalledWith('restart_app');
  });
  it('a cancelled or failed installation permits retry and never says installed', async () => {
    invoke.mockRejectedValueOnce(new Error('Update authorization was cancelled.'));
    await expect(installAvailableUpdate()).rejects.toThrow('cancelled');
    expect(state.updateInstalled).toBe(false);
    expect(state.updateInstalling).toBe(false);
    expect(state.updateProgress).toBe('');
    expect(state.updateInstallError).toContain('cancelled');
    expect(stop).toHaveBeenCalledTimes(1);
    await installAvailableUpdate();
    expect(state.updateInstalled).toBe(true);
    expect(state.updateInstallError).toBe('');
  });
  it('opening a download does not claim installation', async () => {
    state.updateInfo!.installMode = 'download';
    invoke.mockResolvedValue('downloadOpened');
    await installAvailableUpdate();
    expect(state.updateInstalled).toBe(false);
    expect(state.updateProgress).toBe('');
  });
  it('Android installer handoff never claims success and allows cancellation retry', async () => {
    state.updateInfo!.assetName = 'Verenu_0.21.0_android_arm64-v8a.apk';
    expect(updateActionLabel(state.updateInfo!)).toBe('Update Verenu');
    invoke.mockResolvedValue('installerOpened');
    await installAvailableUpdate();
    expect(state.updateInstalled).toBe(false);
    expect(state.updateInstalling).toBe(false);
    expect(state.updateProgress).toBe('Approve the update in the Android installer.');
    await installAvailableUpdate();
    expect(invoke).toHaveBeenCalledTimes(2);
  });
  it('uses the native outcome when an installable offer became a manual download', async () => {
    invoke.mockResolvedValue('downloadOpened');

    await installAvailableUpdate();

    expect(state.updateInstalled).toBe(false);
    expect(state.updateProgress).toBe('');
  });
  it('restarts after a completed reinstall and retries restart without reinstalling', async () => {
    const reinstall = { busy: false, installedVersion: null as string | null, message: '' };
    invoke.mockResolvedValueOnce({ version: '0.21.0', outcome: 'installed' })
      .mockRejectedValueOnce(new Error('Replacement startup failed'));
    await reinstallLatestUpdate(reinstall);
    expect(reinstall.installedVersion).toBe('0.21.0');
    expect(reinstall.message).toContain('Restart failed');
    expect(reinstall.message).toContain('quit and reopen');
    await reinstallLatestUpdate(reinstall);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      'reinstall_latest_update', 'restart_app', 'restart_app',
    ]);
    expect(reinstall.busy).toBe(false);
  });
  it('does not restart when reinstall opens a download', async () => {
    const reinstall = { busy: false, installedVersion: null as string | null, message: '' };
    invoke.mockResolvedValueOnce({ version: '0.21.0', outcome: 'downloadOpened' });
    await reinstallLatestUpdate(reinstall);
    expect(reinstall.message).toContain('Download opened');
    expect(reinstall.installedVersion).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it('leaves Android installation to the installer', async () => {
    const reinstall = { busy: false, installedVersion: null as string | null, message: '' };
    invoke.mockResolvedValueOnce({ version: '0.21.0', outcome: 'installerOpened' });
    await reinstallLatestUpdate(reinstall);
    expect(reinstall.message).toContain('Android installer');
    expect(reinstall.installedVersion).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it('prevents overlapping reinstalls', async () => {
    const reinstall = { busy: false, installedVersion: null as string | null, message: '' };
    let finish!: (result: { version: string; outcome: 'downloadOpened' }) => void;
    invoke.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const first = reinstallLatestUpdate(reinstall);
    await reinstallLatestUpdate(reinstall);
    expect(invoke).toHaveBeenCalledTimes(1);
    finish({ version: '0.21.0', outcome: 'downloadOpened' });
    await first;
    expect(reinstall.busy).toBe(false);
  });
  it('shows bounded progress even without a content length', () => {
    expect(progressLabel({ phase: 'downloading', downloaded: 20, total: 10 })).toBe('Downloading… 100%');
    expect(progressLabel({ phase: 'downloading', downloaded: 1048576, total: null })).toBe('Downloading… 1.0 MB');
    expect(progressLabel({ phase: 'authorizing', downloaded: 0, total: null })).toContain('authorization');
    expect(progressLabel({ phase: 'verifying', downloaded: 0, total: null })).toContain('Verifying');
  });
});
