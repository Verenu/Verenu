//! Android platform support for Verenu.
//!
//! This module is the single home for Android-specific business logic that has
//! no Windows/macOS equivalent. Everything in here is deliberately pure (no
//! `cpal`, no `transcribe-rs`, no `windows`/`objc2` crates) so it compiles on
//! every target — including desktop CI — and is unit-testable without a device.
//!
//! # Architecture
//!
//! * The shared dictation pipeline (`crate::pipeline`), providers
//!   (`crate::api`), SQLite storage (`crate::data`), and sync (`crate::sync`)
//!   are reused unchanged on Android. See `docs/ANDROID.md` for the full map.
//! * Android-only differences live behind the small surface in this module:
//!   keyboard-visibility → overlay state machine, accessibility insertion
//!   strategy (direct `ACTION_SET_TEXT` vs clipboard fallback), permission
//!   rationale/status, foreground-package → Context labels, and the in-memory
//!   credential cache described below.
//! * Native Android code (Kotlin) lives under
//!   `src-tauri/gen/android` and owns: the `AccessibilityService`, the
//!   `TYPE_ACCESSIBILITY_OVERLAY` pill above the IME, `AudioRecord` fallback,
//!   `EncryptedSharedPreferences` (Android Keystore) persistence, and runtime
//!   permission flows. Rust never touches Android APIs directly.
//!
//! # Credentials
//!
//! API keys must never sit in plaintext on Android. Durable storage is the
//! Kotlin `VerenuKeystore` (`EncryptedSharedPreferences` backed by
//! AndroidKeyStore). Rust holds keys only in the in-memory
//! [`AndroidCredentialCache`] below, populated at startup by Kotlin via the
//! `android_provide_credential` command (or `POST /v1/credential`) and
//! cleared on lock/revocation. Saves go through `android_keystore_save`,
//! which updates this cache and stages a single-delivery rotation that
//! Kotlin fetches from the bridge; Rust itself writes no secret to disk.
//!
//! # Local AI
//!
//! [`local_ai_supported_on_android`] returns `false`: neither the
//! `transcribe-rs`/ORT speech models nor the `llama-server` cleanup runtime
//! ship an Android ARM64 build, and their multi-hundred-MB/GB payloads are not
//! realistic on phones. Cloud providers are the supported path; the settings
//! UI hides or clearly marks local options on Android while keeping the code
//! structured so they can be added later (see `docs/ANDROID.md`).

// The bridge server only starts on Android (see main.rs setup) and is
// exercised by unit tests everywhere else; allow its (then-)unused surface
// on desktop non-test builds so `clippy -D warnings` stays green there.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub mod bridge;

/// Tauri mobile-plugin glue for Android's real permission prompts and
/// settings intents. The Kotlin class is copied into the generated project by
/// `scripts/android-sync.mjs`; keeping the registration here makes the Rust
/// command path work in release builds as well as development builds.
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

/// Tauri mobile-plugin glue for durable Android credential writes. The
/// Accessibility service is intentionally not required for Settings to save a
/// key; the Kotlin plugin writes EncryptedSharedPreferences directly.
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
pub const ANDROID_MIN_SDK: u32 = 26;

/// SDK Verenu targets. Kept in sync with `gen/android/app/build.gradle.kts`.
pub const ANDROID_TARGET_SDK: u32 = 36;

/// First-class ARM64 ABI. x86_64 emulator builds work for development, but
/// release testing targets real ARM64 devices.
pub const ANDROID_SUPPORTED_ABI: &str = "arm64-v8a";

/// Why local AI is unavailable on Android. Returned alongside `false` from
/// [`local_ai_supported_on_android`] so the settings UI can show a truthful,
/// non-generic explanation instead of a dead toggle.
pub const LOCAL_AI_ANDROID_UNSUPPORTED_REASON: &str = "On-device speech and cleanup models aren't available on Android yet — the desktop runtimes (ONNX Runtime speech models and the llama-server cleanup runtime) don't ship Android ARM64 builds. Cloud providers work normally; local options will light up here if a compatible mobile runtime lands.";

/// Whether on-device transcription/cleanup runtimes are supported on Android.
///
/// Always `false` today. Kept as a function (not a constant) so a future
/// mobile runtime can switch on capability detection without touching callers.
pub fn local_ai_supported_on_android() -> bool {
    false
}

/// How the final dictated text reaches the focused field on Android.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionStrategy {
    /// The accessibility node accepts `ACTION_SET_TEXT`/`ACTION_PASTE` — the
    /// preferred path. Preserves cursor/selection via `ACTION_SET_SELECTION`.
    DirectAccessibility,
    /// The app blocks direct accessibility edits (banking apps, some
    /// WebViews, DRM fields). Fall back to clipboard insertion and tell the
    /// user what happened so they can paste manually.
    ClipboardFallback,
}

