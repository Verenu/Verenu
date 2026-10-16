
// The bridge server only starts on Android (see main.rs setup) and is
// exercised by unit tests everywhere else; allow its (then-)unused surface
// on desktop non-test builds so `clippy -D warnings` stays green there.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub mod bridge;

#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub mod local_ai;

#[cfg(target_os = "android")]
pub mod updater;

#[cfg(target_os = "android")]
pub mod permissions_plugin {
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use tauri::{plugin::PluginHandle, AppHandle, Manager, Runtime};

    const PLUGIN_IDENTIFIER: &str = "com.verenu.app";
    const PLUGIN_CLASS: &str = "VerenuPermissionPlugin";

    pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
        tauri::plugin::Builder::new("verenu-permissions")
            .setup(|app, api| {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, PLUGIN_CLASS)?;
                app.manage(handle);
                Ok(())
            })
            .build()
    }

    pub async fn run<R: Runtime, T: DeserializeOwned>(
        app: &AppHandle<R>,
        command: &str,
        payload: impl serde::Serialize,
    ) -> Result<T, String> {
        let handle = app.state::<PluginHandle<R>>();
        handle
            .run_mobile_plugin_async(command, payload)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn snapshot<R: Runtime>(app: &AppHandle<R>) -> Result<Value, String> {
        // Android's generated plugin dispatcher expects an object payload even
        // for commands with no arguments. Passing unit serializes to `null`,
        // which is rejected before the Kotlin command is invoked on some
        // Tauri Android versions. Keep the no-argument call JSON-shaped.
        run(app, "snapshot", serde_json::json!({})).await
    }
}

#[cfg(target_os = "android")]
pub mod security_plugin {
    use tauri::{Manager, Runtime};

    const PLUGIN_IDENTIFIER: &str = "com.verenu.app";
    const PLUGIN_CLASS: &str = "VerenuSecurityPlugin";

    pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
        tauri::plugin::Builder::new("verenu-security")
            .setup(|app, api| {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, PLUGIN_CLASS)?;
                app.manage(handle);
                Ok(())
            })
            .build()
    }
}

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// Minimum SDK Verenu supports on Android.
///
/// API 26 (Android 8.0) is the floor: `EncryptedSharedPreferences` + current
/// `androidx.security` MasterKey paths, notification channels, and the
/// `AccessibilityService.SoftKeyboardController` show/hide callbacks Verenu's
/// overlay relies on are all available from here.
/// Placements anchored to the screen rather than the keyboard.
pub const ANDROID_PILL_SCREEN_POSITIONS: [&str; 10] = [
    "screen-top-left",
    "screen-top",
    "screen-top-right",
    "screen-left",
    "screen-middle",
    "screen-right",
    "screen-bottom-left",
    "screen-bottom",
    "screen-bottom-right",
    "punch-hole",
];

pub const ANDROID_PILL_POSITIONS: [&str; 13] = [
    "keyboard-center",
    "keyboard-left",
    "keyboard-right",
    "screen-top-left",
    "screen-top",
    "screen-top-right",
    "screen-left",
    "screen-middle",
    "screen-right",
    "screen-bottom-left",
    "screen-bottom",
    "screen-bottom-right",
    "punch-hole",
];
pub const DEFAULT_ANDROID_PILL_POSITION: &str = "keyboard-center";

pub const DEFAULT_ANDROID_PILL_DOCK_POSITION: &str = "screen-bottom";

pub const ANDROID_MIN_SDK: u32 = 26;

pub const ANDROID_TARGET_SDK: u32 = 36;

pub const ANDROID_SUPPORTED_ABI: &str = "arm64-v8a";

pub const LOCAL_AI_ANDROID_UNSUPPORTED_REASON: &str = "Local AI needs Android 9 or newer and a build with bundled inference runtimes. Update Android or reinstall a build with local AI support. Cloud providers are still available.";

