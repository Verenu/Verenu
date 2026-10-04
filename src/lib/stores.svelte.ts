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
  /** Canonical dictionary row id for Context-scoped DTOs. */
  dictionary_id?: number;
  /** Context that supplies the effective row, when returned by a Context API. */
  context_id?: number | null;
  /** Context-owned correction mapping id used by rejection/deletion paths. */
  correction_id?: number | null;
  /** Context-owned mapping ids, preserving all comma-separated mistake variants. */
  correction_ids?: number[];
  /** All effective Context-owned mappings, in deterministic backend order. */
  corrections?: DictionaryCorrection[];
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

/** A place inside an app: the app plus a window-title rule. */
export interface ContextSubApp {
  id: number;
  uuid: string;
  /** `null` while the sub-app waits in the sub-app list. */
  context_id: number | null;
  executable: string;
  app_name: string | null;
  label: string;
  /** Icon key from CONTEXT_ICON_CHOICES; `null` shows the app icon. */
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
  // Android: show the section list instead of a section (phone drill-down).
  settingsMobileList: false,
  // The settings rail lives in the Sidebar but the panel lives in Settings, so
  // the active section and its swap direction have to be shared state.
  settingsSection: 'general' as SettingsSectionId,
  settingsAnimDir: 1 as 1 | -1,
  appVersion: '',
  devModeEnabled: false,
  devModeOnStartup: false,
  // Developer-only: dump diagnostics into the OS accessibility tree for agent
  // SnapShots. Off by default. Real screen-reader use is unusable while on.
  ruinAccessibility: false,
  appearanceMode: 'system' as AppearanceMode,
  accentColor: null as string | null,
  /** Sub-app capture chord ("Ctrl+Alt+Shift+S"); null means the platform default. */
  subAppCaptureHotkey: null as string | null,
  // Active Omarchy palette (Linux desktop only); drives the Omarchy appearance mode.
  omarchyTheme: null as OmarchyTheme | null,
  // Hex palette for the Custom appearance mode (all desktop platforms).
  customTheme: null as CustomTheme | null,
  // Named palettes from the theme editor, and the one applied this session
  // (breaks ties between saved themes that share colors).
  savedThemes: [] as SavedTheme[],
  activeThemeId: null as string | null,
  // True while an appearance change is being saved; selections made meanwhile are ignored.
  appearanceSaving: false,
  // Mirrors the `cleanup_enabled` setting. Shared here (rather than owned
  // privately by GeneralSection) so Style.svelte and the App Mappings
  // settings page can react live to the toggle without their own
  // fetch/listener plumbing — both are inert once cleanup is off, since
  // profile/tone/per-app cleanup-intensity overrides only ever feed the
  // cleanup LLM call that this setting skips entirely.
  cleanupEnabled: true,
  // Mirrors `legacy_features_enabled`. Gates App Mappings in Settings and the
  // Dictionary/Snippets pages in the sidebar nav — both superseded by Contexts,
  // kept reachable for anyone still relying on the old per-page workflow.
  legacyFeaturesEnabled: false,
  syncEnabled: false,
  pillState: 'idle' as PillState,
  setupComplete: null as boolean | null,
  snippets: [] as Snippet[],
  snippetsFetchStatus: 'idle' as FetchStatus,
  snippetsFetchError: '',
  dictionary: [] as DictionaryEntry[],
  dictionaryFetchStatus: 'idle' as FetchStatus,
  dictionaryFetchError: '',
  updateInfo: null as UpdateInfo | null,
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

/**
 * The one cleanup prompt, used by every model. It was once keyed per
 * provider/model, which quietly lost the edit the moment a fallback took over
 * — you tuned the prompt on your default and the fallback ran the stock one.
 */
export const cleanupPromptStore = $state<{ override: string }>({ override: '' });

export const cleanupPromptEditor = $state<{
  open: boolean;
  intensity: 'light' | 'medium' | 'high' | null;
  tone: 'casual' | 'formal' | 'very_casual' | null;
  /** The model the editor tests the prompt against — not what it saves under. */
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
