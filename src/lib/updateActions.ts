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
  message: string;
}

export async function reinstallLatestUpdate(state: ReinstallState): Promise<void> {
  if (state.busy) return;
  state.busy = true;
  try {
    state.message = 'Preparing reinstall…';
    const result = await invoke<{ version: string; outcome: InstallOutcome }>('reinstall_latest_update');
    if (result.outcome === 'downloadOpened') {
      state.message = `Download opened for v${result.version}.`;
    } else if (result.outcome === 'installerOpened') {
      state.message = 'Approve the update in the Android installer.';
    } else if (result.outcome === 'installed') {
      state.message = `v${result.version} installed. Quit and reopen Verenu to finish.`;
    } else {
      throw new Error('The updater returned an unexpected result.');
    }
  } catch (error) {
    state.message = `Reinstall failed: ${formatIpcError(error, 'Could not install the update')}`;
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
    case 'complete': return 'Update installed. Quit and reopen Verenu to finish.';
    default: return 'Updating…';
  }
}

export function updateActionLabel(update: UpdateInfo, restart = true): string {
  if (update.installMode === 'download') {
    if (update.assetName.toLowerCase().endsWith('.dmg')) return 'Download DMG';
    if (update.assetName.toLowerCase().endsWith('.appimage')) return 'Download AppImage';
    return 'Download Installer';
  }
  if (isLinuxInstaller(update)) return 'Update Verenu';
  if (update.assetName.endsWith('.apk')) return 'Update Verenu';
  return restart ? 'Install & Restart' : 'Install Now';
}

function isLinuxInstaller(update: UpdateInfo): boolean {
  const name = update.assetName.toLowerCase();
  return name.endsWith('.appimage') || name.endsWith('.pkg.tar.zst');
}

export async function installAvailableUpdate(): Promise<void> {
  const update = appStore.updateInfo;
  if (!update || appStore.updateInstalling || appStore.updateInstalled) return;
  appStore.updateInstalling = true;
  appStore.updateInstallError = '';
  appStore.updateProgress = update.installMode === 'download' ? 'Opening download…' : 'Preparing update…';
  let stop: (() => void) | undefined;
  try {
    stop = await listen<UpdateProgress>('verenu:update-progress', ({ payload }) => {
      appStore.updateProgress = progressLabel(payload);
    });
    const outcome = await invoke<InstallOutcome>('install_update', { downloadUrl: update.downloadUrl });
    if (outcome === 'installed') {
      appStore.updateInstalled = true;
      appStore.updateProgress = 'Update installed. Quit and reopen Verenu to finish.';
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
