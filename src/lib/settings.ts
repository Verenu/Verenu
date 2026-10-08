import { invoke } from './tauri';
import { classifyIpcError } from './errors';
import type { TranscriptionLanguageCode } from './transcriptionLanguages';
import type { CustomTheme, SavedTheme } from './customTheme';

export const SETTINGS_SAVE_ERROR_EVENT = 'verenu:setting-save-error';

export type ProviderId =
  | 'groq'
  | 'openai'
  | 'google'
  | 'assemblyai'
  | 'openrouter'
  | 'xai'
  | `custom:${string}`
  | 'local';
export type ProviderModelMap = Record<ProviderId, string[]>;
export type ToneId = 'casual' | 'formal' | 'very_casual';
export type CleanupIntensity = 'none' | 'light' | 'medium' | 'high';
export type HistoryRetention = '7 days' | '30 days' | '90 days' | 'Forever';
export type AppearanceMode = 'system' | 'light' | 'dark' | 'omarchy' | 'custom';

/** Pill placements anchored to the screen. Mirrors `ANDROID_PILL_SCREEN_POSITIONS` in Rust. */
export type AndroidPillScreenPosition =
  | 'screen-top-left'
  | 'screen-top'
  | 'screen-top-right'
  | 'screen-left'
  | 'screen-middle'
  | 'screen-right'
  | 'screen-bottom-left'
  | 'screen-bottom'
  | 'screen-bottom-right'
  | 'punch-hole';

/** Where the Android dictation pill sits. Mirrors `ANDROID_PILL_POSITIONS` in Rust. */
export type AndroidPillPosition =
  | 'keyboard-center'
  | 'keyboard-left'
  | 'keyboard-right'
  | AndroidPillScreenPosition;

export const ANDROID_PILL_SCREEN_POSITION_OPTIONS: { id: AndroidPillScreenPosition; label: string }[] = [
  { id: 'screen-top-left', label: 'Top left' },
  { id: 'screen-top', label: 'Top center' },
  { id: 'screen-top-right', label: 'Top right' },
  { id: 'screen-left', label: 'Middle left' },
  { id: 'screen-middle', label: 'Middle of screen' },
  { id: 'screen-right', label: 'Middle right' },
  { id: 'screen-bottom-left', label: 'Bottom left' },
  { id: 'screen-bottom', label: 'Bottom center' },
  { id: 'screen-bottom-right', label: 'Bottom right' },
  { id: 'punch-hole', label: 'Under the camera' },
];

export const ANDROID_PILL_POSITION_OPTIONS: { id: AndroidPillPosition; label: string }[] = [
  { id: 'keyboard-center', label: 'Above keyboard, center' },
  { id: 'keyboard-left', label: 'Above keyboard, left' },
  { id: 'keyboard-right', label: 'Above keyboard, right' },
  ...ANDROID_PILL_SCREEN_POSITION_OPTIONS,
];

export const DEFAULT_ANDROID_PILL_POSITION: AndroidPillPosition = 'keyboard-center';
/** Where the pill rests when a dictation outlives the keyboard. */
export const DEFAULT_ANDROID_PILL_DOCK_POSITION: AndroidPillScreenPosition = 'screen-bottom';
export type LocalModelMemoryPolicy =
  | 'keep_loaded'
  | 'unload_after_5m'
  | 'unload_after_15m'
  | 'unload_immediately';
export type { TranscriptionLanguageCode } from './transcriptionLanguages';

export interface AppMapping {
  exe: string;
  profile: string;
  name?: string;
  cleanup_intensity?: CleanupIntensity;
}

