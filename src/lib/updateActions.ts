import { appStore, type UpdateInfo } from './stores';
import { invoke, listen } from './tauri';
import { formatIpcError } from './errors';

export interface UpdateProgress {
  phase: string;
  downloaded: number;
  total: number | null;
}

type InstallOutcome = 'installed' | 'downloadOpened' | 'installerOpened';

export interface ReinstallState {
  busy: boolean;
  installedVersion: string | null;
  message: string;
}

export async function reinstallLatestUpdate(state: ReinstallState): Promise<void> {
  if (state.busy) return;
  state.busy = true;
  try {
    if (!state.installedVersion) {
      state.message = 'Preparing reinstall…';
      const result = await invoke<{ version: string; outcome: InstallOutcome }>('reinstall_latest_update');
      if (result.outcome === 'downloadOpened') {
        state.message = `Download opened for v${result.version}.`;
        return;
      }
      if (result.outcome === 'installerOpened') {
        state.message = 'Approve the update in the Android installer.';
        return;
      }
      if (result.outcome !== 'installed') throw new Error('The updater returned an unexpected result.');
      state.installedVersion = result.version;
    }
    state.message = 'Restarting Verenu…';
    await invoke('restart_app');
  } catch (error) {
    state.message = state.installedVersion
      ? `v${state.installedVersion} installed. Restart failed: ${formatIpcError(error, 'Could not restart Verenu')} Try again or quit and reopen Verenu.`
      : `Reinstall failed: ${formatIpcError(error, 'Could not install the update')}`;
  } finally {
    state.busy = false;
  }
}

export function progressLabel(progress: UpdateProgress): string {
  switch (progress.phase) {
    case 'resolving': return 'Checking release…';
    case 'downloading': {
      const downloaded = Math.max(0, progress.downloaded);
      return progress.total && progress.total > 0
        ? `Downloading… ${Math.min(100, Math.floor(downloaded / progress.total * 100))}%`
        : `Downloading… ${(downloaded / 1024 / 1024).toFixed(1)} MB`;
    }
    case 'verifying': return 'Verifying download…';
    case 'backing-up': return 'Backing up data…';
    case 'authorizing': return 'Waiting for administrator authorization…';
    case 'installing': return 'Installing…';
    case 'complete': return 'Update installed. Restart Verenu to finish.';
    default: return 'Updating…';
  }
}

export function updateActionLabel(update: UpdateInfo, restart = true): string {
  if (update.installMode === 'download') {
    if (update.assetName.toLowerCase().endsWith('.dmg')) return 'Download DMG';
    if (update.assetName.toLowerCase().endsWith('.appimage')) return 'Download AppImage';
    return 'Download Installer';
  }
  if (isLinuxInstaller(update)) return 'Install & Restart';
  if (update.assetName.endsWith('.apk')) return 'Update Verenu';
  return restart ? 'Install & Restart' : 'Install Now';
}

function isLinuxInstaller(update: UpdateInfo): boolean {
  const name = update.assetName.toLowerCase();
  return name.endsWith('.appimage') || name.endsWith('.pkg.tar.zst');
}

async function restartInstalledUpdate(): Promise<void> {
  appStore.updateProgress = 'Restarting Verenu…';
  try {
    await invoke('restart_app');
  } catch (error) {
    appStore.updateProgress = 'Update installed. Quit and reopen Verenu to finish.';
    appStore.updateInstallError = formatIpcError(error, 'Could not restart Verenu');
    throw error;
  }
}

export async function installAvailableUpdate(): Promise<void> {
  const update = appStore.updateInfo;
  if (!update || appStore.updateInstalling) return;
  appStore.updateInstalling = true;
  appStore.updateInstallError = '';
  if (appStore.updateInstalled) {
    try {
      await restartInstalledUpdate();
    } finally {
      appStore.updateInstalling = false;
    }
    return;
  }
  appStore.updateProgress = update.installMode === 'download' ? 'Opening download…' : 'Preparing update…';
  let stop: (() => void) | undefined;
  try {
    stop = await listen<UpdateProgress>('verenu:update-progress', ({ payload }) => {
      appStore.updateProgress = progressLabel(payload);
    });
    const outcome = await invoke<InstallOutcome>('install_update', { downloadUrl: update.downloadUrl });
    if (outcome === 'installed') {
      appStore.updateInstalled = true;
      await restartInstalledUpdate();
    } else if (outcome === 'downloadOpened') {
      appStore.updateProgress = '';
    } else if (outcome === 'installerOpened') {
      appStore.updateProgress = 'Approve the update in the Android installer.';
    } else {
      throw new Error('The updater returned an unexpected result. Check for updates and try again.');
    }
  } catch (error) {
    if (!appStore.updateInstalled) {
      appStore.updateProgress = '';
      appStore.updateInstallError = formatIpcError(error, 'Could not install the update');
    }
    throw error;
  } finally {
    stop?.();
    appStore.updateInstalling = false;
  }
}
