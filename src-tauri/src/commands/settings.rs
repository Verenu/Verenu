//! Settings, API keys, prompt configuration, and data import/export.

use super::*;

mod api_keys;
mod custom_providers;
mod model_catalog;
mod import_export;
mod prompts;

pub use api_keys::*;
pub use custom_providers::*;
pub use model_catalog::*;
pub use import_export::*;
pub use prompts::*;

const CLEANUP_PROMPT_OVERRIDE_CHAR_LIMIT: usize = 20_000;

fn github_identity_changed(current: Option<&str>, next: &str) -> bool {
    current.is_none_or(|current| !current.eq_ignore_ascii_case(next))
}

fn save_github_username(
    settings: &store::SettingsHandle,
    value: serde_json::Value,
) -> Result<(), String> {
    let current = settings
        .get(store::GITHUB_USERNAME)
        .and_then(|value| value.as_str().map(str::to_owned));
    let changed =
        github_identity_changed(current.as_deref(), value.as_str().unwrap_or_default());
    if changed {
        // Invalidate in-flight responses and the prior account's cooldown
        // before publishing the new owner in settings.
        crate::commands::github::invalidate_github_refresh_owner();
        settings.save_values([
            (store::GITHUB_USERNAME, value),
            (store::GITHUB_COMMIT_CACHE, serde_json::Value::Null),
        ])
    } else {
        // GitHub logins are case-insensitive. Keep the stored spelling stable
        // so an in-flight response still matches its exact settings owner.
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum SettingKind {
    Provider,
    CustomProviders,
    TranscriptionLanguage,
    StringOrNull,
    DefaultTone,
    CleanupIntensity,
    HistoryRetention,
    GithubUsername,
    LocalModelMemoryPolicy,
    ModelSelectionMode,
    ModelMap,
    StringArray,
    CleanupPromptOverride,
    StylePromptInstructions,
    ProviderModelCache,
    AppearanceMode,
    AccentColor,
    CustomTheme,
    CustomThemes,
    AndroidPillPosition,
    AndroidPillDockPosition,
    SetupProgress,
    Bool,
    MicGain,
    SoundEffectsVolume,
    AppMappings,
    Hotkey,
    ClipboardPhrase,
    SubAppChord,
}

#[derive(Clone, Copy)]
pub struct SettingSpec {
    key: &'static str,
    kind: SettingKind,
    readable: bool,
    exportable: bool,
}

// The table is the allowlist; every listed key is readable.
macro_rules! setting_specs {
    ($($key:ident: $kind:ident = $exportable:literal),* $(,)?) => {
        const SETTING_SPECS: &[SettingSpec] = &[$(
            SettingSpec { key: store::$key, kind: SettingKind::$kind, readable: true, exportable: $exportable },
        )*];
    };
}

setting_specs! {
    CUSTOM_PROVIDERS: CustomProviders = true,
    TRANSCRIPTION_PROVIDER: Provider = true,
    TRANSCRIPTION_LANGUAGE: TranscriptionLanguage = true,
    CLEANUP_PROVIDER: Provider = true,
    MODEL_SELECTION_MODE: ModelSelectionMode = true,
    TRANSCRIPTION_MODEL: StringOrNull = true,
    CLEANUP_MODEL: StringOrNull = true,
    TRANSCRIPTION_MODELS_BY_PROVIDER: ModelMap = true,
    CLEANUP_MODELS_BY_PROVIDER: ModelMap = true,
    TRANSCRIPTION_DEFAULT_MODEL: StringOrNull = true,
    CLEANUP_DEFAULT_MODEL: StringOrNull = true,
    TRANSCRIPTION_FALLBACK_MODELS: StringArray = true,
    DUAL_TRANSCRIPTION_ENABLED: Bool = true,
    CLEANUP_FALLBACK_MODELS: StringArray = true,
    CLEANUP_ENABLED: Bool = true,
    CLEANUP_CACHE_ENABLED: Bool = true,
    HOTKEY: Hotkey = true,
    SUB_APP_CAPTURE_HOTKEY: SubAppChord = false,
    MICROPHONE_DEVICE: StringOrNull = false,
    DEFAULT_TONE: DefaultTone = true,
    CLEANUP_INTENSITY: CleanupIntensity = true,
    APP_MAPPINGS: AppMappings = true,
    NOISE_REDUCTION: Bool = true,
    MUTE_AUDIO: Bool = true,
    MIC_MUTE_BUTTON_DICTATION: Bool = true,
    EXCLUSIVE_MIC: Bool = true,
    PAUSE_MEDIA_DURING_DICTATION: Bool = true,
    MIC_GAIN: MicGain = false,
    PLAY_START_STOP_SOUNDS: Bool = true,
    SOUND_EFFECTS_VOLUME: SoundEffectsVolume = true,
    SETUP_COMPLETE: Bool = false,
    SETUP_PROGRESS: SetupProgress = false,
    AUTO_LEARN_ENABLED: Bool = true,
    AUTO_LEARN_EVENT_MODE: Bool = true,
    CONTEXTUAL_CAPS: Bool = true,
    AUTO_SPACING: Bool = true,
    CONTEXTUAL_FORMATTING: Bool = true,
    APPEARANCE_MODE: AppearanceMode = true,
    ACCENT_COLOR: AccentColor = true,
    CUSTOM_THEME: CustomTheme = true,
    CUSTOM_THEMES: CustomThemes = true,
    ANDROID_PILL_POSITION: AndroidPillPosition = true,
    ANDROID_PILL_DOCK_POSITION: AndroidPillDockPosition = true,
    ANDROID_PILL_COVER_KEYBOARD_MIC: Bool = true,
    ANDROID_PILL_HIDE_OFFLINE: Bool = true,
    FORCE_SETUP_ON_LAUNCH: Bool = false,
    RUIN_ACCESSIBILITY: Bool = false,
    DEV_MODE_ON_STARTUP: Bool = true,
    ADVANCED_MODEL_UI: Bool = true,
    CLEANUP_PROMPT_OVERRIDE: CleanupPromptOverride = true,
    STYLE_PROMPT_INSTRUCTIONS: StylePromptInstructions = true,
    UPDATE_DISMISSED_VERSION: StringOrNull = false,
    UPDATE_NOTIFIED_VERSION: StringOrNull = false,
    BETA_UPDATES_ENABLED: Bool = true,
    ANALYTICS_ENABLED: Bool = true,
    VERENU_SERVICE_CHECKS_ENABLED: Bool = true,
    HISTORY_RETENTION: HistoryRetention = true,
    GITHUB_USERNAME: GithubUsername = false,
    LOCAL_MODEL_MEMORY_POLICY: LocalModelMemoryPolicy = true,
    AUTOSTART_ENABLED: Bool = true,
    CAPS_LOCK_UPPERCASE: Bool = true,
    CLIPBOARD_PHRASE_ENABLED: Bool = true,
    CLIPBOARD_PHRASE: ClipboardPhrase = true,
    LEGACY_FEATURES_ENABLED: Bool = true,
    SYNC_ENABLED: Bool = false,
    SYNC_MUTING_ENABLED: Bool = false,
    PROVIDER_MODEL_CACHE: ProviderModelCache = false,
}

fn spec_for(key: &str) -> Option<&'static SettingSpec> {
    SETTING_SPECS.iter().find(|spec| spec.key == key)
}

pub fn is_readable_setting_key(key: &str) -> bool {
    spec_for(key).is_some_and(|spec| spec.readable)
}

pub fn is_exportable_setting_key(key: &str) -> bool {
    spec_for(key).is_some_and(|spec| spec.exportable)
}

fn exportable_setting_keys() -> impl Iterator<Item = &'static str> {
    SETTING_SPECS
        .iter()
        .filter(|spec| spec.exportable)
        .map(|spec| spec.key)
}

