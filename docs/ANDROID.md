# Verenu on Android

Verenu Android is a first-class platform in the same Tauri 2 + Svelte 5 +
TypeScript + Rust codebase — not a separate clone. This document maps what is
shared, what is Android-specific, and how to build it.

## Architecture

```
┌─ Svelte frontend (shared, adaptive shell) ──────────────┐
│ App.svelte + width-class shell │ MobileNav │ Setup wizard │
│ Settings · Contexts · History · Insights (parity)        │
└────────────── Tauri commands / events ───────────────────┘
┌─ Rust backend (shared pipeline) ─────────────────────────┐
│ pipeline · api/providers · cleanup · dual transcription  │
│ snippets · vocabulary · contexts · SQLite · sync         │
│ android/ : overlay logic · bridge · Keystore cache       │
└─── loopback bridge (127.0.0.1, token-authed) ────────────┘
┌─ Kotlin (OS-required only) ──────────────────────────────┐
│ AccessibilityService · overlay pill · FGS mic holder     │
│ EncryptedSharedPreferences (Keystore) · bridge client   │
└──────────────────────────────────────────────────────────┘
```

### Reused unchanged

Dictation pipeline orchestration, cloud transcription + cleanup providers,
prompt assembly, dual-model transcription + reconciliation, fallbacks, cleanup
cache, snippets, personal dictionary, Contexts, history/SQLite, insights,
import/export, and LAN sync (`mdns-sd`, `rustls`) are the same code on all
platforms. Platform differences sit behind `#[cfg(target_os = "android")]`
branches and the `src-tauri/src/android/` module — Windows/macOS paths are
untouched.

### Android-specific

| Area | Implementation |
| --- | --- |
| Keyboard detection + overlay | `VerenuAccessibilityService` (`src-tauri/android/kotlin/…`); `TYPE_ACCESSIBILITY_OVERLAY`, top-anchored, `FLAG_NOT_FOCUSABLE` |
| Recording trigger/stop/retry | Loopback bridge `POST /v1/recording/*` → existing `commands::recording` entry points |
| Text insertion | `ACTION_SET_TEXT` + cursor restoration; clipboard + `ACTION_PASTE` fallback; ack drives pill/diagnostics |
| Credentials | Kotlin `VerenuKeystore` (durable) + Rust memory-only cache; `android_keystore_save` stages rotations |
| Audio capture | Rust `cpal` (AAudio); Kotlin foreground service (microphone type) holds priority/liveness |
| Permissions onboarding | `AndroidPermissionsStep` + `android_*` commands; rationale matches Rust strings |
| Adaptive shell | Width classes (600/840dp) in `src/lib/android/viewport.ts` + `App.svelte`; bottom nav on compact; safe-area edge-to-edge |
| Local AI | On-device speech and cleanup on Android 9+, with bundled inference runtimes and downloaded model weights |

## The overlay

- Appears while an IME keyboard is visible over an editable field. During an
  active dictation it stays available as a docked recorder when the keyboard
  closes, then returns to the keyboard when it reopens.
- Never steals focus (`FLAG_NOT_FOCUSABLE` + `FLAG_NOT_TOUCH_MODAL`) and never
  replaces the keyboard — it is not an IME.
- States mirror the desktop pill: idle → recording → transcribing → cleaning →
  inserting, plus error (retry) and cancelled. The recording waveform uses the
  recorder's peak envelope and redraws each display frame.
- By default the pill covers the keyboard's own mic key (Settings -> General turns
  this off) so it never sits over the text being typed.
- The cancelled notice shows an X (dismiss, back to the idle pill) and a back
  arrow (restart). It does not keep the pill alive after the keyboard closes.
  Error notices carry a title, the reason, and a Retry button when retrying can
  help.
- Settings -> General -> Pill position selects above-keyboard center, left or
  right, top, middle, or under the camera hole. Keyboard placement follows the
  IME bounds. The pill waits for those bounds before appearing and animates
  between positions.

