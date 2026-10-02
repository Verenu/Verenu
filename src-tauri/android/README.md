# Verenu Android native sources

This directory holds Verenu's Android-specific code. Everything portable
(pipeline, providers, SQLite, sync, settings) stays in shared Rust and is
documented in `docs/ANDROID.md` — this directory is ONLY what the OS requires:

| Path | What |
| --- | --- |
| `kotlin/com/verenu/app/VerenuAccessibilityService.kt` | IME/focus detection, overlay lifecycle, insertion, Keystore sync |
| `kotlin/com/verenu/app/VerenuOverlayView.kt` | Native dictation pill (desktop pill visual parity, touch-sized) |
| `kotlin/com/verenu/app/VerenuDictationService.kt` | Microphone-type foreground-service holder (Rust/cpal captures) |
| `kotlin/com/verenu/app/VerenuBridge.kt` | Authed loopback client for `src-tauri/src/android/bridge.rs` |
| `kotlin/com/verenu/app/VerenuKeystore.kt` | `EncryptedSharedPreferences` (Android Keystore) credential store |
| `res/xml/accessibility_service_config.xml` | Accessibility-service declaration |
| `res/values/verenu_strings.xml` | Service/notification strings |
| `AndroidManifest.snippet.xml` | Permissions + services merged into the generated manifest |

## Deliberate constraints

- **Zero Tauri imports in Kotlin.** The service talks to Rust over the
  token-authed loopback bridge (`VerenuBridge.kt` ↔ `bridge.rs`), so this
  code compiles against any AGP/CLI combination and keeps working when the
  main activity is dead. Only Android SDK + `androidx.security:security-crypto`
  + `androidx.core` APIs are used.
- **No audio capture in Kotlin.** `cpal` (AAudio backend) captures in Rust;
  the foreground service only holds mic priority and process liveness.
- **`src-tauri/gen/` stays gitignored.** Nothing here is edited into
  generated output by hand — `scripts/android-sync.mjs` installs it.

## Installing into a generated project

```powershell
# one-time per machine: Android SDK 34, JDK 17, Rust android targets
node scripts/android-sync.mjs
# then:
npx tauri android build   # or open gen/android in Android Studio
```

The script runs `tauri android init` when needed, copies these sources and
resources, merges the manifest snippet, and pins `minSdk 26` +
`security-crypto` in the app module. It is idempotent and fails loudly if a
generated anchor it patches is missing (pin the Tauri CLI in package.json).

## First-SDK-build verification checklist

The environment that authored this code has no Android SDK, so these
call-sites must be eyeballed against compiler errors on the first real build:

1. `softKeyboardController` + `addOnShowCallback`/`addOnHideCallback` (API 33+,
   `VerenuAccessibilityService.armImeCallbacks`) — callback interface names.
2. `EncryptedSharedPreferences.create` + `MasterKey.Builder` signatures
   against the pinned `security-crypto` version.
3. `startForeground(id, notification, FOREGROUND_SERVICE_TYPE_MICROPHONE)` and
   the `FOREGROUND_SERVICE_MICROPHONE` manifest permission (targetSdk 36).
4. `TYPE_ACCESSIBILITY_OVERLAY` layering over Gboard/Samsung Keyboard on a
   real device (emulator IME behavior differs).
5. `filesDir/android_bridge.json` path agreement with Tauri's app-data dir
   (the client already tries two candidates; add more if needed, never widen
   permissions).
