//! Android bridge commands.
//!
//! These are the Rust side of the Android integration. On desktop they return
//! honest `unsupported` errors (or pure-logic answers) so shared frontend code
//! can call them unconditionally; on Android the Kotlin layer in
//! `gen/android` implements the native half and calls back into these same
//! commands:
//!
//! * `android_provide_credential` — Kotlin pushes a Keystore-unlocked API key
//!   into Rust's in-memory cache at startup/rotation. Rust never persists
//!   secrets on Android; durable storage is Kotlin `VerenuKeystore`
//!   (`EncryptedSharedPreferences` + AndroidKeyStore).
//! * `android_on_keyboard_visibility` / `android_on_focus_changed` — Kotlin
//!   reports IME + focus state; Rust answers with the overlay decision and
//!   insertion strategy so both sides agree (logic lives in `crate::android`).
//! * `android_insert_text_result` — Kotlin reports direct-vs-fallback outcome
//!   for history/diagnostics.
//!
//! The overlay itself, `AudioRecord` capture fallback, permission intents, and
//! Keystore I/O are Kotlin-owned. See `docs/ANDROID.md`.

use crate::android::{
    self, AndroidPermission, AndroidPermissionSnapshot, InsertionStrategy, OverlayState,
};
use tauri::{AppHandle, Emitter};

/// Static Android platform facts for the frontend (SDK floor/target, ABI,
/// local-AI support). Same shape on every OS so Settings/Setup can render
/// without branching on `isAndroid` first.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidPlatformInfo {
    pub is_android_runtime: bool,
    pub min_sdk: u32,
    pub target_sdk: u32,
    pub supported_abi: String,
    pub local_ai_supported: bool,
    pub local_ai_unsupported_reason: String,
}

#[tauri::command]
pub fn android_get_platform_info() -> AndroidPlatformInfo {
    AndroidPlatformInfo {
        is_android_runtime: cfg!(target_os = "android"),
        min_sdk: android::ANDROID_MIN_SDK,
        target_sdk: android::ANDROID_TARGET_SDK,
        supported_abi: android::ANDROID_SUPPORTED_ABI.to_string(),
        local_ai_supported: android::local_ai_supported_on_android(),
        local_ai_unsupported_reason: android::LOCAL_AI_ANDROID_UNSUPPORTED_REASON.to_string(),
    }
}

/// Overlay visibility decision for a keyboard/focus report from Kotlin.
///
/// Returns the overlay state Kotlin should render plus the two booleans that
/// drive its `WindowManager` handling: `visible` (add/remove the overlay
/// view — never a permanent bubble) and `dictationActive` (keep rendering
/// across keyboard churn vs settle back to idle/hidden).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidOverlayDecision {
    pub state: OverlayState,
    pub visible: bool,
    pub dictation_active: bool,
}

#[tauri::command]
pub fn android_on_keyboard_visibility(
    keyboard_visible: bool,
    has_editable_focus: bool,
    current: Option<OverlayState>,
) -> AndroidOverlayDecision {
    let state = current.unwrap_or(OverlayState::Hidden);
    let state = if android::should_show_overlay(keyboard_visible, has_editable_focus) {
        state.on_keyboard_shown()
    } else {
        state.on_keyboard_hidden()
    };
    AndroidOverlayDecision {
        visible: state.is_visible(),
        dictation_active: state.is_dictation_active(),
        state,
    }
}

/// Insertion strategy for the currently focused field, from Kotlin's focus
/// report. Direct accessibility text editing where the app allows it,
/// clipboard fallback where it doesn't.
#[tauri::command]
pub fn android_decide_insertion(
    has_editable_focus: bool,
    supports_set_text: bool,
) -> InsertionStrategy {
    android::decide_insertion_strategy(has_editable_focus, supports_set_text)
}

/// Context label for the foreground package (package name only — never field
/// contents). Used for per-app Context resolution on Android.
#[tauri::command]
pub fn android_context_for_package(package: String) -> serde_json::Value {
    let (label, is_generic) = android::context_label_for_package(&package);
    serde_json::json!({ "package": package, "label": label, "isGeneric": is_generic })
}