/// Bounded device-local theme library, using the active palette's validation.
fn is_saved_themes(value: &serde_json::Value) -> bool {
    use crate::system::omarchy_theme::{is_custom_theme, is_hex_color};
    let Some(themes) = value.as_array().filter(|themes| themes.len() <= 24) else {
        return false;
    };
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    themes.iter().all(|theme| {
        let Some(object) = theme.as_object() else { return false; };
        if object.len() != 4 || !object.keys().all(|key| matches!(key.as_str(), "id" | "name" | "palette" | "accent")) {
            return false;
        }
        let Some(id) = object.get("id").and_then(|v| v.as_str()) else { return false; };
        let Some(name) = object.get("name").and_then(|v| v.as_str()) else { return false; };
        !id.is_empty() && id.len() <= 64
            && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            && !name.trim().is_empty() && name.chars().count() <= 40 && name == name.trim()
            && ids.insert(id) && names.insert(name.to_lowercase())
            && object.get("palette").is_some_and(is_custom_theme)
            && object.get("accent").is_some_and(|v| v.is_null() || v.as_str().is_some_and(is_hex_color))
    })
}

pub fn validate_setting(key: &str, value: &serde_json::Value) -> Result<(), String> {
    let is_model_map = |v: &serde_json::Value| {
        let Some(obj) = v.as_object() else {
            return false;
        };
        obj.keys()
            .all(|k| store::PROVIDERS.contains(&k.as_str()) || crate::api::custom::is_custom_id(k))
            && obj.values().all(|val| {
                val.as_array().is_some_and(|arr| {
                    arr.iter()
                        .all(|x| x.as_str().is_some_and(|s| !s.trim().is_empty()))
                })
            })
    };
    let is_non_empty_string_array = |v: &serde_json::Value| {
        v.as_array().is_some_and(|arr| {
            arr.iter()
                .all(|x| x.as_str().is_some_and(|s| !s.trim().is_empty()))
        })
    };
    let is_cleanup_prompt_override = |v: &serde_json::Value| {
        v.as_str()
            .is_some_and(|text| text.chars().count() <= CLEANUP_PROMPT_OVERRIDE_CHAR_LIMIT)
    };
    let is_valid_app_mappings = |v: &serde_json::Value| {
        let Ok(mappings) = serde_json::from_value::<Vec<AppMapping>>(v.clone()) else {
            return false;
        };
        let mut seen = std::collections::HashSet::new();
        mappings.iter().all(|mapping| {
            let exe = mapping.exe.trim().to_lowercase();
            let profile = mapping.profile.trim();
            !exe.is_empty()
                && seen.insert(exe)
                && store::is_supported_default_tone(profile)
                && mapping
                    .cleanup_intensity
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|value| {
                        value.is_empty() || store::is_supported_cleanup_intensity(value)
                    })
        })
    };
    // Strict reject, not normalize: this function only answers yes/no and never
    // mutates the value before it is saved. The catalog store is the sole
    // writer and normalizes on its side; anything malformed reaching here is a
    // hand-edited settings file, which should bounce rather than be guessed at.
    let is_provider_model_cache = |v: &serde_json::Value| {
        let Some(obj) = v.as_object() else {
            return false;
        };
        obj.iter().all(|(provider, entry)| {
            if !store::PROVIDERS.contains(&provider.as_str()) {
                return false;
            }
            let Some(entry) = entry.as_object() else {
                return false;
            };
            let string_array = |key: &str| {
                entry.get(key).is_some_and(|v| {
                    v.as_array()
                        .is_some_and(|arr| arr.iter().all(serde_json::Value::is_string))
                })
            };
            let finite_timestamp = |key: &str| {
                entry
                    .get(key)
                    .is_some_and(|v| v.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
            };
            string_array("ids")
                && entry.get("metadata").is_none_or(|metadata| {
                    metadata.as_object().is_some_and(|models| models.len() <= 10000 && models.values().all(|model| {
                        model.as_object().is_some_and(|model| {
                            model.get("label").and_then(serde_json::Value::as_str).is_some_and(|label| label.len() <= 800)
                                && model.get("tasks").and_then(serde_json::Value::as_array).is_some_and(|tasks| tasks.len() <= 2 && tasks.iter().all(|task| matches!(task.as_str(), Some("transcription" | "cleanup"))))
                        })
                    }))
                })
                && entry.get("warning").is_none_or(|warning| warning.is_null() || warning.is_string())
                && string_array("everSeen")
                && finite_timestamp("lastSuccessAt")
                && finite_timestamp("lastAttemptAt")
                && entry
                    .get("lastError")
                    .is_some_and(|v| v.is_string() || v.is_null())
                && entry.get("missing").is_some_and(|missing| {
                    missing.as_object().is_some_and(|counters| {
                        counters.values().all(|counter| {
                            counter.as_object().is_some_and(|counter| {
                                counter.get("count").is_some_and(|c| c.as_u64().is_some())
                                    && counter.get("lastCountedAt").is_some_and(|t| {
                                        t.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0)
                                    })
                            })
                        })
                    })
                })
        })
    };
    let Some(spec) = spec_for(key) else {
        return Err(format!("Invalid or unsupported setting: {key}"));
    };
    let valid = match spec.kind {
        SettingKind::Provider => value
            .as_str()
            .is_some_and(|v| store::PROVIDERS.contains(&v) || crate::api::custom::is_custom_id(v)),
        SettingKind::CustomProviders => crate::api::custom::normalize_list(value).is_ok(),
        SettingKind::TranscriptionLanguage => value
            .as_str()
            .is_some_and(store::is_supported_transcription_language),
        SettingKind::StringOrNull => value.is_string() || value.is_null(),
        SettingKind::DefaultTone => value.as_str().is_some_and(store::is_supported_default_tone),
        SettingKind::CleanupIntensity => value
            .as_str()
            .is_some_and(store::is_supported_cleanup_intensity),
        SettingKind::HistoryRetention => value
            .as_str()
            .is_some_and(store::is_supported_history_retention),
        SettingKind::GithubUsername => value
            .as_str()
            .is_some_and(|value| value.is_empty() || crate::api::github::valid_username(value)),
        SettingKind::LocalModelMemoryPolicy => value
            .as_str()
            .is_some_and(store::is_supported_local_model_memory_policy),
        SettingKind::ModelMap => is_model_map(value),
        SettingKind::ModelSelectionMode => value.as_str().is_some_and(|mode| matches!(mode, "manual" | "fastest" | "balanced" | "quality")),
        SettingKind::SubAppChord => value
            .as_str()
            .is_some_and(|v| crate::core::hotkey::chord::Chord::parse(v).is_some()),
        SettingKind::ClipboardPhrase => value
            .as_str()
            .map(store::normalize_clipboard_phrase)
            .is_some_and(|v| store::is_valid_clipboard_phrase(&v)),
        SettingKind::StringArray => is_non_empty_string_array(value),
        SettingKind::CleanupPromptOverride => is_cleanup_prompt_override(value),
        SettingKind::StylePromptInstructions => value.as_object().is_some_and(|map| {
            map.iter().all(|(key, value)| {
                matches!(
                    key.as_str(),
                    "light" | "medium" | "high" | "casual" | "formal" | "very_casual"
                ) && is_cleanup_prompt_override(value)
            })
        }),
        SettingKind::ProviderModelCache => is_provider_model_cache(value),
        SettingKind::AppearanceMode => value
            .as_str()
            .is_some_and(|v| matches!(v, "system" | "light" | "dark" | "omarchy" | "custom")),
        SettingKind::AndroidPillPosition => value
            .as_str()
            .is_some_and(|v| crate::android::ANDROID_PILL_POSITIONS.contains(&v)),
        SettingKind::AndroidPillDockPosition => value
            .as_str()
            .is_some_and(|v| crate::android::ANDROID_PILL_SCREEN_POSITIONS.contains(&v)),
        SettingKind::AccentColor => {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(crate::system::omarchy_theme::is_hex_color)
        }
        SettingKind::CustomTheme => {
            value.is_null() || crate::system::omarchy_theme::is_custom_theme(value)
        }
        SettingKind::CustomThemes => {
            value.is_null() || is_saved_themes(value)
        }
        SettingKind::SetupProgress => is_setup_progress(value),
        SettingKind::Bool => value.is_boolean(),
        SettingKind::MicGain => value.as_f64().is_some_and(|v| (1.0..=8.0).contains(&v)),
        SettingKind::SoundEffectsVolume => {
            value.as_f64().is_some_and(|v| (0.0..=100.0).contains(&v))
        }
        SettingKind::AppMappings => is_valid_app_mappings(value),
        SettingKind::Hotkey => value.as_array().is_some_and(|keys| {
            let codes: Option<Vec<String>> = keys
                .iter()
                .map(|key| key.as_str().map(String::from))
                .collect();
            codes.is_some_and(|codes| crate::core::hotkey::normalize_codes(&codes).is_ok())
        }),
    };

    if valid {
        Ok(())
    } else {
        Err(format!("Invalid or unsupported setting: {key}"))
    }
}

