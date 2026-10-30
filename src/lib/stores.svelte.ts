import { invoke } from './tauri';
import { formatIpcError as formatError } from './errors';
import type { ProviderId } from './settings';
import type { SettingsSectionId } from './settingsSections';
import type { OmarchyTheme } from './omarchyTheme';
import type { CustomTheme, SavedTheme } from './customTheme';

type PageId = 'home' | 'insights' | 'contexts' | 'dictionary' | 'snippets' | 'style';
export type AppearanceMode = 'system' | 'light' | 'dark' | 'omarchy' | 'custom';
export type PillState = 'idle' | 'recording' | 'processing' | 'handsfree';
type FetchStatus = 'idle' | 'loading' | 'loaded' | 'error';

export interface Snippet {
  id: number;
  trigger: string;
  expansion: string;
  instructions: string;
  use_count: number;
  created_at: string;
  inherited_from_everywhere?: boolean;
  assigned_to_context?: boolean;
}

export interface DictionaryCorrection {
  id: number;
  dictionary_id: number;
  context_id: number;
  mistake: string;
  auto_learned: boolean;
  correction_count: number;
  confidence_tier: 'manual' | 'low' | 'medium' | 'high' | string;
  last_seen_at: string | null;
  created_at: string;
}

export interface DictionaryEntry {
  id: number;
  dictionary_id?: number;
  context_id?: number | null;
  correction_id?: number | null;
  correction_ids?: number[];
  corrections?: DictionaryCorrection[];
  /** This group's mappings, excluding inherited Everywhere corrections. */
  context_corrections?: DictionaryCorrection[];
  inherited_from_everywhere?: boolean;
  assigned_to_context?: boolean;
  term: string;
  mistake: string | null;
  auto_learned: boolean;
  correction_count: number;
  confidence_tier?: 'manual' | 'low' | 'medium' | 'high';
  last_seen_at?: string | null;
  created_at: string;
}