/// Kotlin pushes a Keystore-unlocked credential into Rust's memory-only cache.
/// Never writes to disk; durable storage stays in Kotlin `VerenuKeystore`.
#[tauri::command]
pub fn android_provide_credential(provider: String, key: String) -> Result<(), String> {
    let (provider, key) = android::credential_input(&provider, &key)?;
    android::provide_credential(&provider, &key);
    Ok(())
}

/// Drop all in-memory credentials (lock event, permission revocation).
#[tauri::command]
pub fn android_clear_credentials() -> Result<(), String> {
    android::clear_credentials();
    crate::android::bridge::clear_keystore_rotation();
    Ok(())
}

/// Whether Rust currently holds a key for `provider` in memory.
#[tauri::command]
pub fn android_has_credential(provider: String) -> bool {
    android::has_cached_credential(&provider)
}

/// Permission rationale strings for onboarding, in display order.
#[tauri::command]
pub fn android_permission_rationale() -> Vec<serde_json::Value> {
    [
        AndroidPermission::Microphone,
        AndroidPermission::AccessibilityService,
        AndroidPermission::BatteryExemption,
        AndroidPermission::Notifications,
    ]
    .into_iter()
    .map(|permission| {
        let name = match permission {
            AndroidPermission::Microphone => "microphone",
            AndroidPermission::AccessibilityService => "accessibility_service",
            AndroidPermission::BatteryExemption => "battery_exemption",
            AndroidPermission::Notifications => "notifications",
        };
        serde_json::json!({
            "id": name,
            "required": android::permission_is_required(permission),
            "rationale": android::permission_rationale(permission),
        })
    })
    .collect()
}

/// Evaluate a permission snapshot from Kotlin. Pure logic — same function the
/// frontend uses to decide whether onboarding can complete.
#[tauri::command]
pub fn android_evaluate_permissions(snapshot: AndroidPermissionSnapshot) -> serde_json::Value {
    serde_json::json!({
        "functional": snapshot.is_functional(),
        "missingRequired": snapshot.missing_required(),
    })
}

/// Width-class for an available window width in dp. Lets the backend include
/// layout hints in events; the Svelte shell is the source of truth visually.
#[tauri::command]
pub fn android_width_class(width_dp: f32) -> android::WidthClass {
    android::width_class_for_dp(width_dp)
}

/// Kotlin reports the outcome of a text-insertion attempt so history and
/// diagnostics record whether direct accessibility editing or the clipboard
/// fallback delivered the text.
#[tauri::command]
pub async fn android_insert_text_result(
    app: AppHandle,
    package: String,
    strategy: InsertionStrategy,
    success: bool,
    error: Option<String>,
) -> Result<(), String> {
    log::info!(
        "android insertion package={package} strategy={strategy:?} success={success} error={}",
        error.as_deref().unwrap_or("-")
    );
    let _ = app.emit(
        "verenu:android-insertion-result",
        serde_json::json!({
            "package": package,
            "strategy": strategy,
            "success": success,
            "error": error,
        }),
    );
    Ok(())
}

/// Save (or delete, when `key` is empty) an API key on Android.
///
/// Writes Rust's memory-only cache immediately so the pipeline can use the
/// key, and stages a rotation that the Kotlin `VerenuKeystore` fetches once
/// via `GET /v1/keystore/pending` and persists to `EncryptedSharedPreferences`
/// (AndroidKeyStore). The frontend calls this instead of `save_api_key` on
/// Android — `save_api_key` stays memory-only there by design.
#[tauri::command]
pub fn android_keystore_save(provider: String, key: String) -> Result<(), String> {
    let (provider, normalized) = crate::android::credential_input(&provider, &key)?;
    crate::android::replace_credential(&provider, &normalized);
    crate::android::bridge::stage_keystore_rotation(&provider, &normalized);
    log::info!(
        "android keystore: staged rotation provider={provider} key_len={}",
        normalized.len()
    );
    Ok(())
}

/// Ask Kotlin to fire the system prompt for `permission` (runtime dialog,
/// accessibility settings, battery-optimization settings, notification
/// settings), then publish the fresh native snapshot to the webview.
#[tauri::command]
pub async fn android_request_permission(
    app: AppHandle,
    permission: AndroidPermission,
) -> Result<(), String> {
    log::info!("android permission request: {permission:?}");
    #[cfg(target_os = "android")]
    {
        let snapshot = crate::android::permissions_plugin::run::<_, serde_json::Value>(
            &app,
            "request",
            serde_json::json!({ "permission": permission }),
        )
        .await?;
        app.emit("verenu:android-permission-snapshot", snapshot)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app.emit(
            "verenu:android-request-permission",
            serde_json::json!({ "permission": permission }),
        );
        Ok(())
    }
}

