import type { UpdateInfo } from './stores';
import { appStore } from './stores';
import { saveSetting } from './settings';
import { invoke } from './tauri';
import { ensureNotificationPermission } from './notifications';

const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
let automaticCheckInFlight = false;

async function checkForAutomaticUpdates(active: () => boolean): Promise<void> {
  if (automaticCheckInFlight || appStore.updateInstalling || appStore.updateInstalled) return;
  automaticCheckInFlight = true;
  const channel = appStore.betaUpdatesEnabled;
  try {
  let update: UpdateInfo | null = null;

  try {
    update = await invoke<UpdateInfo | null>('check_for_update');
  } catch (error) {
    console.warn('Automatic update check failed:', error);
    return;
  }

  if (!active() || channel !== appStore.betaUpdatesEnabled || appStore.updateInstalling || appStore.updateInstalled || !update) return;

  let dismissedVersion: string | null = null;
  let notifiedVersion: string | null = null;
  try {
    [dismissedVersion, notifiedVersion] = await Promise.all([
      invoke<string | null>('get_setting', { key: 'update_dismissed_version' }).catch((error) => {
        console.warn('Failed to look up dismissed update version:', error);
        return null;
      }),
      invoke<string | null>('get_setting', { key: 'update_notified_version' }).catch((error) => {
        console.warn('Failed to look up notified update version:', error);
        return null;
      }),
    ]);
  } catch (error) {
    console.warn('Update state lookup failed:', error);
  }
  if (
    !active()
    || channel !== appStore.betaUpdatesEnabled
    || appStore.updateInstalling
    || appStore.updateInstalled
  ) return;

  if (dismissedVersion === update.version) return;

  appStore.updateInfo = update;

  if (notifiedVersion === update.version) return;
  if (!(await ensureNotificationPermission())) return;

  try {
    await invoke('notify_update_available', { version: update.version });
  } catch (error) {
    console.warn('Update notification failed:', error);
    return;
  }

  try {
    await saveSetting('update_notified_version', update.version);
  } catch (error) {
    console.warn('Failed to persist notified update version:', error);
  }
  } finally {
    automaticCheckInFlight = false;
  }
}

export function startAutomaticUpdateChecks(): () => void {
  let active = true;
  const check = () => { void checkForAutomaticUpdates(() => active); };
  check();

  const timer = window.setInterval(() => {
    check();
  }, UPDATE_CHECK_INTERVAL_MS);
  window.addEventListener('online', check);

  return () => {
    active = false;
    window.removeEventListener('online', check);
    window.clearInterval(timer);
  };
}