export interface Context {
  id: number;
  name: string;
  is_everywhere: boolean;
  icon: string | null;
  tone: string | null;
  cleanup_intensity: string | null;
  color: string | null;
  custom_instructions: string | null;
  contextual_formatting_disabled: boolean;
  paste_in_chunks: boolean;
  t3_skill_mentions_disabled?: boolean;
  contextual_formatting_override: boolean | null;
  /** ISO timestamp of when the user pinned this context, or null if unpinned. */
  pinned_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface ContextTarget {
  id: number;
  context_id: number;
  executable: string;
  app_name: string | null;
  developer: string | null;
  platform: string | null;
  created_at: string;
}

export type TitleMatchMode = 'contains' | 'starts_with' | 'equals';

export interface ContextSubApp {
  id: number;
  uuid: string;
  context_id: number | null;
  executable: string;
  app_name: string | null;
  label: string;
  icon: string | null;
  title_pattern: string;
  match_mode: TitleMatchMode;
  platform: string | null;
  created_at: string;
}

export interface ContextWebsiteTarget {
  id: number;
  context_id: number;
  domain: string;
  created_at: string;
}

export interface UpdateInfo {
  version: string;
  downloadUrl: string;
  assetName: string;
  installMode: 'install' | 'download';
  installHint?: string | null;
}

export interface ProviderStatusAlert {
  providerId: ProviderId;
  providerName: string;
  status: string;
  severity: string;
  message: string;
  detailsUrl: string;
}

export interface GlobalMessage {
  message: string;
  showToUsers: boolean;
  visibleUntil?: number | null;
}

export const appStore = $state({
  currentPage: 'home' as PageId,
  settingsOpen: false,
  settingsMobileList: false,
  mobileCompact: false,
  settingsSection: 'general' as SettingsSectionId,
  settingsAnimDir: 1 as 1 | -1,
  appVersion: '',
  devModeEnabled: false,
  devModeOnStartup: false,
  ruinAccessibility: false,
  appearanceMode: 'system' as AppearanceMode,
  accentColor: null as string | null,
  subAppCaptureHotkey: null as string | null,
  omarchyTheme: null as OmarchyTheme | null,
  customTheme: null as CustomTheme | null,
  savedThemes: [] as SavedTheme[],
  activeThemeId: null as string | null,
  appearanceSaving: false,
  appearanceChangeVersion: 0,
  legacyFeaturesChangeVersion: 0,
  cleanupEnabled: true,
  cleanupChangeVersion: 0,
  legacyFeaturesEnabled: false,
  pillState: 'idle' as PillState,
  setupComplete: null as boolean | null,
  snippets: [] as Snippet[],
  snippetsFetchStatus: 'idle' as FetchStatus,
  snippetsFetchError: '',
  dictionary: [] as DictionaryEntry[],
  dictionaryFetchStatus: 'idle' as FetchStatus,
  dictionaryFetchError: '',
  updateInfo: null as UpdateInfo | null,
  updateInstalling: false,
  updateProgress: '',
  updateInstalled: false,
  updateInstallError: '',
  betaUpdatesEnabled: false,
  providerStatusAlerts: [] as ProviderStatusAlert[],
  providerStatusSimulation: false,
  globalMessage: null as GlobalMessage | null,
  globalMessageSimulation: false,
  recoveryStorageWarning: false,
  apiHealthy: null as boolean | null,
  isOnline: true,
});

let snippetsFetchToken = 0;
let dictionaryFetchToken = 0;

export function cancelSnippetsFetch() {
  snippetsFetchToken++;
  if (appStore.snippetsFetchStatus === 'loading') appStore.snippetsFetchStatus = 'loaded';
}
export function cancelDictionaryFetch() {
  dictionaryFetchToken++;
  if (appStore.dictionaryFetchStatus === 'loading') appStore.dictionaryFetchStatus = 'loaded';
}

export function formatIpcError(err: unknown, action?: string): string {
  return formatError(err, action);
}

export async function fetchSnippets(): Promise<void> {
  const token = ++snippetsFetchToken;
  appStore.snippetsFetchStatus = 'loading';
  appStore.snippetsFetchError = '';
  try {
    const data = await invoke<Snippet[]>('get_snippets');
    if (token !== snippetsFetchToken) return;
    appStore.snippets = data ?? [];
    appStore.snippetsFetchStatus = 'loaded';
  } catch (err) {
    if (token !== snippetsFetchToken) return;
    console.error('IPC fetchSnippets failed:', err);
    appStore.snippetsFetchStatus = 'error';
    appStore.snippetsFetchError = formatIpcError(err, 'Could not load your snippets');
  }
}

export const cleanupPromptStore = $state<{ override: string }>({ override: '' });

export const cleanupPromptEditor = $state<{
  open: boolean;
  intensity: 'light' | 'medium' | 'high' | null;
  tone: 'casual' | 'formal' | 'very_casual' | null;
  provider: ProviderId | null;
  model: string | null;
  origin: { x: number; y: number } | null;
}>({
  open: false,
  intensity: null,
  tone: null,
  provider: null,
  model: null,
  origin: null,
});

export function openCleanupPromptEditor(
  provider: ProviderId,
  model: string,
  triggerRect: DOMRect,
  intensity: 'light' | 'medium' | 'high' | null = null,
  tone: 'casual' | 'formal' | 'very_casual' | null = null
) {
  cleanupPromptEditor.intensity = intensity;
  cleanupPromptEditor.tone = tone;
  cleanupPromptEditor.provider = provider;
  cleanupPromptEditor.model = model;
  cleanupPromptEditor.origin = {
    x: triggerRect.left + triggerRect.width / 2,
    y: triggerRect.top + triggerRect.height / 2,
  };
  cleanupPromptEditor.open = true;
}

export function closeCleanupPromptEditor() {
  cleanupPromptEditor.open = false;
}

export async function fetchDictionary(): Promise<void> {
  const token = ++dictionaryFetchToken;
  appStore.dictionaryFetchStatus = 'loading';
  appStore.dictionaryFetchError = '';
  try {
    const data = await invoke<DictionaryEntry[]>('get_dictionary');
    if (token !== dictionaryFetchToken) return;
    appStore.dictionary = data ?? [];
    appStore.dictionaryFetchStatus = 'loaded';
  } catch (err) {
    if (token !== dictionaryFetchToken) return;
    console.error('IPC fetchDictionary failed:', err);
    appStore.dictionaryFetchStatus = 'error';
    appStore.dictionaryFetchError = formatIpcError(err, 'Could not load your vocabulary');
  }
}