## Permissions

| Permission | Why | Without it |
| --- | --- | --- |
| Microphone (`RECORD_AUDIO`) | Capture dictation audio | Cannot dictate (onboarding blocks) |
| Accessibility service | See keyboard state, show the pill, insert text, per-app Context labels | Cannot dictate (onboarding blocks) |
| Battery exemption | OEM killers cut background audio | Recordings may stop mid-sentence (warned, not blocked) |
| Notifications (`POST_NOTIFICATIONS`, 33+) | Recording indicator + shade Stop | Indicator hidden (warned, not blocked) |

Revocation at runtime clears in-memory keys and re-opens onboarding recovery
(`android_on_permission_revoked` → `verenu:android-permission-revoked`).

## Credentials

API keys rest **only** in `EncryptedSharedPreferences` (AES256-GCM,
AndroidKeyStore). Rust holds them in memory, populated at unlock
(`POST /v1/credential`) and on save (`android_keystore_save` → staged
rotation → single-delivery `GET /v1/keystore/pending`). Rust never writes a
secret to disk on Android and never logs one (lengths only). Fail-closed: a
broken device Keystore surfaces recovery instead of downgrading to plaintext.

## Loopback bridge

`src-tauri/src/android/bridge.rs` is the protocol source of truth: per-boot
256-bit token, `0600` connection file, `127.0.0.1`-only ephemeral port, one
request per connection. Endpoints are listed in the module docs; behavior is
pinned by `bridge.rs` unit tests (real TCP, run on desktop CI). Kotlin never
imports Tauri — the bridge works with the main activity dead.

## Local AI

Android uses the shared speech engines, Silero voice detection, and cleanup
pipeline. `scripts/android-sync.mjs` bundles checksum-pinned ONNX Runtime
1.24.3 and an NDK-built llama.cpp server for ARM64 and x86_64. Runtime code
ships inside the APK; only model weights are downloaded into private app storage.
Cleanup executes the packaged runtime from Android's native library directory.
The bundled cleanup runtime cannot be removed separately from the app.

Local AI requires Android 9 (API 28) or newer. Android 8 can still use cloud
providers. The capability check also verifies that both runtimes are present;
Settings and Setup hide local options when the installed build lacks them.

Phone presets start with Moonshine Tiny (English speech, about 31 MB) and
Qwen 2.5 0.5B cleanup (about 430 MB). Devices with little memory offer speech
without AI cleanup. Larger cleanup models remain available in Advanced Models.
Downloads require a connection; inference works offline afterward. The existing
memory policy controls unloading both engines.

For emulator pipeline checks, a debug APK can opt into `android-local-testing`.
Its `android_test_local_audio` command accepts bounded 16 kHz mono PCM WAV
fixtures and runs the production pipeline without inserting text. It requires
local speech and cleanup selections and empty cloud fallback lists. This feature
is rejected by release builds and is absent from ordinary debug APKs.

## Adaptive UI

Width classes follow the *window*, not the device: phones, tall/narrow
foldables, landscape, outer displays, unfolded Fold/Pixel Fold, tablets,
split-screen, and freeform windows all reclassify live (no restart, no state
loss). Compact → bottom nav + single column; expanded → rail + comfortable
multi-column (`shouldUseMultiPane` is available for list-detail views).
On compact and medium windows Settings is a section list that drills into each
section (list and section side by side on medium); expanded windows keep the
sidebar rail. Mobile-only styling lives in `src/mobile.css`, scoped to
`.app[data-android='true']` so desktop is untouched.
Safe-area insets, cutouts, gesture nav, and hinge half-open postures are
handled; `android:configChanges` (set by the Tauri template) keeps rotation
and folding from recreating the activity.

## Building

Prerequisites: JDK 17, Android SDK (platform 36, build-tools),
`ANDROID_HOME`/`ANDROID_SDK_ROOT`, Rust targets
`aarch64-linux-android` (+ `x86_64-linux-android` for the emulator), and the
NDK matching AGP.