/// Highest wizard step number any platform uses (the Done screen).
const SETUP_PROGRESS_MAX_STEP: u64 = 12;

/// `null` clears the saved progress; otherwise a bounded step and, optionally,
/// the provider the wizard was configuring.
fn is_setup_progress(value: &serde_json::Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(map) = value.as_object() else {
        return false;
    };
    map.iter().all(|(key, value)| match key.as_str() {
        "step" => value
            .as_u64()
            .is_some_and(|step| step <= SETUP_PROGRESS_MAX_STEP),
        "provider" => value.as_str().is_some_and(|v| store::PROVIDERS.contains(&v)),
        _ => false,
    }) && map.contains_key("step")
}

#[cfg(test)]
mod setting_key_tests {
    use super::*;

    #[test]
    fn style_prompt_instructions_only_accept_editable_styles_and_bounded_text() {
        assert!(validate_setting(
            store::STYLE_PROMPT_INSTRUCTIONS,
            &serde_json::json!({"light": "Custom", "high": "", "formal": "Professional wording"})
        )
        .is_ok());
        for invalid in [
            serde_json::json!({"none": "Custom"}),
            serde_json::json!({"medium": false}),
            serde_json::json!({"medium": "x".repeat(20_001)}),
        ] {
            assert!(validate_setting(store::STYLE_PROMPT_INSTRUCTIONS, &invalid).is_err());
        }
    }

    #[test]
    fn hotkeys_accept_variable_length_and_legacy_single_key_settings() {
        for codes in [
            serde_json::json!(["F5"]),
            serde_json::json!(["F5", ""]),
            serde_json::json!(["ControlLeft", "AltLeft", "ShiftLeft", "MetaLeft", "KeyK"]),
        ] {
            assert!(validate_setting(store::HOTKEY, &codes).is_ok());
        }
        for codes in [
            serde_json::json!([]),
            serde_json::json!([""]),
            serde_json::json!(["ControlLeft", "ControlRight"]),
            serde_json::json!(["KeyK", "KeyK"]),
            serde_json::json!(["ControlLeft", "", "KeyK"]),
            serde_json::json!(["ControlLeft", 3]),
            serde_json::json!(["NoSuchKey"]),
        ] {
            assert!(validate_setting(store::HOTKEY, &codes).is_err());
        }
    }

