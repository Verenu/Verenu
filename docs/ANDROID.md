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
| Local AI | Explicitly unsupported (see below); UI hides via `local_models_supported_on_this_platform == false` |

## The overlay

- Appears **only** while an IME keyboard is visible over an editable field;
  disappears when the keyboard closes. No permanent bubble.
- Never steals focus (`FLAG_NOT_FOCUSABLE` + `FLAG_NOT_TOUCH_MODAL`) and never
  replaces the keyboard — it is not an IME.
- States mirror the desktop pill: idle → recording → transcribing → cleaning →
  inserting, plus error (retry) and cancelled. Live waveform comes from the
  bridge `audioLevel` polls.
- Position is top-anchored below the status bar on purpose: keyboard height is
  not queryable from a service on every supported API, so a bottom-anchored
  pill would occlude the field or the IME somewhere. Top placement is
  deterministic on API 26–34 and never fights either.

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

## Local AI status

Neither `transcribe-rs`/ONNX Runtime nor `llama-server` ships Android ARM64
builds, and their payloads (tens of MB to GBs) are unrealistic on phones:

- `transcribe-rs` is a desktop-only dependency (`cfg(not(target_os =
  "android"))`); `local_stt/engine.rs` and `media/vad.rs` expose the same
  surface as explicit errors, and callers already fall back (cloud
  transcription, RMS speech gate).
- `local_models_supported_on_this_platform` returns `false` on Android, so
  Settings and Setup hide local options; `android_get_platform_info` carries
  the user-facing reason.
- Structure is ready for a future mobile runtime: re-add the dependency,
  implement the two stubs, flip the gate.

Also on Android: `reqwest` must move from `native-tls` to `rustls-tls`
(OpenSSL does not cross-compile under the NDK). The exact per-target edit is
marked `ANDROID BUILD NOTE` in `src-tauri/Cargo.toml` — apply it as part of
the first SDK build (it needs one networked `cargo build` to resolve the
`rustls-tls` feature set).

## Adaptive UI

Width classes follow the *window*, not the device: phones, tall/narrow
foldables, landscape, outer displays, unfolded Fold/Pixel Fold, tablets,
split-screen, and freeform windows all reclassify live (no restart, no state
loss). Compact → bottom nav + single column; expanded → rail + comfortable
multi-column (`shouldUseMultiPane` is available for list-detail views).
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

`scripts/android-sync.mjs` is idempotent — re-run it after CLI upgrades or a
fresh `tauri android init`. `src-tauri/gen/` stays gitignored; these sources
are the truth.

Minimum SDK is 26 (Android 8.0); target is 36. ARM64 (`arm64-v8a`) first;
x86_64 for emulators.

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

- Context resolution falls back to Everywhere on Android (the pipeline
  resolves before Kotlin reports the package). The ack already carries the
  package; feeding it into resolution is the designed follow-up
  (`android_context_for_package` exists for it).
- Contextual caps/spacing probes use desktop focus reads; on Android the
  cleanup model + dictionary carry formatting. Kotlin can supply surrounding
  text later for full parity.
- Auto-learn monitors are inert without desktop focus APIs.
- `copy_paste_failure_to_clipboard` has no Android clipboard backend in Rust;
  the Kotlin fallback owns clipboard duty.