/// Read the native Android permission state when the app regains focus. This
/// catches changes made in Accessibility, battery, notification, and app-info
/// settings without requiring a restart.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[tauri::command]
pub async fn android_read_permissions(app: AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        return crate::android::permissions_plugin::snapshot(&app).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(serde_json::json!({
            "microphone": "not_asked",
            "accessibility_service": "not_asked",
            "battery_exemption": "not_asked",
            "notifications": "not_asked"
        }))
    }
}

/// Placeholder for permission-revocation handling. Kotlin calls this when
/// Android revokes a grant at runtime (user toggles it off, OEM battery
/// manager intervenes); Rust drops in-memory secrets and tells the UI to
/// re-run onboarding recovery.
#[tauri::command]
pub async fn android_on_permission_revoked(
    app: AppHandle,
    permission: AndroidPermission,
) -> Result<(), String> {
    if matches!(
        permission,
        AndroidPermission::Microphone | AndroidPermission::AccessibilityService
    ) {
        android::clear_credentials();
    }
    let _ = app.emit(
        "verenu:android-permission-revoked",
        serde_json::json!({ "permission": permission }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_visibility_command_drives_overlay() {
        let shown = android_on_keyboard_visibility(true, true, None);
        assert_eq!(shown.state, OverlayState::VisibleIdle);
        assert!(shown.visible);
        assert!(!shown.dictation_active);

        let hidden = android_on_keyboard_visibility(false, true, None);
        assert_eq!(hidden.state, OverlayState::Hidden);
        assert!(!hidden.visible);

        let no_focus = android_on_keyboard_visibility(true, false, None);
        assert_eq!(no_focus.state, OverlayState::Hidden);

        // In-flight dictation survives keyboard churn.
        let flight = android_on_keyboard_visibility(false, false, Some(OverlayState::Recording));
        assert_eq!(flight.state, OverlayState::Recording);
        assert!(flight.dictation_active);
    }

    #[test]
    fn insertion_command_prefers_direct() {
        assert_eq!(
            android_decide_insertion(true, true),
            InsertionStrategy::DirectAccessibility
        );
        assert_eq!(
            android_decide_insertion(true, false),
            InsertionStrategy::ClipboardFallback
        );
    }

    #[test]
    fn credential_commands_round_trip_in_memory_only() {
        let _serial = crate::android::test_serial();
        android::clear_credentials();
        assert!(!android_has_credential("groq".to_string()));
        android_provide_credential("groq".to_string(), "gsk-test".to_string()).unwrap();
        assert!(android_has_credential("groq".to_string()));
        assert!(android_provide_credential("".to_string(), "x".to_string()).is_err());
        android_clear_credentials().unwrap();
        assert!(!android_has_credential("groq".to_string()));
    }

    #[test]
    fn keystore_save_stages_rotation() {
        let _serial = crate::android::test_serial();
        crate::android::clear_credentials();
        crate::android::bridge::clear_keystore_rotation_for_tests();
        android_keystore_save("google".to_string(), "AIza-test".to_string()).unwrap();
        assert!(android_has_credential("google".to_string()));
        assert!(crate::android::bridge::has_keystore_rotation());
        // Delete rotates an empty key through the same single-delivery path.
        android_keystore_save("google".to_string(), "".to_string()).unwrap();
        assert!(!android_has_credential("google".to_string()));
        assert!(crate::android::bridge::has_keystore_rotation());
        crate::android::bridge::clear_keystore_rotation_for_tests();
    }

    #[test]
    fn rationale_lists_required_first() {
        let rationale = android_permission_rationale();
        assert_eq!(rationale.len(), 4);
        assert_eq!(rationale[0]["id"], "microphone");
        assert_eq!(rationale[0]["required"], true);
        assert_eq!(rationale[2]["id"], "battery_exemption");
        assert_eq!(rationale[2]["required"], false);
    }
}