pub fn local_ai_supported_on_android() -> bool {
    local_ai::supported()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionStrategy {
    DirectAccessibility,
    ClipboardFallback,
}

pub fn decide_insertion_strategy(
    has_editable_focus: bool,
    supports_set_text: bool,
) -> InsertionStrategy {
    if has_editable_focus && supports_set_text {
        InsertionStrategy::DirectAccessibility
    } else {
        InsertionStrategy::ClipboardFallback
    }
}

pub fn should_show_overlay(keyboard_visible: bool, has_editable_focus: bool) -> bool {
    keyboard_visible && has_editable_focus
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayState {
    Hidden,
    VisibleIdle,
    Recording,
    Transcribing,
    Cleaning,
    Inserting,
    Error,
    Cancelled,
}

impl OverlayState {
    pub fn is_visible(self) -> bool {
        !matches!(self, OverlayState::Hidden)
    }

    pub fn is_dictation_active(self) -> bool {
        matches!(
            self,
            OverlayState::Recording
                | OverlayState::Transcribing
                | OverlayState::Cleaning
                | OverlayState::Inserting
        )
    }

    pub fn on_keyboard_hidden(self) -> OverlayState {
        match self {
            OverlayState::Recording
            | OverlayState::Transcribing
            | OverlayState::Cleaning
            | OverlayState::Inserting => self,
            _ => OverlayState::Hidden,
        }
    }

    pub fn on_keyboard_shown(self) -> OverlayState {
        match self {
            OverlayState::Hidden => OverlayState::VisibleIdle,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AndroidPermission {
    Microphone,
    AccessibilityService,
    BatteryExemption,
    Notifications,
}

pub fn permission_rationale(permission: AndroidPermission) -> &'static str {
    match permission {
        AndroidPermission::Microphone => {
            "Verenu needs microphone access to record your dictation. Audio is transcribed and then discarded; recordings never leave the device except as an encrypted transcription request to the provider you configured."
        }
        AndroidPermission::AccessibilityService => {
            "Verenu uses an Accessibility Service to see when the keyboard is open, show the dictation pill above it, read which app you're typing in (for per-app Contexts), and insert the dictated text into the focused field. It never reads passwords, never clicks for you, and never runs when the keyboard is closed."
        }
        AndroidPermission::BatteryExemption => {
            "Some manufacturers aggressively kill background audio. Exempting Verenu from battery optimization keeps recordings from being cut off mid-sentence. Verenu only records after you tap the pill, and stops when you tap Stop."
        }
        AndroidPermission::Notifications => {
            "Verenu posts a status notification while recording so Android keeps the microphone alive and you can see — and stop — a dictation from anywhere."
        }
    }
}

pub fn permission_is_required(permission: AndroidPermission) -> bool {
    matches!(
        permission,
        AndroidPermission::Microphone | AndroidPermission::AccessibilityService
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrant {
    Granted,
    Denied,
    NotAsked,
    PermanentlyDenied,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AndroidPermissionSnapshot {
    pub microphone: PermissionGrant,
    pub accessibility_service: PermissionGrant,
    pub battery_exemption: PermissionGrant,
    pub notifications: PermissionGrant,
}

impl AndroidPermissionSnapshot {
    pub fn is_functional(&self) -> bool {
        matches!(self.microphone, PermissionGrant::Granted)
            && matches!(self.accessibility_service, PermissionGrant::Granted)
    }

    pub fn missing_required(&self) -> Vec<AndroidPermission> {
        let mut missing = Vec::new();
        if !matches!(self.microphone, PermissionGrant::Granted) {
            missing.push(AndroidPermission::Microphone);
        }
        if !matches!(self.accessibility_service, PermissionGrant::Granted) {
            missing.push(AndroidPermission::AccessibilityService);
        }
        missing
    }
}

#[cfg(any(target_os = "android", test))]
pub fn is_browser_package(package: &str) -> bool {
    matches!(
        package.trim().to_ascii_lowercase().as_str(),
        "com.android.chrome"
            | "com.chrome.beta"
            | "com.chrome.dev"
            | "com.chrome.canary"
            | "org.mozilla.firefox"
            | "org.mozilla.firefox_beta"
            | "org.mozilla.fenix"
            | "com.brave.browser"
            | "com.microsoft.emmx"
            | "com.sec.android.app.sbrowser"
            | "com.opera.browser"
            | "com.opera.mini.native"
            | "com.duckduckgo.mobile.android"
            | "com.vivaldi.browser"
            | "com.kiwibrowser.browser"
    )
}

pub fn context_label_for_package(package: &str) -> (String, bool) {
    let package = package.trim();
    if package.is_empty() {
        return ("Everywhere".to_string(), true);
    }
    let known: &[(&str, &str)] = &[
        ("com.google.android.gm", "Gmail"),
        ("com.google.android.apps.docs", "Docs"),
        ("com.microsoft.office.outlook", "Outlook"),
        ("com.microsoft.office.word", "Word"),
        ("com.slack", "Slack"),
        ("com.whatsapp", "WhatsApp"),
        ("com.telegram.messenger", "Telegram"),
        ("com.google.android.apps.messaging", "Messages"),
        ("com.android.chrome", "Chrome"),
        ("org.mozilla.firefox", "Firefox"),
        ("com.microsoft.emmx", "Edge"),
        ("com.google.android.keep", "Keep"),
        ("com.evernote", "Evernote"),
        ("com.notion.id", "Notion"),
        ("com.twitter.android", "X"),
        ("com.instagram.android", "Instagram"),
        ("com.linkedin.android", "LinkedIn"),
        ("com.reddit.frontpage", "Reddit"),
        ("com.google.android.youtube", "YouTube"),
        ("com.spotify.music", "Spotify"),
    ];
    for (pkg, label) in known {
        if package == *pkg {
            return (label.to_string(), false);
        }
    }
    let fallback = package
        .rsplit('.')
        .next()
        .unwrap_or(package)
        .replace('_', " ");
    let mut chars = fallback.chars();
    let title = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Everywhere".to_string(),
    };
    (title, true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthClass {
    Compact,
    Medium,
    Expanded,
}

pub fn width_class_for_dp(width_dp: f32) -> WidthClass {
    if width_dp < 600.0 {
        WidthClass::Compact
    } else if width_dp < 840.0 {
        WidthClass::Medium
    } else {
        WidthClass::Expanded
    }
}

pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

pub fn credential_input(provider: &str, key: &str) -> Result<(String, String), String> {
    let canonical_provider = match provider.trim().to_ascii_lowercase().as_str() {
        crate::data::store::GROQ => crate::data::store::GROQ,
        crate::data::store::OPENAI => crate::data::store::OPENAI,
        crate::data::store::GOOGLE => crate::data::store::GOOGLE,
        crate::data::store::ASSEMBLYAI => crate::data::store::ASSEMBLYAI,
        crate::data::store::OPENROUTER => crate::data::store::OPENROUTER,
        crate::data::store::XAI => crate::data::store::XAI,
        _ => return Err("Unknown provider".to_string()),
    };
    let normalized_key = key
        .trim()
        .trim_end_matches(|c: char| c == '\0' || c.is_control())
        .trim();
    if normalized_key.len() > MAX_CREDENTIAL_BYTES {
        return Err("Credential is too large".to_string());
    }
    Ok((canonical_provider.to_string(), normalized_key.to_string()))
}

fn credential_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn protected_credential_providers() -> &'static Mutex<HashSet<String>> {
    static PROTECTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PROTECTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn cache_key(provider: &str) -> String {
    provider.trim().to_lowercase()
}

pub fn provide_credential(provider: &str, key: &str) {
    let key = key.trim().to_string();
    let provider = cache_key(provider);
    let protected = protected_credential_providers()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if protected.contains(&provider) {
        return;
    }
    drop(protected);
    let mut cache = credential_cache().lock().unwrap_or_else(|e| e.into_inner());
    if key.is_empty() {
        cache.remove(&provider);
    } else {
        cache.insert(provider, key);
    }
}

pub fn replace_credential(provider: &str, key: &str) {
    let provider = cache_key(provider);
    let key = key.trim().to_string();
    let mut cache = credential_cache().lock().unwrap_or_else(|e| e.into_inner());
    if key.is_empty() {
        cache.remove(&provider);
    } else {
        cache.insert(provider.clone(), key);
    }
    drop(cache);
    protected_credential_providers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider);
}

pub fn cached_credential(provider: &str) -> String {
    credential_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&cache_key(provider))
        .cloned()
        .unwrap_or_default()
}

pub fn has_cached_credential(provider: &str) -> bool {
    !cached_credential(provider).is_empty()
}

pub fn clear_credentials() {
    credential_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    protected_credential_providers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

#[cfg(test)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_positions_stay_in_step_with_the_kotlin_overlay() {
        let kotlin = include_str!("../../android/kotlin/com/verenu/app/VerenuPillPlacement.kt");
        for position in ANDROID_PILL_POSITIONS {
            assert!(
                kotlin.contains(&format!("\"{position}\"")),
                "VerenuPillPlacement.kt does not know {position}"
            );
        }
        for position in ANDROID_PILL_SCREEN_POSITIONS {
            assert!(
                ANDROID_PILL_POSITIONS.contains(&position),
                "{position} is a dock position but not a valid pill position"
            );
        }
        assert!(kotlin.contains(&format!("\"{DEFAULT_ANDROID_PILL_POSITION}\"")));
        assert!(kotlin.contains(&format!("\"{DEFAULT_ANDROID_PILL_DOCK_POSITION}\"")));
        assert!(ANDROID_PILL_SCREEN_POSITIONS.contains(&DEFAULT_ANDROID_PILL_DOCK_POSITION));
    }

    #[test]
    fn browser_packages_are_recognized_case_insensitively() {
        assert!(is_browser_package("com.android.chrome"));
        assert!(is_browser_package(" COM.BRAVE.BROWSER "));
        assert!(is_browser_package("org.mozilla.firefox"));
        assert!(!is_browser_package("com.slack"));
        assert!(!is_browser_package(""));
    }

    #[test]
    fn overlay_only_shows_over_editable_keyboard() {
        assert!(should_show_overlay(true, true));
        assert!(!should_show_overlay(true, false));
        assert!(!should_show_overlay(false, true));
        assert!(!should_show_overlay(false, false));
    }

    #[test]
    fn keyboard_hide_clears_idle_but_preserves_flight() {
        assert_eq!(
            OverlayState::VisibleIdle.on_keyboard_hidden(),
            OverlayState::Hidden
        );
        assert_eq!(
            OverlayState::Hidden.on_keyboard_hidden(),
            OverlayState::Hidden
        );
        assert_eq!(
            OverlayState::Error.on_keyboard_hidden(),
            OverlayState::Hidden
        );
        assert_eq!(
            OverlayState::Recording.on_keyboard_hidden(),
            OverlayState::Recording
        );
        assert_eq!(
            OverlayState::Transcribing.on_keyboard_hidden(),
            OverlayState::Transcribing
        );
        assert_eq!(
            OverlayState::Cleaning.on_keyboard_hidden(),
            OverlayState::Cleaning
        );
        assert_eq!(
            OverlayState::Inserting.on_keyboard_hidden(),
            OverlayState::Inserting
        );
    }

    #[test]
    fn keyboard_show_wakes_hidden_pill() {
        assert_eq!(
            OverlayState::Hidden.on_keyboard_shown(),
            OverlayState::VisibleIdle
        );
        assert_eq!(
            OverlayState::Recording.on_keyboard_shown(),
            OverlayState::Recording
        );
    }

    #[test]
    fn rapid_keyboard_cycles_settle_hidden() {
        let mut state = OverlayState::Hidden;
        for _ in 0..5 {
            state = state.on_keyboard_shown();
            assert!(state.is_visible());
            assert!(!state.is_dictation_active());
            state = state.on_keyboard_hidden();
        }
        assert_eq!(state, OverlayState::Hidden);
    }

    #[test]
    fn insertion_prefers_direct_accessibility() {
        assert_eq!(
            decide_insertion_strategy(true, true),
            InsertionStrategy::DirectAccessibility
        );
        assert_eq!(
            decide_insertion_strategy(true, false),
            InsertionStrategy::ClipboardFallback
        );
        assert_eq!(
            decide_insertion_strategy(false, true),
            InsertionStrategy::ClipboardFallback
        );
        assert_eq!(
            decide_insertion_strategy(false, false),
            InsertionStrategy::ClipboardFallback
        );
    }

    #[test]
    fn credential_input_canonicalizes_provider_and_bounds_secret() {
        let (provider, key) = credential_input("  GROQ ", "  gsk-test\0\r\n  ").unwrap();
        assert_eq!(provider, "groq");
        assert_eq!(key, "gsk-test");
        assert!(credential_input("unknown", "key").is_err());
        assert!(credential_input("groq", &"x".repeat(MAX_CREDENTIAL_BYTES + 1)).is_err());
    }

    #[test]
    fn permission_snapshot_gates_on_mic_and_accessibility() {
        let blocked = AndroidPermissionSnapshot {
            microphone: PermissionGrant::Granted,
            accessibility_service: PermissionGrant::Denied,
            battery_exemption: PermissionGrant::Granted,
            notifications: PermissionGrant::Granted,
        };
        assert!(!blocked.is_functional());
        assert_eq!(
            blocked.missing_required(),
            vec![AndroidPermission::AccessibilityService]
        );

        let functional = AndroidPermissionSnapshot {
            microphone: PermissionGrant::Granted,
            accessibility_service: PermissionGrant::Granted,
            battery_exemption: PermissionGrant::Denied,
            notifications: PermissionGrant::NotAsked,
        };
        assert!(functional.is_functional());
        assert!(functional.missing_required().is_empty());
    }

    #[test]
    fn permission_rationale_covers_every_gate() {
        for permission in [
            AndroidPermission::Microphone,
            AndroidPermission::AccessibilityService,
            AndroidPermission::BatteryExemption,
            AndroidPermission::Notifications,
        ] {
            let rationale = permission_rationale(permission);
            assert!(
                rationale.len() > 40,
                "{permission:?} needs a real rationale"
            );
        }
        assert!(permission_is_required(AndroidPermission::Microphone));
        assert!(permission_is_required(
            AndroidPermission::AccessibilityService
        ));
        assert!(!permission_is_required(AndroidPermission::BatteryExemption));
        assert!(!permission_is_required(AndroidPermission::Notifications));
    }

    #[test]
    fn context_labels_favor_known_apps() {
        assert_eq!(
            context_label_for_package("com.google.android.gm"),
            ("Gmail".to_string(), false)
        );
        assert_eq!(
            context_label_for_package("com.slack"),
            ("Slack".to_string(), false)
        );
        assert_eq!(
            context_label_for_package(""),
            ("Everywhere".to_string(), true)
        );
        let (label, generic) = context_label_for_package("com.example.my_notes");
        assert_eq!(label, "My notes".to_string());
        assert!(generic);
    }

    #[test]
    fn width_classes_follow_window_not_device() {
        assert_eq!(width_class_for_dp(360.0), WidthClass::Compact);
        assert_eq!(width_class_for_dp(599.0), WidthClass::Compact);
        assert_eq!(width_class_for_dp(500.0), WidthClass::Compact);
        assert_eq!(width_class_for_dp(601.0), WidthClass::Medium);
        assert_eq!(width_class_for_dp(839.0), WidthClass::Medium);
        assert_eq!(width_class_for_dp(841.0), WidthClass::Expanded);
        assert_eq!(width_class_for_dp(700.0), WidthClass::Medium);
    }

    #[test]
    fn credential_cache_round_trips_and_clears() {
        let _serial = test_serial();
        clear_credentials();
        assert_eq!(cached_credential("groq"), "");
        assert!(!has_cached_credential("groq"));
        provide_credential("groq", "  gsk-test  ");
        assert_eq!(cached_credential("GROQ"), "gsk-test");
        assert!(has_cached_credential("groq"));
        provide_credential("groq", "");
        assert!(!has_cached_credential("groq"));
        provide_credential("openai", "sk-test");
        clear_credentials();
        assert!(!has_cached_credential("openai"));
    }

    #[test]
    fn local_ai_requires_initialized_packaged_runtimes() {
        assert!(!local_ai_supported_on_android());
        assert!(LOCAL_AI_ANDROID_UNSUPPORTED_REASON.contains("Android"));
    }
}