/// Decide how to insert, from what the `AccessibilityService` observed.
///
/// * `has_editable_focus` — an editable `AccessibilityNodeInfo` currently has
///   input focus.
/// * `supports_set_text` — that node advertises `ACTION_SET_TEXT` (or a
///   paste-compatible action).
///
/// Anything else → [`InsertionStrategy::ClipboardFallback`]. Pure so both
/// Rust and the Kotlin service (mirrored logic) agree, and unit-tested here.
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

/// Whether the overlay pill should be on screen.
///
/// The overlay is the Android equivalent of the desktop dictation pill: it
/// appears **only** while an IME/soft keyboard is visible over an editable
/// field, and disappears cleanly when the keyboard closes. There is never a
/// permanent floating bubble.
pub fn should_show_overlay(keyboard_visible: bool, has_editable_focus: bool) -> bool {
    keyboard_visible && has_editable_focus
}

/// Overlay pill lifecycle. Mirrors the desktop pill states
/// (`PillApp.svelte`: recording → processing → error/cancelled/…) with an
/// explicit `Hidden` terminal for the keyboard-closed case.
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
    /// Whether the pill is currently on screen in any form.
    pub fn is_visible(self) -> bool {
        !matches!(self, OverlayState::Hidden)
    }

    /// Whether a dictation is in flight (mic open or pipeline running).
    pub fn is_dictation_active(self) -> bool {
        matches!(
            self,
            OverlayState::Recording
                | OverlayState::Transcribing
                | OverlayState::Cleaning
                | OverlayState::Inserting
        )
    }

    /// Keyboard closed: any non-dictating state hides cleanly; an in-flight
    /// dictation keeps its state (audio keeps recording) but the decision to
    /// render is left to the caller via [`should_show_overlay`].
    pub fn on_keyboard_hidden(self) -> OverlayState {
        match self {
            OverlayState::Recording
            | OverlayState::Transcribing
            | OverlayState::Cleaning
            | OverlayState::Inserting => self,
            _ => OverlayState::Hidden,
        }
    }

    /// Keyboard opened over an editable field: a hidden pill becomes idle.
    pub fn on_keyboard_shown(self) -> OverlayState {
        match self {
            OverlayState::Hidden => OverlayState::VisibleIdle,
            other => other,
        }
    }
}

/// Android runtime permissions/features Verenu needs, in onboarding order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AndroidPermission {
    Microphone,
    AccessibilityService,
    BatteryExemption,
    Notifications,
}

/// User-facing rationale for each permission. Shown verbatim in onboarding so
/// the user knows exactly why Verenu asks.
pub fn permission_rationale(permission: AndroidPermission) -> &'static str {
    match permission {
        AndroidPermission::Microphone => {
            "Verenu needs microphone access to record your dictation. Audio is transcribed and then discarded; recordings never leave the device except as an encrypted transcription request to the provider you configured."
        }
        AndroidPermission::AccessibilityService => {
            "Verenu uses an Accessibility Service to see when the keyboard is open, show the dictation pill above it, read which app you're typing in (for per-app Contexts), and insert the dictated text into the focused field. It never reads passwords, never clicks for you, and never runs when the keyboard is closed."
        }
        AndroidPermission::BatteryExemption => {
            "Some manufacturers aggressively kill background audio. Exempting Verenu from battery optimization keeps recordings from being cut off mid-sentence. Verenu still records only while you hold the pill."
        }
        AndroidPermission::Notifications => {
            "Verenu posts a status notification while recording so Android keeps the microphone alive and you can see — and stop — a dictation from anywhere."
        }
    }
}

/// Whether a permission is strictly required for dictation to function.
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

/// Snapshot of the four Android gates, as reported by Kotlin.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AndroidPermissionSnapshot {
    pub microphone: PermissionGrant,
    pub accessibility_service: PermissionGrant,
    pub battery_exemption: PermissionGrant,
    pub notifications: PermissionGrant,
}

impl AndroidPermissionSnapshot {
    /// Dictation can function: mic + accessibility granted.
    pub fn is_functional(&self) -> bool {
        matches!(self.microphone, PermissionGrant::Granted)
            && matches!(self.accessibility_service, PermissionGrant::Granted)
    }

    /// What's still missing before dictation works, in onboarding order.
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

/// Derive a human Context label from an Android foreground package name.
///
/// Returns `(label, is_generic)`: well-known packages map to friendly names
/// ("com.google.android.gm" → "Gmail"); anything else falls back to the
/// last package segment title-cased. Never includes field contents, URLs, or
/// notification text — package name only, matching the desktop
/// executable-name granularity.
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

/// Compact/medium/expanded width classes, mirroring
/// `src/lib/android/viewport.ts`. Both sides use the same 600/840dp
/// breakpoints so Rust-driven layout hints and the Svelte shell agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthClass {
    Compact,
    Medium,
    Expanded,
}