    #[test]
    fn local_model_maps_accept_all_picker_providers_and_reject_invalid_values() {
        let value = serde_json::json!({
            "groq": [], "openai": [], "google": [], "assemblyai": [],
            "openrouter": [], "xai": [], "local": ["qwen2.5-7b-instruct"]
        });
        for key in [
            store::TRANSCRIPTION_MODELS_BY_PROVIDER,
            store::CLEANUP_MODELS_BY_PROVIDER,
        ] {
            assert!(validate_setting(key, &value).is_ok());
            assert!(validate_setting(key, &serde_json::json!({"local": [""]})).is_err());
            assert!(validate_setting(key, &serde_json::json!({"unknown": ["model"]})).is_err());
        }
    }

    #[test]
    fn readable_settings_exclude_credential_keys() {
        assert!(is_readable_setting_key(store::APPEARANCE_MODE));
        assert!(!is_readable_setting_key(store::KEY_GROQ));
        assert!(!is_readable_setting_key(store::KEY_OPENAI));
        assert!(!is_readable_setting_key(store::KEY_GOOGLE));
    }

    #[test]
    fn exportable_settings_exclude_credential_keys() {
        assert!(is_exportable_setting_key(store::APP_MAPPINGS));
        assert!(!is_exportable_setting_key(store::KEY_GROQ));
        assert!(!is_exportable_setting_key(store::KEY_OPENAI));
        assert!(!is_exportable_setting_key(store::KEY_GOOGLE));
    }

    #[test]
    fn pause_media_during_dictation_is_boolean_setting() {
        assert!(is_readable_setting_key(store::PAUSE_MEDIA_DURING_DICTATION));
        assert!(is_exportable_setting_key(
            store::PAUSE_MEDIA_DURING_DICTATION
        ));
        assert!(validate_setting(
            store::PAUSE_MEDIA_DURING_DICTATION,
            &serde_json::json!(true)
        )
        .is_ok());
        assert!(validate_setting(
            store::PAUSE_MEDIA_DURING_DICTATION,
            &serde_json::json!("yes")
        )
        .is_err());
    }

    #[test]
    fn sound_effects_volume_accepts_percent_range_only() {
        assert!(validate_setting(store::SOUND_EFFECTS_VOLUME, &serde_json::json!(0)).is_ok());
        assert!(validate_setting(store::SOUND_EFFECTS_VOLUME, &serde_json::json!(100)).is_ok());
        assert!(validate_setting(store::SOUND_EFFECTS_VOLUME, &serde_json::json!(-1)).is_err());
        assert!(validate_setting(store::SOUND_EFFECTS_VOLUME, &serde_json::json!(101)).is_err());
    }

    #[test]
    fn setup_progress_accepts_a_bounded_step_and_known_provider() {
        for valid in [
            serde_json::Value::Null,
            serde_json::json!({"step": 0}),
            serde_json::json!({"step": 4, "provider": "groq"}),
            serde_json::json!({"step": 12, "provider": "local"}),
        ] {
            assert!(validate_setting(store::SETUP_PROGRESS, &valid).is_ok(), "{valid}");
        }
        for invalid in [
            serde_json::json!({}),
            serde_json::json!({"provider": "groq"}),
            serde_json::json!({"step": 13}),
            serde_json::json!({"step": -1}),
            serde_json::json!({"step": 1.5}),
            serde_json::json!({"step": 2, "provider": "nope"}),
            serde_json::json!({"step": 2, "apiKey": "secret"}),
            serde_json::json!(3),
        ] {
            assert!(validate_setting(store::SETUP_PROGRESS, &invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn android_pill_dock_position_accepts_only_known_placements() {
        for position in crate::android::ANDROID_PILL_SCREEN_POSITIONS {
            assert!(
                validate_setting(store::ANDROID_PILL_DOCK_POSITION, &serde_json::json!(position))
                    .is_ok()
            );
        }
        for invalid in [
            serde_json::json!("keyboard-center"),
            serde_json::json!("floating"),
            serde_json::json!(3),
            serde_json::Value::Null,
        ] {
            assert!(validate_setting(store::ANDROID_PILL_DOCK_POSITION, &invalid).is_err());
        }
    }

    #[test]
    fn android_pill_position_accepts_only_known_placements() {
        for position in crate::android::ANDROID_PILL_POSITIONS {
            assert!(
                validate_setting(store::ANDROID_PILL_POSITION, &serde_json::json!(position)).is_ok()
            );
        }
        assert!(
            validate_setting(store::ANDROID_PILL_POSITION, &serde_json::json!("floating")).is_err()
        );
        assert!(validate_setting(store::ANDROID_PILL_POSITION, &serde_json::Value::Null).is_err());
    }

    #[test]
    fn accent_color_accepts_hex_or_default() {
        assert!(validate_setting(store::ACCENT_COLOR, &serde_json::json!("#4F7FD8")).is_ok());
        assert!(validate_setting(store::ACCENT_COLOR, &serde_json::Value::Null).is_ok());
        assert!(validate_setting(store::ACCENT_COLOR, &serde_json::json!("blue")).is_err());
        assert!(validate_setting(store::ACCENT_COLOR, &serde_json::json!("#1234")).is_err());
    }

    #[test]
    fn custom_theme_setting_and_mode_are_accepted() {
        let theme = serde_json::json!({"background": "#101315", "foreground": "#cacccc"});
        assert!(validate_setting(store::CUSTOM_THEME, &theme).is_ok());
        assert!(validate_setting(store::CUSTOM_THEME, &serde_json::Value::Null).is_ok());
        assert!(validate_setting(
            store::CUSTOM_THEME,
            &serde_json::json!({"background": "#101315"})
        )
        .is_err());
        assert!(validate_setting(store::APPEARANCE_MODE, &serde_json::json!("custom")).is_ok());
    }

    #[test]
    fn saved_themes_are_readable_exportable_and_validated() {
        let theme = serde_json::json!({
            "id": "t-night", "name": "Night",
            "palette": {"background": "#101315", "foreground": "#cacccc"},
            "accent": "#4f7fd8"
        });
        assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::json!([theme.clone()])).is_ok());
        assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::Value::Null).is_ok());
        assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::json!([])).is_ok());
        assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::json!([theme.clone(), theme.clone()])).is_err());
        assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::json!(vec![theme.clone(); 25])).is_err());
        for (key, invalid) in [
            ("name", serde_json::json!(" ")),
            ("name", serde_json::json!("n".repeat(41))),
            ("id", serde_json::json!("invalid id")),
            ("accent", serde_json::json!("blue")),
            ("palette", serde_json::json!({"background": "#101315"})),
            ("extra", serde_json::json!(true)),
        ] {
            let mut next = theme.clone();
            next[key] = invalid;
            assert!(validate_setting(store::CUSTOM_THEMES, &serde_json::json!([next])).is_err(), "{key}");
        }
        let spec = SETTING_SPECS.iter().find(|spec| spec.key == store::CUSTOM_THEMES).unwrap();
        assert!(spec.readable && spec.exportable);
        assert!(!crate::sync::engine::SYNCABLE_SETTINGS.contains(&store::CUSTOM_THEMES));
    }
}
// ---------- generic settings ----------