type SettingsValueMap = {
  autostart_enabled: boolean;
  hotkey: string[];
  custom_providers: import('./customProviders.svelte').CustomProvider[];
  transcription_provider: ProviderId;
  transcription_language: TranscriptionLanguageCode;
  cleanup_provider: ProviderId;
  transcription_model: string;
  cleanup_model: string;
  transcription_models_by_provider: ProviderModelMap;
  cleanup_models_by_provider: ProviderModelMap;
  transcription_default_model: string;
  cleanup_default_model: string;
  transcription_fallback_models: string[];
  dual_transcription_enabled: boolean;
  model_selection_mode: 'manual' | 'fastest' | 'balanced' | 'quality';
  analytics_enabled: boolean;
  cleanup_fallback_models: string[];
  cleanup_enabled: boolean;
  cleanup_cache_enabled: boolean;
  default_tone: ToneId;
  cleanup_intensity: CleanupIntensity;
  app_mappings: AppMapping[];
  noise_reduction: boolean;
  mute_audio: boolean;
  mic_mute_button_dictation: boolean;
  exclusive_mic: boolean;
  pause_media_during_dictation: boolean;
  play_start_stop_sounds: boolean;
  sound_effects_volume: number;
  mic_gain: number;
  setup_complete: boolean;
  /** Unfinished wizard position; cleared (null) once setup completes. */
  setup_progress: import('./setup/setupProgress').SetupProgress | null;
  force_setup_on_launch: boolean;
  ruin_accessibility: boolean;
  dev_mode_on_startup: boolean;
  auto_learn_enabled: boolean;
  contextual_formatting_enabled: boolean;
  /** @deprecated Compatibility mirror for one downgrade cycle. */
  contextual_caps_enabled: boolean;
  /** @deprecated Compatibility mirror for one downgrade cycle. */
  auto_spacing_enabled: boolean;
  caps_lock_uppercase_enabled: boolean;
  clipboard_phrase_enabled: boolean;
  clipboard_phrase: string;
  history_retention: HistoryRetention;
  github_username: string;
  local_model_memory_policy: LocalModelMemoryPolicy;
  microphone_device: string | null;
  update_dismissed_version: string | null;
  update_notified_version: string | null;
  beta_updates_enabled: boolean;
  verenu_service_checks_enabled: boolean;
  appearance_mode: AppearanceMode;
  /** Sub-app capture chord, e.g. "Ctrl+Alt+Shift+S"; unset means the platform default. */
  sub_app_capture_hotkey: string | null;
  accent_color: string | null;
  /** Hex palette for the Custom appearance mode. */
  custom_theme: CustomTheme | null;
  /** Named palettes the user created in the theme editor. */
  custom_themes: SavedTheme[] | null;
  android_pill_position: AndroidPillPosition;
  android_pill_dock_position: AndroidPillScreenPosition;
  /** Android: sit the pill over the keyboard's own mic button when it can be found. */
  android_pill_cover_keyboard_mic: boolean;
  /** Android: hide offline if any active selected model needs a network provider. */
  android_pill_hide_offline: boolean;
  advanced_model_ui: boolean;
  /** One cleanup prompt for every model — see stores.svelte.ts. */
  cleanup_prompt_override: string;
  style_prompt_instructions: Partial<Record<Exclude<CleanupIntensity, 'none'> | ToneId, string>>;
  /** Derived cache of each provider's live model list. Written only by modelCatalogStore. */
  provider_model_cache: Record<string, unknown>;
  legacy_features_enabled: boolean;
  sync_enabled: boolean;
  sync_muting_enabled: boolean;
  /** Per-peer Tailscale routes, written only by SyncManager's validated command. */
  sync_peer_addresses: Record<string, string> | null;
};

type SettingKey = keyof SettingsValueMap;
type WritableSettingKey = Exclude<SettingKey, 'sync_peer_addresses'>;

/** Nullable persisted values; callers retain their own UI defaults. No cache. */
export type SettingsSnapshot = {
  [K in Exclude<SettingKey, 'default_tone' | 'cleanup_intensity' | 'app_mappings' | 'setup_complete'>]?: SettingsValueMap[K] | null;
};

export function loadSettingsSnapshot(): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>('get_all_settings');
}

type BooleanSettingKey = {
  [K in WritableSettingKey]: SettingsValueMap[K] extends boolean ? K : never;
}[WritableSettingKey];

export function booleanSettingHandler(
  key: BooleanSettingKey,
  update: (value: boolean) => void,
  failed: () => void,
) {
  return async (value: boolean) => {
    update(value);
    try {
      await saveSetting(key, value);
    } catch (err) {
      update(!value);
      failed();
      console.error(`save ${key} failed:`, err);
    }
  };
}

export function saveSetting<K extends WritableSettingKey>(key: K, value: SettingsValueMap[K]) {
  return invoke('save_setting', { key, value }).catch((error) => {
    const classified = classifyIpcError(error);
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(SETTINGS_SAVE_ERROR_EVENT, {
        detail: `Could not save this setting. ${classified.message}`,
      }));
    }
    throw error;
  });
}
