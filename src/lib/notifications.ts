import { isPermissionGranted, requestPermission } from '@tauri-apps/plugin-notification';
import { isTauriRuntime } from './tauri';

export async function ensureNotificationPermission(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  try {
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === 'granted';
  } catch (error) {
    console.warn('Notification permission request failed:', error);
    return false;
  }
}

export async function isNotificationPermissionGranted(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  try {
    return await isPermissionGranted();
  } catch (error) {
    console.warn('Notification permission check failed:', error);
    return false;
  }
}
