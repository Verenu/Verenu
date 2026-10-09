import { appStore } from './stores';
import { directionFromOrder, SETTINGS_SECTION_ORDER } from './motion';
import type { SettingsSectionId } from './settingsSections';

export function openSetupSettings(section: SettingsSectionId): void {
  appStore.settingsAnimDir = directionFromOrder(appStore.settingsSection, section, SETTINGS_SECTION_ORDER);
  appStore.settingsSection = section;
  appStore.settingsMobileList = false;
  appStore.settingsOpen = true;
}