/// Enables a process-local failure mode for exercising full-disk error UI.
/// The flag intentionally never touches settings.json, so it can be disabled
/// even while simulated saves are failing.
#[tauri::command]
pub fn set_storage_full_simulation(enabled: bool) -> Result<(), String> {
    store::set_storage_full_simulation(enabled);
    Ok(())
}

#[tauri::command]
pub fn get_storage_full_simulation() -> bool {
    store::storage_full_simulation_enabled()
}

#[tauri::command]
pub async fn save_setting(
    app: AppHandle,
    key: String,
    mut value: serde_json::Value,
) -> Result<(), String> {
    validate_setting(&key, &value)?;
    let disable_cleanup_cache = key == store::CLEANUP_CACHE_ENABLED && value == false;
    if key == store::CUSTOM_PROVIDERS {
        let providers = crate::api::custom::normalize_list(&value)?;
        value = serde_json::to_value(&providers)
            .map_err(|_| "Could not encode custom providers.".to_string())?;
    }
    let history_prune_days = if key == store::HISTORY_RETENTION {
        value.as_str().and_then(store::history_retention_days)
    } else {
        None
    };
    let sound_effects_volume = if key == store::SOUND_EFFECTS_VOLUME {
        value.as_f64().map(|volume| (volume as f32) / 100.0)
    } else {
        None
    };
    let ruin_accessibility = if key == store::RUIN_ACCESSIBILITY {
        value.as_bool()
    } else {
        None
    };
    let settings = store::settings_handle(&app)?;
    let credential_app = app.clone();
    let key_clone = key.clone();
    let save_result = run_blocking("save_setting", move || {
        if store::storage_full_simulation_enabled() {
            return Err(format!(
                "{}: simulated settings write failure",
                store::STORAGE_FULL_ERROR
            ));
        }
        if key_clone == store::CUSTOM_PROVIDERS {
            let providers = crate::api::custom::normalize_list(&value)?;
            settings
                .save_values_with_commit(
                    [(key_clone, value)],
                    |previous| {
                        clear_changed_custom_provider_keys(&credential_app, previous, &providers)
                    },
                    || Ok(()),
                )
                .map(|_| ())
        } else if key_clone == store::GITHUB_USERNAME {
            save_github_username(&settings, value)
        } else if key_clone == store::CONTEXTUAL_FORMATTING {
            settings.save_values([
                (store::CONTEXTUAL_FORMATTING, value.clone()),
                (store::CONTEXTUAL_CAPS, value.clone()),
                (store::AUTO_SPACING, value),
            ])
        } else {
            settings.save_value(key_clone, value)
        }
    })
    .await;
    save_result?;
    if disable_cleanup_cache {
        let db = db_state(&app);
        run_blocking("disable_cleanup_cache", move || {
            db::cleanup_cache_clear_all(&db).map_err(|e| e.to_string())
        })
        .await?;
    }

    // LAN sync: stamp the change so peers LWW-compare it, and nudge the sync
    // manager to schedule a session. Both are best-effort — a sync failure
    // never blocks saving a setting.
    if crate::sync::engine::SYNCABLE_SETTINGS.contains(&key.as_str()) {
        let db = app.state::<DbHandle>().inner().clone();
        let key_for_stamp = key.clone();
        let stamped = run_blocking("save_setting_stamp", move || {
            let conn = db
                .lock()
                .map_err(|_| "database lock poisoned".to_string())?;
            crate::sync::engine::record_local_setting_change(&conn, &key_for_stamp)
                .map_err(|e| e.to_string())
        })
        .await;
        if let Err(err) = stamped {
            log::warn!("sync: failed to stamp setting change: {err}");
        }
        if let Some(manager) = app.try_state::<crate::sync::SyncManager>() {
            manager.mark_dirty();
        }
    }

    if crate::app_tray::setting_updates_runtime_icons(&key) {
        crate::apply_runtime_icons(&app, None);
    }
    if key == store::APPEARANCE_MODE {
        if let Some(mode) = store::settings_handle(&app)?
            .get(&key)
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            let _ = app.emit("verenu:appearance-mode-changed", mode);
        }
    }
    #[cfg(target_os = "windows")]
    if key == store::APPEARANCE_MODE || key == store::CUSTOM_THEME {
        crate::system::windows_titlebar::refresh_for_app(&app);
    }
    if let Some(volume) = sound_effects_volume {
        crate::media::sound::set_volume(volume);
    }

    if key == store::SUB_APP_CAPTURE_HOTKEY {
        apply_sub_app_capture_hotkey(&app);
    }

    if key == store::MIC_MUTE_BUTTON_DICTATION || key == store::MICROPHONE_DEVICE {
        crate::media::mic_mute_trigger::reload(&app);
    }

    if let Some(enabled) = ruin_accessibility {
        let _ = app.emit("verenu:ruin-accessibility-changed", enabled);
    }

    if let Some(days) = history_prune_days {
        let db = app.state::<DbHandle>().inner().clone();
        let settings = store::settings_handle(&app)?;
        let deleted = tokio::task::spawn_blocking(move || {
            db::prune_transcriptions_for_retention(&db, &settings, days)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        if deleted > 0 {
            let _ = app.emit("verenu:history-pruned", ());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn get_setting(app: AppHandle, key: String) -> Result<Option<serde_json::Value>, String> {
    if !is_readable_setting_key(&key) {
        return Err(format!("Unsupported setting key: {key}"));
    }
    Ok(store::settings_handle(&app)?.get(&key))
}

#[derive(serde::Serialize)]
pub struct AllSettings {
    pub clipboard_phrase: Option<String>,
    pub clipboard_phrase_enabled: Option<bool>,
    pub legacy_features_enabled: Option<bool>,
    pub sync_enabled: Option<bool>,
    pub sync_muting_enabled: Option<bool>,
    pub sync_peer_addresses: Option<serde_json::Value>,
    pub ruin_accessibility: Option<bool>,
    pub dev_mode_on_startup: Option<bool>,
    pub transcription_provider: Option<String>,
    pub transcription_model: Option<String>,
    pub transcription_language: Option<String>,
    pub cleanup_provider: Option<String>,
    pub cleanup_model: Option<String>,
    pub transcription_models_by_provider: Option<serde_json::Value>,
    pub cleanup_models_by_provider: Option<serde_json::Value>,
    pub transcription_default_model: Option<String>,
    pub cleanup_default_model: Option<String>,
    pub transcription_fallback_models: Option<Vec<String>>,
    pub dual_transcription_enabled: Option<bool>,
    pub model_selection_mode: Option<String>,
    pub cleanup_fallback_models: Option<Vec<String>>,
    pub advanced_model_ui: Option<bool>,
    pub cleanup_enabled: Option<bool>,
    pub cleanup_cache_enabled: Option<bool>,
    pub noise_reduction: Option<bool>,
    pub mute_audio: Option<bool>,
    pub mic_mute_button_dictation: Option<bool>,
    pub exclusive_mic: Option<bool>,
    pub pause_media_during_dictation: Option<bool>,
    pub play_start_stop_sounds: Option<bool>,
    pub sound_effects_volume: Option<f64>,
    pub setup_progress: Option<serde_json::Value>,
    pub autostart_enabled: Option<bool>,
    pub auto_learn_enabled: Option<bool>,
    pub analytics_enabled: Option<bool>,
    pub force_setup_on_launch: Option<bool>,
    pub contextual_caps_enabled: Option<bool>,
    pub auto_spacing_enabled: Option<bool>,
    pub contextual_formatting_enabled: Option<bool>,
    pub caps_lock_uppercase_enabled: Option<bool>,
    pub mic_gain: Option<f64>,
    pub history_retention: Option<String>,
    pub github_username: Option<String>,
    pub local_model_memory_policy: Option<String>,
    pub microphone_device: Option<String>,
    pub update_dismissed_version: Option<String>,
    pub update_notified_version: Option<String>,
    pub beta_updates_enabled: Option<bool>,
    pub verenu_service_checks_enabled: Option<bool>,
    pub hotkey: Option<Vec<String>>,
    pub sub_app_capture_hotkey: Option<String>,
    pub appearance_mode: Option<String>,
    pub accent_color: Option<String>,
    pub custom_theme: Option<serde_json::Value>,
    pub custom_themes: Option<serde_json::Value>,
    pub android_pill_position: Option<String>,
    pub android_pill_dock_position: Option<String>,
    pub android_pill_cover_keyboard_mic: Option<bool>,
    pub android_pill_hide_offline: Option<bool>,
    pub cleanup_prompt_override: Option<String>,
    pub style_prompt_instructions: Option<serde_json::Value>,
    pub provider_model_cache: Option<serde_json::Value>,
    pub custom_providers: Vec<crate::api::custom::CustomProvider>,
}

// The struct above is the IPC contract (tests/OnePyFone.py parses it); this
// table keeps each field's store key and reader together.
macro_rules! all_settings {
    ($($field:ident = $reader:ident($key:path)),* $(,)?) => {
        fn read_all_settings(s: &store::SettingsSnapshot) -> AllSettings {
            AllSettings { $($field: snapshot_value!(s, $reader, $key),)* }
        }
    };
}

macro_rules! snapshot_value {
    ($s:ident, bool_val, $key:path) => { $s.get($key).and_then(serde_json::Value::as_bool) };
    ($s:ident, str_val, $key:path) => { $s.get($key).and_then(|v| v.as_str().map(String::from)) };
    ($s:ident, f64_val, $key:path) => { $s.get($key).and_then(serde_json::Value::as_f64) };
    ($s:ident, json_val, $key:path) => { $s.get_cloned($key) };
    ($s:ident, str_array_val, $key:path) => {
        $s.get($key).and_then(|v| v.as_array().map(|arr| {
            arr.iter().filter_map(|x| x.as_str().map(String::from)).collect()
        }))
    };
    ($s:ident, custom, $key:path) => { crate::api::custom::parse_stored($s.get($key)) };
}

all_settings! {
    clipboard_phrase = str_val(store::CLIPBOARD_PHRASE),
    clipboard_phrase_enabled = bool_val(store::CLIPBOARD_PHRASE_ENABLED),
    legacy_features_enabled = bool_val(store::LEGACY_FEATURES_ENABLED),
    sync_enabled = bool_val(store::SYNC_ENABLED),
    sync_muting_enabled = bool_val(store::SYNC_MUTING_ENABLED),
    sync_peer_addresses = json_val(store::SYNC_PEER_ADDRESSES),
    ruin_accessibility = bool_val(store::RUIN_ACCESSIBILITY),
    dev_mode_on_startup = bool_val(store::DEV_MODE_ON_STARTUP),
    transcription_provider = str_val(store::TRANSCRIPTION_PROVIDER),
    transcription_model = str_val(store::TRANSCRIPTION_MODEL),
    transcription_language = str_val(store::TRANSCRIPTION_LANGUAGE),
    cleanup_provider = str_val(store::CLEANUP_PROVIDER),
    cleanup_model = str_val(store::CLEANUP_MODEL),
    transcription_models_by_provider = json_val(store::TRANSCRIPTION_MODELS_BY_PROVIDER),
    cleanup_models_by_provider = json_val(store::CLEANUP_MODELS_BY_PROVIDER),
    transcription_default_model = str_val(store::TRANSCRIPTION_DEFAULT_MODEL),
    cleanup_default_model = str_val(store::CLEANUP_DEFAULT_MODEL),
    transcription_fallback_models = str_array_val(store::TRANSCRIPTION_FALLBACK_MODELS),
    dual_transcription_enabled = bool_val(store::DUAL_TRANSCRIPTION_ENABLED),
    model_selection_mode = str_val(store::MODEL_SELECTION_MODE),
    cleanup_fallback_models = str_array_val(store::CLEANUP_FALLBACK_MODELS),
    advanced_model_ui = bool_val(store::ADVANCED_MODEL_UI),
    cleanup_enabled = bool_val(store::CLEANUP_ENABLED),
    cleanup_cache_enabled = bool_val(store::CLEANUP_CACHE_ENABLED),
    noise_reduction = bool_val(store::NOISE_REDUCTION),
    mute_audio = bool_val(store::MUTE_AUDIO),
    mic_mute_button_dictation = bool_val(store::MIC_MUTE_BUTTON_DICTATION),
    exclusive_mic = bool_val(store::EXCLUSIVE_MIC),
    pause_media_during_dictation = bool_val(store::PAUSE_MEDIA_DURING_DICTATION),
    play_start_stop_sounds = bool_val(store::PLAY_START_STOP_SOUNDS),
    sound_effects_volume = f64_val(store::SOUND_EFFECTS_VOLUME),
    setup_progress = json_val(store::SETUP_PROGRESS),
    autostart_enabled = bool_val(store::AUTOSTART_ENABLED),
    auto_learn_enabled = bool_val(store::AUTO_LEARN_ENABLED),
    analytics_enabled = bool_val(store::ANALYTICS_ENABLED),
    force_setup_on_launch = bool_val(store::FORCE_SETUP_ON_LAUNCH),
    contextual_caps_enabled = bool_val(store::CONTEXTUAL_CAPS),
    auto_spacing_enabled = bool_val(store::AUTO_SPACING),
    contextual_formatting_enabled = bool_val(store::CONTEXTUAL_FORMATTING),
    caps_lock_uppercase_enabled = bool_val(store::CAPS_LOCK_UPPERCASE),
    mic_gain = f64_val(store::MIC_GAIN),
    history_retention = str_val(store::HISTORY_RETENTION),
    github_username = str_val(store::GITHUB_USERNAME),
    local_model_memory_policy = str_val(store::LOCAL_MODEL_MEMORY_POLICY),
    microphone_device = str_val(store::MICROPHONE_DEVICE),
    update_dismissed_version = str_val(store::UPDATE_DISMISSED_VERSION),
    update_notified_version = str_val(store::UPDATE_NOTIFIED_VERSION),
    beta_updates_enabled = bool_val(store::BETA_UPDATES_ENABLED),
    verenu_service_checks_enabled = bool_val(store::VERENU_SERVICE_CHECKS_ENABLED),
    hotkey = str_array_val(store::HOTKEY),
    sub_app_capture_hotkey = str_val(store::SUB_APP_CAPTURE_HOTKEY),
    appearance_mode = str_val(store::APPEARANCE_MODE),
    accent_color = str_val(store::ACCENT_COLOR),
    custom_theme = json_val(store::CUSTOM_THEME),
    custom_themes = json_val(store::CUSTOM_THEMES),
    android_pill_position = str_val(store::ANDROID_PILL_POSITION),
    android_pill_dock_position = str_val(store::ANDROID_PILL_DOCK_POSITION),
    android_pill_cover_keyboard_mic = bool_val(store::ANDROID_PILL_COVER_KEYBOARD_MIC),
    android_pill_hide_offline = bool_val(store::ANDROID_PILL_HIDE_OFFLINE),
    cleanup_prompt_override = str_val(store::CLEANUP_PROMPT_OVERRIDE),
    style_prompt_instructions = json_val(store::STYLE_PROMPT_INSTRUCTIONS),
    provider_model_cache = json_val(store::PROVIDER_MODEL_CACHE),
    custom_providers = custom(store::CUSTOM_PROVIDERS),
}

#[derive(serde::Serialize)]
pub struct CleanupCacheStatus {
    pub entry_count: i64,
    pub payload_bytes: i64,
    pub session: crate::pipeline::cache::CleanupCacheMetrics,
}

#[tauri::command]
pub async fn get_all_settings(app: AppHandle) -> Result<AllSettings, String> {
    Ok(read_all_settings(&store::settings_snapshot(&app)?))
}

#[cfg(test)]
mod github_setting_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_username_is_readable_but_not_exported_or_synced() {
        assert!(validate_setting(store::GITHUB_USERNAME, &json!("octocat")).is_ok());
        assert!(validate_setting(store::GITHUB_USERNAME, &json!("")).is_ok());
        assert!(validate_setting(store::GITHUB_USERNAME, &json!("a repo:private")).is_err());
        assert!(validate_setting(store::GITHUB_COMMIT_CACHE, &json!({})).is_err());
        assert!(is_readable_setting_key(store::GITHUB_USERNAME));
        assert!(!is_exportable_setting_key(store::GITHUB_USERNAME));
        assert!(!crate::sync::engine::SYNCABLE_SETTINGS.contains(&store::GITHUB_USERNAME));
        assert!(!is_readable_setting_key(store::GITHUB_COMMIT_CACHE));
        assert!(!is_exportable_setting_key(store::GITHUB_COMMIT_CACHE));
    }

    #[test]
    fn same_account_save_preserves_cache_and_disconnect_clears_it() {
        let path = std::env::temp_dir().join(format!(
            "verenu_github_username_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        settings
            .save_values([
                (store::GITHUB_USERNAME, json!("octocat")),
                (store::GITHUB_COMMIT_CACHE, json!({"counts": [3]})),
            ])
            .unwrap();

        save_github_username(&settings, json!("OCTOCAT")).unwrap();
        assert_eq!(settings.get(store::GITHUB_USERNAME), Some(json!("octocat")));
        assert_eq!(
            settings.get(store::GITHUB_COMMIT_CACHE),
            Some(json!({"counts": [3]}))
        );

        save_github_username(&settings, json!("")).unwrap();
        assert_eq!(
            settings.get(store::GITHUB_COMMIT_CACHE),
            Some(serde_json::Value::Null)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn explicit_disconnect_materializes_empty_username_when_none_was_saved() {
        let path = std::env::temp_dir().join(format!(
            "verenu_github_empty_username_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        assert_eq!(settings.get(store::GITHUB_USERNAME), None);

        save_github_username(&settings, json!("")).unwrap();

        assert_eq!(settings.get(store::GITHUB_USERNAME), Some(json!("")));
        assert_eq!(
            settings.get(store::GITHUB_COMMIT_CACHE),
            Some(serde_json::Value::Null)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn case_only_save_keeps_in_flight_cache_owner_valid_without_existing_cache() {
        let path = std::env::temp_dir().join(format!(
            "verenu_github_inflight_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        settings
            .save_value(store::GITHUB_USERNAME, json!("octocat"))
            .unwrap();

        save_github_username(&settings, json!("OCTOCAT")).unwrap();
        assert_eq!(settings.get(store::GITHUB_USERNAME), Some(json!("octocat")));
        assert_eq!(settings.get(store::GITHUB_COMMIT_CACHE), None);

        assert!(settings
            .save_value_if_owner_matches_when(
                store::GITHUB_USERNAME,
                &json!("octocat"),
                store::GITHUB_COMMIT_CACHE,
                json!({"counts": [1]}),
                || true,
            )
            .unwrap());
        assert_eq!(
            settings.get(store::GITHUB_COMMIT_CACHE),
            Some(json!({"counts": [1]}))
        );

        save_github_username(&settings, json!("other-account")).unwrap();
        assert!(!settings
            .save_value_if_owner_matches_when(
                store::GITHUB_USERNAME,
                &json!("octocat"),
                store::GITHUB_COMMIT_CACHE,
                json!({"counts": [2]}),
                || true,
            )
            .unwrap());
        assert_eq!(
            settings.get(store::GITHUB_COMMIT_CACHE),
            Some(serde_json::Value::Null)
        );
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_nullable_typed_fields_and_additive_flags() {
        let snapshot = store::SettingsSnapshot::from_pairs([
            (store::ANALYTICS_ENABLED.into(), json!(false)),
            (store::FORCE_SETUP_ON_LAUNCH.into(), json!(true)),
            (store::CLEANUP_ENABLED.into(), json!("invalid")),
            (store::MIC_GAIN.into(), json!(3.5)),
            (store::HOTKEY.into(), json!(["Ctrl", 42, "Super"])),
            (store::TRANSCRIPTION_MODELS_BY_PROVIDER.into(), json!({"groq": ["speech"]})),
            (store::SETUP_PROGRESS.into(), json!({"step": 4, "provider": "groq"})),
            (store::ANDROID_PILL_DOCK_POSITION.into(), json!("bottom")),
        ]);
        let output = serde_json::to_value(read_all_settings(&snapshot)).unwrap();
        assert_eq!(output["analytics_enabled"], false);
        assert_eq!(output["force_setup_on_launch"], true);
        assert_eq!(output["cleanup_enabled"], serde_json::Value::Null);
        assert_eq!(output["mic_gain"], 3.5);
        assert_eq!(output["hotkey"], json!(["Ctrl", "Super"]));
        assert_eq!(output["transcription_models_by_provider"], json!({"groq": ["speech"]}));
        assert_eq!(output["setup_progress"], json!({"step": 4, "provider": "groq"}));
        assert_eq!(output["android_pill_dock_position"], "bottom");
        assert_eq!(output["custom_providers"], json!([]));
        assert_eq!(output["transcription_provider"], serde_json::Value::Null);
    }
}

#[cfg(test)]
mod provider_model_cache_tests {
    use super::*;
    use serde_json::json;

    fn well_formed() -> serde_json::Value {
        json!({
            "groq": {
                "ids": ["whisper-large-v3"],
                "everSeen": ["whisper-large-v3", "llama-3.3-70b-versatile"],
                "lastSuccessAt": 1_700_000_000_000u64,
                "lastAttemptAt": 1_700_000_000_000u64,
                "lastError": null,
                "missing": {
                    "groq/llama-3.3-70b-versatile": {
                        "count": 1,
                        "lastCountedAt": 1_700_000_000_000u64
                    }
                }
            }
        })
    }

    fn check(value: &serde_json::Value) -> Result<(), String> {
        validate_setting(store::PROVIDER_MODEL_CACHE, value)
    }

    #[test]
    fn accepts_a_well_formed_cache() {
        check(&well_formed()).expect("well-formed cache should validate");
    }

    #[test]
    fn validates_persisted_capabilities_for_new_providers() {
        let mut value = well_formed();
        value["openrouter"] = value["groq"].clone();
        value["openrouter"]["metadata"] = json!({"org/new:free":{"label":"New model","tasks":["cleanup"]}});
        value["openrouter"]["warning"] = json!(null);
        assert!(check(&value).is_ok());
        value["openrouter"]["metadata"]["org/new:free"]["tasks"] = json!(["unknown"]);
        assert!(check(&value).is_err());
    }

    #[test]
    fn accepts_an_empty_cache_and_an_error_string() {
        check(&json!({})).expect("empty cache should validate");
        let mut value = well_formed();
        value["groq"]["lastError"] = json!("offline");
        check(&value).expect("a recorded error should validate");
    }

    #[test]
    fn rejects_unknown_provider_keys() {
        let mut value = well_formed();
        value["not-a-provider"] = value["groq"].clone();
        assert!(check(&value).is_err());
    }

    #[test]
    fn rejects_non_string_ids() {
        let mut value = well_formed();
        value["groq"]["ids"] = json!([1, 2]);
        assert!(check(&value).is_err());
        value["groq"]["ids"] = json!("whisper-large-v3");
        assert!(check(&value).is_err());
    }

    #[test]
    fn rejects_missing_and_non_finite_timestamps() {
        let mut value = well_formed();
        value["groq"]["lastSuccessAt"] = json!(-1);
        assert!(check(&value).is_err());

        let mut value = well_formed();
        value["groq"]
            .as_object_mut()
            .unwrap()
            .remove("lastAttemptAt");
        assert!(check(&value).is_err());
    }

    #[test]
    fn rejects_malformed_missing_counters() {
        let mut value = well_formed();
        value["groq"]["missing"] = json!({ "groq/x": 2 });
        assert!(check(&value).is_err());

        let mut value = well_formed();
        value["groq"]["missing"] = json!({ "groq/x": { "count": -1, "lastCountedAt": 0 } });
        assert!(check(&value).is_err());
    }

    #[test]
    fn is_readable_but_not_exportable() {
        assert!(is_readable_setting_key(store::PROVIDER_MODEL_CACHE));
        assert!(!is_exportable_setting_key(store::PROVIDER_MODEL_CACHE));
        assert!(is_readable_setting_key(store::SETUP_PROGRESS));
        assert!(!is_exportable_setting_key(store::SETUP_PROGRESS));
    }
}

/// Applies the saved sub-app capture chord (or the platform default).
pub fn apply_sub_app_capture_hotkey(app: &AppHandle) {
    let chord = store::settings_handle(app)
        .ok()
        .and_then(|settings| settings.get(store::SUB_APP_CAPTURE_HOTKEY))
        .and_then(|value| {
            value
                .as_str()
                .and_then(crate::core::hotkey::chord::Chord::parse)
        })
        .unwrap_or_else(crate::core::hotkey::chord::Chord::default_for_platform);
    crate::core::hotkey::set_sub_app_capture_chord(chord);
}