/// Classify an available window width (density-independent pixels) into a
/// Material 3–style window size class. Uses live window dimensions, never a
/// hard-coded device category, so folding/unfolding, rotation, split-screen,
/// and freeform windows all reclassify without restart.
pub fn width_class_for_dp(width_dp: f32) -> WidthClass {
    if width_dp < 600.0 {
        WidthClass::Compact
    } else if width_dp < 840.0 {
        WidthClass::Medium
    } else {
        WidthClass::Expanded
    }
}

// ---------------------------------------------------------------------------
// In-memory credential cache (Android only, by convention on all targets)
// ---------------------------------------------------------------------------

/// API keys are small opaque tokens. Bound native bridge input before it can
/// become process-resident secret material or a memory-amplification vector.
pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

/// Canonicalize and validate credentials entering the Android bridge. The
/// bridge is authenticated, but validating at its boundary keeps malformed or
/// future-provider input from bypassing the normal credential command checks.
pub fn credential_input(provider: &str, key: &str) -> Result<(String, String), String> {
    let canonical_provider = match provider.trim().to_ascii_lowercase().as_str() {
        crate::data::store::GROQ => crate::data::store::GROQ,
        crate::data::store::OPENAI => crate::data::store::OPENAI,
        crate::data::store::GOOGLE => crate::data::store::GOOGLE,
        crate::data::store::ASSEMBLYAI => crate::data::store::ASSEMBLYAI,
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

/// Providers with a just-saved value must not be overwritten by an
/// asynchronous AccessibilityService hydration pass that is still carrying
/// the previous Keystore value. The staged rotation writes the durable value;
/// this guard keeps the active Rust cache on that new value until the next
/// process/service lifecycle rehydrates from Android storage.
fn protected_credential_providers() -> &'static Mutex<HashSet<String>> {
    static PROTECTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PROTECTED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Normalize a provider id for cache keys. Unknown providers pass through so
/// future providers don't need a Rust change to work.
fn cache_key(provider: &str) -> String {
    provider.trim().to_lowercase()
}

/// Called by Kotlin (via `android_provide_credential`) after unlocking the
/// Android Keystore, and by `credentials::set` on Android after the frontend
/// saves a key. Memory-only: never persisted by Rust.
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

/// Replace a credential from the trusted frontend save path, even when an
/// earlier rotation already protected this provider from stale hydration.
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

/// Read a cached key. Empty string when absent — same contract as
/// `credentials::get` on desktop.
pub fn cached_credential(provider: &str) -> String {
    credential_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&cache_key(provider))
        .cloned()
        .unwrap_or_default()
}

/// Whether a key is cached for `provider`.
pub fn has_cached_credential(provider: &str) -> bool {
    !cached_credential(provider).is_empty()
}

/// Drop all cached keys (lock event, permission revocation, logout).
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

/// Serializes tests that share process-global bridge state (credential
/// cache, insertion outbox, keystore staging, pill mirrors). Rust runs tests
/// on threads in one process, so without this two tests publishing to the
/// same cell interleave and flake. Held across awaits: the test harness runs
/// each test future to completion on one thread, so a std guard is safe.
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
        // In-flight dictation survives keyboard churn; the pill re-anchors
        // when the keyboard returns instead of dropping the recording.
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
        // Gboard flicker / fast app switching: show → hide → show → hide must
        // end hidden with no dictation active.
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
        // Battery + notifications are recommended, never blocking.
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
        // Unknown packages degrade to a readable segment, never raw PII.
        let (label, generic) = context_label_for_package("com.example.my_notes");
        assert_eq!(label, "My notes".to_string());
        assert!(generic);
    }

    #[test]
    fn width_classes_follow_window_not_device() {
        assert_eq!(width_class_for_dp(360.0), WidthClass::Compact);
        assert_eq!(width_class_for_dp(599.0), WidthClass::Compact);
        // Tall/narrow outer foldable display stays compact.
        assert_eq!(width_class_for_dp(500.0), WidthClass::Compact);
        // Unfolded Fold / Pixel Fold inner display and tablets expand.
        assert_eq!(width_class_for_dp(601.0), WidthClass::Medium);
        assert_eq!(width_class_for_dp(839.0), WidthClass::Medium);
        assert_eq!(width_class_for_dp(841.0), WidthClass::Expanded);
        // Landscape phone in split-screen can be medium, not compact.
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
    fn local_ai_is_explicitly_unsupported() {
        assert!(!local_ai_supported_on_android());
        assert!(LOCAL_AI_ANDROID_UNSUPPORTED_REASON.contains("Android"));
    }
}