```powershell
npm install
node scripts/android-sync.mjs   # tauri android init + sources + manifest + gradle
npx tauri android dev           # device or emulator
npx tauri android build         # signed/unsigned APK + AAB in gen/android
```

Native runtime preparation also requires CMake, Ninja, `curl`, and the JDK's
`jar` command. Pinned sources and build outputs are cached under this worktree's
`src-tauri/target/android-local-runtimes`; no host CPU tuning is used.

The repository-root `.cargo/config.toml` sets 16 KB ELF and RELRO linker
alignment for both Android Rust targets. Keep these target flags at the
repository root: Tauri launches Cargo from there while building the manifest in
`src-tauri/`. Oboe and llama.cpp use the NDK's static C++ runtime, so the APK
does not include `libc++_shared.so`. For release checks, inspect every packaged
`.so` LOAD alignment and GNU_RELRO end, then run `zipalign -c -P 16 -v 4 <apk>`;
the Android developer guide documents the 16 KB checks and linker requirements.

`scripts/android-sync.mjs` is idempotent — re-run it after CLI upgrades or a
fresh `tauri android init`. `src-tauri/gen/` stays gitignored; these sources
are the truth.

Minimum SDK is 26 (Android 8.0); target is 36. ARM64 (`arm64-v8a`) first;
x86_64 for emulators.

## Testing on an emulator

An x86_64 API 34+ AVD is enough for the full dictation path:

```bash
node scripts/android-sync.mjs            # after `npm run build`
npx tauri android build --debug --target x86_64 --apk
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb shell pm grant com.verenu.app android.permission.RECORD_AUDIO
adb shell settings put secure enabled_accessibility_services \
  com.verenu.app/com.verenu.app.VerenuAccessibilityService
```

Enter a provider key under Settings → API Keys (it goes straight into the
Android Keystore). To feed speech to the emulated microphone, start the
emulator with `-grpc 8554` and stream PCM to the emulator controller's
`injectAudio` RPC (16 kHz mono S16 works); host-audio passthrough is
unreliable. Open any text field (for example
`adb shell am start -a android.intent.action.INSERT -t vnd.android.cursor.dir/contact`),
tap the pill, inject audio, then tap Stop. `adb shell setprop log.tag.VerenuA11y DEBUG`
turns on the accessibility service's event log (event types only, never text).

## Testing

- Rust: `cargo test android` — overlay state machine, insertion strategy,
  permission gates, Context labels, width classes, credential cache, and the
  full bridge protocol over real TCP (auth, focus→overlay, handoff→ack,
  keystore rotation, malformed input).
- Frontend: `npx vitest run src/lib/android` — viewport classes, tracking,
  permission model, recovery copy.
- Desktop suites must stay green: `npm run check`, `npm run test:unit`,
  `cargo test`, `npm test` (fast profile).
- Device/manual (first build): Gboard + Samsung Keyboard show/hide cycles,
  rapid app switching, rotation, fold/unfold, split-screen, dark/light +
  custom accents, interrupted recordings, provider failures, permission
  revocation, process recreation. `docs/ANDROID.md` (this file) + the
  checklist in `src-tauri/android/README.md` track what automation cannot.

## Known limitations (v1)

- Context resolution uses the package Kotlin reports when recording starts, and in
  a browser the page's host read from its address bar (with the last site seen as a
  fallback while the omnibox is being edited). Apps without a readable address bar,
  or pages with the toolbar scrolled away for longer than ten minutes, resolve to
  Everywhere.
- Contextual caps/spacing probes use desktop focus reads; on Android the
  cleanup model + dictionary carry formatting. Kotlin can supply surrounding
  text later for full parity.
- Auto-learn monitors are inert without desktop focus APIs.
- `copy_paste_failure_to_clipboard` has no Android clipboard backend in Rust;
  the Kotlin fallback owns clipboard duty.
