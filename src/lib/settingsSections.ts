import type { icons } from './icons';
import { isAndroid } from './platform';

export type SettingsSectionId =
  | 'general'
  | 'text'
  | 'advanced'
  | 'subapps'
  | 'apps'
  | 'keys'
  | 'models'
  | 'privacy'
  | 'sync'
  | 'integrations'
  | 'permissions'
  | 'developer'
  | 'about';

interface SettingsSection {
  id: SettingsSectionId;
  label: string;
  icon: keyof typeof icons;
  group: string;
  macOnly?: boolean;
  devOnly?: boolean;
  legacyOnly?: boolean;
  desktopOnly?: boolean;
}

const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  { id: 'general',      label: 'General',      icon: 'sliders', group: 'Dictation' },
  { id: 'text',         label: 'Text',         icon: 'type',    group: 'Dictation' },
  { id: 'advanced',     label: 'Audio',        icon: 'mic',     group: 'Dictation' },
  { id: 'subapps',      label: 'Sub-apps',     icon: 'apps',    group: 'Dictation', desktopOnly: true },
  { id: 'apps',         label: 'App Mappings', icon: 'apps',    group: 'Dictation', legacyOnly: true },
  { id: 'keys',         label: 'Providers',    icon: 'key',     group: 'AI' },
  { id: 'models',       label: 'Models',       icon: 'command', group: 'AI' },
  { id: 'privacy',      label: 'Privacy',      icon: 'lock',    group: 'Data' },
  { id: 'sync',         label: 'Sync',         icon: 'devices', group: 'Data' },
  { id: 'integrations', label: 'Integrations', icon: 'layers',  group: 'Data' },
  { id: 'permissions',  label: 'Permissions',  icon: 'shield',  group: 'System', macOnly: true },
  { id: 'developer',    label: 'Developer',    icon: 'code',    group: 'System', devOnly: true },
  { id: 'about',        label: 'About',        icon: 'help',    group: 'Verenu' },
];

export const SETTINGS_SECTION_ORDER: readonly SettingsSectionId[] =
  SETTINGS_SECTIONS.map((s) => s.id);

export function isSettingsSectionId(value: string): value is SettingsSectionId {
  return (SETTINGS_SECTION_ORDER as readonly string[]).includes(value);
}

interface SettingsSectionGroup {
  group: string;
  items: SettingsSection[];
}

export function visibleSettingsSections(opts: {
  isMac: boolean;
  devMode: boolean;
  legacyMode?: boolean;
}): SettingsSectionGroup[] {
  const groups: SettingsSectionGroup[] = [];
  for (const section of SETTINGS_SECTIONS) {
    if (section.macOnly && !opts.isMac && !isAndroid) continue;
    if (section.devOnly && !opts.devMode) continue;
    if (section.legacyOnly && !opts.legacyMode) continue;
    if (section.desktopOnly && isAndroid) continue;
    const last = groups[groups.length - 1];
    if (last && last.group === section.group) last.items.push(section);
    else groups.push({ group: section.group, items: [section] });
  }
  return groups;
}
