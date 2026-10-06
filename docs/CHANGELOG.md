# Changelog

Notable project changes are recorded here. GitHub Release pages remain the source for full release descriptions, installer assets, VirusTotal links, and platform-specific download notes.

## Unreleased

- Strengthened CI and agent verification with strict skipped-test handling, complete owned-session test discovery, native capability/platform evidence, Android Kotlin/JVM checks, native WebView PR checks, workflow linting, and exact-source nightly release verification. Added Context lifecycle, populated-cache clearing, accessibility, and visual regression coverage.
- Queued runtime icon updates on the UI thread, including tray-handle lookup and disposal, to prevent unsafe reference-count access during concurrent appearance saves. Added an owned-session concurrency regression.

- Fixed installed Linux windows tiling or stretching when a release executable is renamed. Verenu uses a stable GTK app ID and installs its main-window and dictation-pill policies automatically on Omarchy Quattro, including first launch. Volume, playback, and brightness keys no longer cancel held dictation.

- Fixed Linux global shortcuts failing when Verenu is launched outside an editor or terminal with a known app identity. Verenu now registers its own portal connection, reconnects after listener failure, and shows persistent shortcut errors with the cause and recovery guidance in Home and Settings.

- Fixed the Android Microphone gain and Sound effects volume sliders rendering as thick slabs. They now draw a slim rounded track with a centered, touch-sized thumb.

- Linux Context matching now keeps the captured app and window title throughout dictation. Executable aliases also match sub-app rules, including T3 Code's differing Wayland class. Website detection activates Chromium's accessibility tree, distinguishes windows sharing a process, recognizes more browser app IDs and channels, and ignores ambiguous or changed targets. Localhost websites are detected too. Address-bar reads have a hard deadline and never stack workers.

- Fixed Linux system notifications failing with a nested-runtime panic. Update, model-ready, and service notices now send with Verenu branding and report delivery errors so failed update and service notices can retry.

- Added desktop-only Paste in Chunks in Context Advanced settings. It inserts small pieces with pauses for CLI prompts, preserves the clipboard, and stops remaining chunks when focus or clipboard ownership changes.

- Fixed Linux shortcut capture rejecting Ctrl + Super when the WebView reports Super as an OS key.
- Added copyable Tailscale connection details, remote code-based pairing, and a saved connection editor in Sync. Setup explains relayed multi-device sync and Android foreground limits. Android enables nearby Wi-Fi multicast discovery while Verenu is visible.
- Redesigned the Sync page. Paired devices keep their status, last sync, and route in one row, and the Connection editor opens inside that row. Tailscale setup is a three-step flow with a copy button that confirms, and a newly paired device offers a reminder to give the other side a way back. Pairing shows connecting, code, and checking phases, and its dialogs no longer sit under the phone sidebar. All motion follows the shared reduced-motion settings.
- Multi-device hubs promptly relay imported settings and counter-only updates. Busy peers end the connection attempt without probing alternate LAN addresses, allowing simultaneous edits to recover promptly.

- Paired devices automatically sync committed edits from either side within the next 750 ms check, with queued follow-up sessions and retry backoff. Existing pairs can save a persistent Tailscale route. Connection errors include the destination port and firewall guidance. Provider/model choices remain local; the open General settings page refreshes shared values after sync.

- Home history, lifetime totals, app filters, and Insights now refresh after a device sync completes, including sessions that only merge lifetime counters.

- Split cloud priorities from Local AI choices in grouped model lists. Each row opens model and fallback details, with one row expanded at a time and the standard Settings dropdown style. Cloud presets can prepare on-device recovery for speech and cleanup. Quality keeps dual transcription to catch hallucinated additions and reports when only one transcript succeeds. Cleanup failures preserve completed speech. Automatic priorities can use private session timings while Advanced selections keep their explicit order; local speed tests use bundled synthetic speech. Recommendations respect language support and confirmed catalog retirement.

- Unified remaining Settings dropdown controls with the compact Insights style and kept privacy retention and model strategy menus within narrow panels.

- Removed field-specific microphone buttons from term, vocabulary, and snippet dialogs. Normal hotkey dictation can now paste into Verenu's captured main window on Windows instead of falling back to manual paste.

- Reduced the Linux dictation pill window to fit its capsule and context chip, with room for animation. It grows for longer errors and shrinks afterward. Passive recording and processing states stay click-through, while visible controls accept clicks inside the capsule. Delayed state updates can no longer make hold-to-dictate interactive.
- Removed app context hints from settings, onboarding, sync, and cleanup requests. App and website matching still selects Contexts locally.
- Tightened shared cleanup prompts and transcription instructions. Cleanup preserves uncertainty and meaningful qualifiers, and Strong restructures without summarizing. Stable rules precede request-specific vocabulary to improve prompt-cache reuse.
- Hardened cleanup caching with exact, versioned request hashes and validation against current transcripts. Results now expire after two idle days or seven days total. Privacy settings show cached text size and local session reuse counts, and can disable persistence while clearing stored results. Old normalized cache keys are discarded on upgrade.

- Linux app icons now use the desktop's icon theme resolver, including inherited themes, category icons, and Flatpak exports. Missing or failed icon requests can retry when the picker reopens.

- Android keeps the dictation pill available offline when all selected active models run on-device. Settings > General adds "Hide pill when offline", enabled by default for network models and their fallbacks; turn it off for LAN-hosted models.

- Android: every phone now defaults to the Qwen 2.5 0.5B cleanup model with Moonshine Tiny speech. The 1.5B preset is gone (cleanup was too slow on real phones); 1.5B and larger stay available under Advanced Models, and 0.5B carries the Recommended badge. The model picker no longer freezes the app on a phone: it is now a full-screen page above the bottom bar, opening on the current provider's models, with no row animations, blur, or keyboard pop-up. The provider list is a row of chips, rows are touch-sized with Download, Delete, and add-as-fallback buttons always visible, and the 0.5B model is listed first on phones.
- Android: dictation targets now work. Contexts attached to apps match the app you start dictating in, and Contexts attached to websites match the page open in Chrome, Firefox, Brave, Edge, Samsung Internet, Opera, DuckDuckGo, Vivaldi, or Kiwi (only the host is read, never the URL or page text). The app picker lists the apps installed on the phone with their real icons and name-ranked search. Sub-apps are hidden on Android.
- Android: the pill states were redrawn to match the desktop pill. Recording, cancelled, and error are 34 dp capsules with borderless glyph buttons (a muted X on the left, retry or undo on the right) and a plain centred label; the error pill is the flat dark-red capsule with a ring, no icon, no title. Over the keyboard (the default) every state sits inside the keyboard's top row on the mic key and grows to the left, vertically centred on that row. Over the keyboard each state is as tall as the mic key, so the keyboard's own chips behind it stay covered. The busy states (Transcribing, Cleaning up, Pasting) drop the spinner for the desktop's dimmed label with a bright band sweeping across the letters.
- Android: the pill now appears about as soon as the keyboard does (about 270 ms to about 130 ms after the field gains focus in testing). It treats a sliding keyboard as already docked, remembers where each keyboard keeps its mic key, and re-checks immediately instead of a tenth of a second later. Settings gains slide and rise transitions between the list and each section, a growing indicator on the bottom bar, and press feedback. Fixed Save and Clear buttons from the Providers page showing over the Settings list.
- Android: context groups can be edited and deleted from the Contexts screen (an Edit button under the name; delete is inside the editor). The group editor is a full screen. The Add app and Add website pickers dock above the keyboard instead of jumping across the row. Opening Settings no longer pops up the keyboard. Verenu is portrait-only.
- Android: forms and sheets now sit above the keyboard instead of under it. The page resizes when the keyboard opens, the bottom bar steps aside, sheets keep their actions in view, and the focused field scrolls into view. Settings has its own search box. The cover-the-keyboard-mic-button pill placement is now the default, so the pill no longer floats over the text being typed.
- Android: the recording notification now uses a minimum-importance, silent channel with no lock-screen content. Android requires a notification while the microphone service runs, so it cannot be removed entirely, and some versions still show its small icon.
- Reworked the Android layout. Settings is now a list of sections that opens each one on its own screen with a back button (a list beside the section on unfolded windows), replacing the scrolling tab strip that cut off sections. The bottom bar has a tonal active indicator and keeps its tabs together on wide windows. Switches, buttons, inputs, and sliders are larger touch targets, type is larger, and wide windows use a centred reading column. All of it is scoped to Android; desktop is unchanged.
- Android pill: the cancelled notice now has an X that dismisses it and a back arrow that restarts the dictation, instead of any tap restarting it. A cancelled notice no longer lingers at the top of the screen when the keyboard closes. Error notices show what failed, the specific reason, and a labelled Retry button (or just dismiss when retrying cannot help). The X is drawn as one stroke, which fixes the brighter centre it had when translucent. The pill also appears sooner after the keyboard opens.
- Added custom providers in Settings > Providers. Connect an OpenAI, Anthropic, or xAI compatible base URL, name the transcription and cleanup models it offers, and optionally configure an API key header, non-secret headers, and cleanup request options. Models appear under the provider's name in Settings > Models. Keys stay in the native credential store; local endpoints can work without a key. Removing a provider removes its selected models and repairs the fallback chain. Adding a provider starts from a searchable preset picker with about 30 unofficial presets for cloud APIs, Anthropic-style endpoints, and local servers such as Ollama and LM Studio. Presets only prefill the form; Verenu does not test or support those vendors, and every field stays editable.
- Reworked Appearance with visual color-scheme cards, a theme gallery, and saved custom themes. The corner theme editor previews changes across the app, stays open during navigation, and restores the previous appearance when cancelled.
- Added instruction editors to the cleanup and Personal Tone tiles. Edits change only the instructions injected by each preset, with reset to built-in defaults and an audit against the composed system prompt before saving.
- Fixed Save and Clear labels overlapping on the API Keys page when the inactive action was disabled.
- Added automatic cloud model discovery with capability metadata, daily and manual refresh, an updateable AssemblyAI catalog, and cached model lists for offline use.
- Added Android local speech recognition and cleanup with APK-bundled ONNX
  Runtime and llama.cpp. Android 9+ can download models and dictate offline;
  phone presets favor Moonshine Tiny and small Qwen cleanup models, with
  transcription-only presets on low-memory devices.

- Split frontend IPC routing from browser mocks and their model/Insights fixtures,
  preserving existing imports and loading mocks only in preview mode. Corrected
  contributor setup and verification instructions, consolidated stale agent
  planning notes, and ignored local environment and test-report artifacts.

- Added source-bound agent verification, owned real-backend browser sessions,
  synthetic speech checks, native WebView tests, and independent agent test
  evaluations. Test reporting now distinguishes unavailable live checks from
  passes, fails executed optional checks, and retains retry failures. Test
  servers use owned ports instead of reusing or stopping another session.

- Fixed history rows and app filters displaying Linux window IDs instead of installed app names, including existing history entries and Wayland IDs that correspond to a short desktop window class.

- Hotkey capture now records the full held combination and saves on release, with no fixed two-key limit. Windows tracks arbitrary key chords; macOS and Linux support multiple modifiers with one trigger key. Detected conflicts and unsupported combinations report an error and keep the previous binding. Home and setup display the entire saved combination.
- Added native macOS correction reads for Auto-learn and tied learning and rejection to the original editable control on Windows, macOS, and Linux. Monitoring now anchors the text actually inserted, retries slow accessibility reads, accepts single-word dictation, and requires a stable edit before recording evidence. Learned brand capitalization and split names apply to later dictations, short technical corrections can accumulate real evidence, and pending observations last 30 days. The regression matrix now checks real detector confidence and promotion instead of supplying invented scores.
- Fixed local cleanup model downloads omitting the shared engine. Local AI presets wait for the engine as well as model weights, and the picker offers engine installation for existing downloads. The dictation pill now replays its context label after startup.

- Added automatic redacted log files per app session, with 30-day retention, a 256 MiB folder budget, background batched writes, and automatic pause/resume when disk space is low. Documented log locations for agent diagnosis.
- Added custom icons for Context groups: emoji, and one- or two-character badges with a softened background and chosen text color. Icon colors use a permanent row of preset swatches plus a native custom-color button. Emoji render without a background, including saved ones. The right-click color popup in the Context editor is gone.

- Reworked parts of onboarding. The analytics arrow is now a centred icon. The models step is titled "Speed or accuracy?" and no longer offers Local AI to people who picked a cloud provider. English leads the language list. The Try It result eases in with a short glow instead of snapping. The API key walkthrough ends on a fifth "paste your key" slide, replacing "I've got my key", and the existing Skip for now still applies. The final summary is compact, shows each choice (models, writing, language, audio), and its step numbers are centred.

- Added OpenRouter and xAI as bring-your-own providers in Settings > Providers and the model picker. OpenRouter transcribes through its JSON `audio/transcriptions` endpoint and cleans up through chat completions; xAI transcribes with Grok Voice Transcribe (`/v1/stt`) and cleans up through chat completions. Providers now declare a separate cleanup and transcription adapter, so one provider can mix wire formats. Cleanup skips always-reasoning models on both. Neither provider appears in the one-click presets or the setup wizard, and their model lists are not live-synced, so any model id can be entered in Advanced Models.
- Detect Android text-field length limits before insertion, preserving the existing field and copying the full dictation instead of partially inserting it. The pill now explains when the dictation is too long for the field (including Samsung's 100-character app search).

- Fixed Android app-search dictations falling back to manual paste after privacy-indicator or notification events changed the remembered app to System UI. The focused editable field now determines insertion eligibility, and system/keyboard windows no longer replace the app target.

- Stopped Android paste attempts from stacking accessibility timeouts when an editor stops responding. Insertion now reuses one focused field, bounds focus discovery and readback retries, and preserves the text on the clipboard when it cannot confirm the edit. It does not issue a second paste after an uncertain result.

- Reworked the Android dictation pill after on-device testing:
  - New **Pill position** setting (Settings → General): above the keyboard at center, left or right, top of the screen, middle of the screen, or under the camera hole. It changes live.
  - The pill no longer spawns mid-keyboard and jumps: it waits for the keyboard's bounds, fades in at its final spot, and eases between positions. It no longer blinks off and on when focus or system windows change (hiding is debounced).
  - While a dictation is running with the keyboard closed, the pill stays on screen as a small docked recorder (waveform, timer, stop) and returns to the keyboard when it reopens, instead of vanishing while still recording.
  - The recording row uses the desktop pill's visualizer: 12 mirrored bars driven by the recorder's 10 ms peak envelope and redrawn every display frame, replacing the ~4 Hz level bars. Added a recording dot and timer, a clear Stop button, spinner stages, and animated width/color transitions between states.
  - The pill is near-black in both themes (it vanished against a light keyboard) and follows Verenu's own appearance setting.
  - Fixed pasting that could loop on "Pasting…": the target is now the field focused at that moment rather than a remembered foreground package, retries every 350 ms, and after a few seconds with no field the text is copied and the outbox cleared with a clear message. Empty fields no longer get their hint text ("Search") inserted in front of the dictation.
  - Foldable inner display: the Home page no longer overflows next to the sidebar.
- Made the Android build work end to end, verified on an API 34 emulator with synthetic speech through the real Groq pipeline:
  - Restored the Android manifest snippet, strings resource, and the `VerenuOverlayView`, `VerenuKeystore`, and `VerenuDictationService` sources that the Android port merge dropped, so `scripts/android-sync.mjs` and the Gradle build succeed again. The shared `analytics` module now compiles on Android (still only registered on desktop), and a missing PostHog token no longer crashes debug builds on launch.
  - The accessibility service now starts the Rust backend itself (a background-mode `MainActivity` launch) when Android restarts the app process, re-pushes saved API keys once it is up, and waits for it before the first recording, instead of failing with "Could not start recording" until the app had been opened by hand.
  - Fixed a native abort when the audio stack outlived the Activity (opening Settings after backing out of the app): the audio context is now the Application context rather than a destroyed Activity reference.
  - The pill now appears reliably when the keyboard reopens on an already-focused field and over WebView text fields, sits just above the keyboard instead of covering the app's header, and uses larger text, bigger touch targets, and a high-contrast color pair.
  - Fixed layout bugs: the Home screen no longer overflows the phone width, setup permissions cards have side padding, Save/Clear no longer overlap in API Keys, and the bottom navigation clears the system gesture bar.

- Fixed Linux context groups falling back to Everywhere when an app's window class differs from its saved executable target. Matching now tries the captured process's executable after website and window-class matches.

- Calibrated the sidebar RAM bar to a locally saved average from the first hour of sampled usage. Typical usage fills half the bar on each device instead of topping out at a fixed 400 MB; calibration resumes across restarts and the MB reading stays unchanged.

- Fixed the Linux dictation pill drifting off-centre ("teleporting") and its Cancel/Confirm/Dismiss/Retry buttons ignoring clicks. The pill window no longer resizes with its content, which raced Hyprland and left the window box and the drawn pill out of step; it is now one fixed transparent window with the capsule centred inside it, so every state morph is pure CSS animation. Only the capsule itself accepts clicks, the rest stays click-through, placement is re-checked after mapping, and the window stays mapped long enough for the exit animation to play. Verenu also clears a user `no_focus` rule on the mapped pill, since Hyprland otherwise skips the pointer entirely for such windows.
- Fixed Linux double-tap hands-free activation starting a second recording and showing a short-recording error. The first tap's capture now stays open for conversion. Modifier-prefix rejection uses a separate action so mouse-mapped Ctrl+Super and other shortcuts cannot cancel an active hands-free dictation.

- Fixed Linux Settings showing Windows wording: Start on Boot now says "Launch Verenu when you log in" and Appearance says "Follow your desktop".

- Reworked user-facing errors for recording, provider access and request limits, local models, downloads, backups, sync, permissions, and local data. Messages explain the failure and give a next step, preserve full recovery instructions, and link to the relevant settings where available. Copy, model deletion, and update failures now show feedback instead of only logging an error; a failed update check no longer reports that Verenu is up to date.
- Linux recording failures now explain when no microphone is detected and suggest reconnecting it or selecting another input in General settings.

- Fixed LAN sync skipping edits made during snapshots, partially applying failed batches, losing Context snippet assignments across batches, retaining removed app targets, and lowering lifetime totals from stale relayed counters. Manual sync now waits for the result, receiving peers update their status, and incomplete transfers or settings-save failures report errors. Pairing uses its full approval deadline and confirms only after trust is saved. Added a focused `npm run test:sync` gate with encrypted two-device fixtures and three-device relay coverage on one machine.

- Reduced Linux memory retention by disabling WebKit's browser resource and back/forward page caches in the main window and dictation pill. On glibc builds, freed native allocations are returned to the OS during the existing idle maintenance cycle, including after local models unload.
- Fixed a WebKitGTK crash on Hyprland/Wayland by forcing the shared-memory renderer transport. Disabling DMA-BUF left accelerated compositing without a backing store.
- Fixed the September 21 nightly build failure: bumped `@tauri-apps/plugin-notification` to 2.4.0 to match the Rust crate (Cargo had resolved `tauri-plugin-notification` to 2.4.0 while the npm lockfile still pinned 2.3.3, and the Tauri CLI aborts on any major/minor mismatch). Also added a PR check that fails fast when any Tauri plugin's Rust and npm versions disagree, so this class of drift breaks a PR instead of a release.
- Fixed Linux automatic spacing and capitalization focus lookup: AT-SPI Collection queries now use valid match modes and find editable fields directly instead of focused browser document containers. Accessibility is requested at startup, metadata queries activate Chromium/Electron renderer trees, and rich-text cursor reads follow embedded paragraphs to their actual text and caret. Insertion logs report context availability without user text.
- Fixed Linux dictation reliability for hold-to-talk and hands-free: the portal hotkey thread now reconnects with backoff instead of dying silently (a dead thread stranded recordings with no way to stop them), and releasing then quickly double-tapping the chord carries the just-recorded audio into the new hands-free session instead of stranding it behind a Continue offer.
- Added Escape-to-cancel on Linux: a trigger-less portal cancel action dispatched through a bare-Escape Hyprland bind that is only installed while a chord is held, hands-free is active, or transcription is running — Escape is never swallowed while idle.
- Made the Linux dictation pill clickable in button-bearing states (hands-free Confirm/Cancel and error/retry controls), so a hands-free dictation can be ended with the mouse. Click-through is still used for passive states and the first-reveal guard keeps the pre-realize Wayland abort impossible.
- Ported the microphone mute-button dictation toggle to Linux: a mute→unmute pulse on the selected mic (PipeWire source mute via `pactl subscribe` plus a digital-silence PCM fallback for USB buttons that zero samples without flipping a mute flag) toggles hands-free dictation, matching the Windows behavior.
- Fixed inflated word totals: history, lifetime, and daily counts now use spoken words (snippet triggers and punctuation-only tokens excluded) instead of a raw whitespace split. Existing databases are repaired on open, refunding the overcount from the lifetime total.
- Fixed multiple Verenu sessions running at once on Windows: a newer launch
  now asks the older session to shut down cleanly and takes over after it has
  released its instance lock, including when dev and packaged builds overlap.
- Added a Developer setting to automatically enable Developer mode on startup.
- Removed the microphone calibration step and Audio-page calibration controls; manual microphone gain remains available. Rejected quiet or speech-free captures now make a quick follow-up dictation more sensitive for a bounded number of attempts, including explicit retries.
- Fixed Windows dictation appearing to freeze after CPAL reported that the
  microphone stream was no longer available; the dead recording now shuts down
  promptly and shows a microphone error instead of later reporting a misleading
  "Recording too short" rejection.
- Added sub-apps: a place inside an app (a Discord server, a Slack workspace, a VS Code project) matched by a window-title rule. Press Ctrl+Alt+Shift+S (Cmd+Option+Shift+S on macOS) in the window to capture it; Verenu suggests a rule, shows whether it matches, and saves it with a name to the sub-app list. Add sub-apps to a context group from its page with **Add sub-app**; each belongs to one group at a time. Assigned sub-apps take priority over website and app targets and show on the pill as "Context · Sub-app". **Settings → Sub-apps** lists them, removes them, and changes the capture shortcut. Sub-apps sync as their own list and are included in backups with the same cross-device app matching.
- On Omarchy, the **System** appearance now follows the active Omarchy theme across the whole UI (the separate Omarchy choice is kept only for installs that already saved it): surfaces, text, lines, overlays, shadows, the dictation pill, error/success/warning colors (when the theme's red, green, and yellow are vivid enough to read as status), and the default accent come from the theme's `colors.toml`, light/dark follows the theme, tray icons match, and `omarchy theme set` applies live without a restart. A custom accent still takes precedence.
- Fixed Linux dictation misfires on Omarchy: Ctrl+Super is also the start of many Omarchy shortcuts, so pressing another key while the chord is held now cancels that dictation instead of recording or transcribing it. It no longer arms the double-tap hands-free gesture, and a chord pressed within 400 ms of such a shortcut is ignored. Space (hands-free) and Escape (cancel) still work during a hold. Omarchy themes now keep text and lines neutral instead of tinting them with the theme color.
- Added a **Custom** appearance mode on Windows, macOS, and Linux: enter background, text, and optional sidebar and surface colors as hex codes (or start from a Catppuccin, Tokyo Night, Nord, Gruvbox, or Solarized preset) and the whole UI, dictation pill, native title bar, and tray icon follow live. The palette is validated, stored as `custom_theme`, and included in backups; like the other appearance settings it stays device-local.
- Made backups portable across devices: restoring a backup rebinds each Context's app targets to the closest app installed on the new device (for example `chrome.exe` to `google-chrome`), leaves out apps with no local match so the Context still imports and can be edited later, and reports linked, matched, and left-out apps in the import summary. Linux app icons now load from desktop entries and the icon theme.
- Brought Linux closer to Windows feature parity: Mute PC Audio now mutes the default PipeWire/PulseAudio output while dictating and restores only a sink Verenu muted; smart capitalization/spacing and AutoLearn read the focused text field through AT-SPI (Chromium and Electron apps need `--force-renderer-accessibility`); website contexts read the browser address bar through AT-SPI; dictation payloads are kept out of clipboard history and the clipboard is cleared afterwards when nothing textual was there before; Context targets are tagged per platform so synced Windows/macOS targets no longer show or match on Linux, and the target app is resolved even after focus moves.
- Added a developer-only **Ruin accessibility** toggle that dumps a compact diagnostics block into the OS accessibility tree for agent SnapShots: git SHA/branch/dirty, pipeline events, providers, mic, and stable `data-debug-id`s. Off by default. It does not include API keys, clipboard phrase, cleanup prompt text, logs, or dictated history.
- Added an optional Windows Audio setting to toggle hands-free dictation with a mute→unmute pulse on the selected microphone. Matches the selected mic across WASAPI/CPAL name variants, watches the endpoint mute bit plus USB/headset hardware mute controls on the capture path, and uses a digital-silence PCM fallback when those controls are absent — releasing that idle capture client before dictation opens the mic so the pill visualizer is not starved. Mute watching runs only while the setting is on.
- Removed orange from the native and packaged app icons. Runtime icons now use a pure black-and-white pair that follows light and dark appearance modes instead of the selected accent color.
- Fixed hands-free conversion from an active hold-to-talk dictation ending when
  the original Windows modifier chord is released; stale release events now
  have a three-second handoff window before hands-free stop gestures are
  accepted.
- Replaced the Insights speaking-pace dial with a tick-scale meter that matches the rest of the page: the tile is now left-aligned on the same baseline grid as its two neighbours, quarter marks make the scale readable at a glance, the scale ceiling grows to always clear your personal best, and the best itself is marked on the scale instead of only being named underneath.
- Fixed the main window stuttering and spiking CPU when dragged or resized quickly on Windows: moving the window no longer re-runs title-bar and icon updates per mouse event, resizes coalesce to a single refresh once the size settles, title-bar metrics are only forwarded when visible values change, and themed icons reuse cached artwork instead of re-rendering on every focus or settings event.
- Restored the Windows tray icon's classic proportions — the accent-theming rework had grown the waveform to fill ~70% of the tile edge to edge; it sits back inside the tile with real margins, keeping the sharper native-size rendering.
- Recolored the running tray, taskbar/window, and macOS Dock icons with the selected accent while preserving their existing light/dark backgrounds and bundled launcher icons.
- Added an automatic light-mode contrast backdrop for transparent app icons whose artwork is mostly white, keeping them visible without changing ordinary colorful or dark icons.
- Switched the default accent from terracotta to theme-neutral black in light mode and white in dark mode. Custom accents still override the full accent scale; the Home hotkey tile keeps colored accents exact and only lifts near-black neutrals to white for contrast.

- Context group app targets now survive versioned/nightly app updates by matching a close replacement name with publisher/developer evidence on Windows and macOS.
- Prevented the same "Often mistranscribed as" variant from mapping to multiple terms in one context group, with prompt filtering for older conflicting data.
- Reworked cleanup prompting around one default shared by every model, with explicit rule priority, conservative ambiguity handling, multilingual preservation, self-corrections and repair commands, spoken symbols and spelling, technical-token reconstruction, restrained formatting, safer number treatment, and context-assisted disambiguation.
- **The cleanup prompt is now a single template used by every model**, edited from Clean-up → Edit prompt. It used to be stored per model, so an edit made on your default was silently ignored the moment a fallback model took over. An existing per-model edit is carried over.
- Fixed Gemini 3 requests failing with `Thinking level MINIMAL is not supported for this model` — affected both cleanup and transcription on newer Gemini 3 flash models, which accept `low` but not `minimal`.
- Fixed the cleanup prompt editor opening in the bottom-right corner instead of centred.
- The model picker now lists every model a provider reports, not just the curated ones, behind a **Show N more models** toggle at the foot of the list — so a newly released model is selectable the day it ships without waiting for a Verenu update. Non-text models (image, TTS, embedding, and similar) are filtered out.
- Fixed the model picker's search icon sitting below the centre of the search field.
- Fixed the settings sidebar highlight blinking off the item under the cursor while the selection pill travelled to it.
- Added a **Legacy pages** toggle (Settings → General) that hides the standalone App Mappings, Dictionary, and Snippets pages by default in favor of Contexts, and brings them back — along with a heads-up that they're no longer actively maintained — when turned on.
- Contexts is now hidden from the primary nav while Legacy pages is on, so there's only one place to manage app tones, vocabulary, and snippets at a time.
- Fixed the App Mappings list playing an entrance animation for every existing row on first load; rows now only animate on actual reorder, matching the Dictionary list.
- Reduced the context group name limit from 120 to 30 characters and added a live character counter, and enforced it client-side with an input `maxlength` (previously unenforced, allowing names that overflowed the page).
- Fixed the context icon color picker rendering as a full-width bar instead of a compact popup near the click point.
- Context group websites are now checked for DNS existence before being saved, so a typo can't silently create a website target that will never match anything.
- Added a subtle pop-in animation when adding an app or website to a context group, without replaying it for the rest of the list when switching between context groups.
- Added `docs/CONTEXTS.md` and marked App Mappings, Dictionary, and Snippets as legacy pages across the docs.
- Fixed the Insights page stacking its summary tiles too early on narrow windows, stranding the gauge in a half-empty row — the hero band, heatmap rail, and vocabulary sections now adapt to the available column width and stay side by side down to the minimum window size.

- Reduced dictation memory allocations by encoding Gemini and OpenRouter audio directly into a preallocated JSON request, releasing cached WAV uploads before cleanup, and trimming completed PCM buffers before retry storage. Native windows now load browser test mocks only in browser preview mode. Added repeatable audio-request allocation measurements.

- Deferred secondary pages, settings sections, onboarding, and optional dialogs until opened. The development worker skips loading the application UI. Settings search and keyboard focus wait for deferred sections, and failed loads offer an app reload.

## 0.18.1

- Removed orange from the native and packaged app icons. Runtime icons now use a pure black-and-white pair that follows light and dark appearance modes instead of the selected accent color
- Fixed hands-free conversion ending when the original Windows modifier chord is released, and added a short handoff window for stale release events
- Replaced the Insights speaking-pace dial with a tick-scale meter that keeps the best pace visible and scales to the user's history
- Reduced Windows main-window CPU spikes and stutter during fast dragging or resizing by coalescing refresh work and caching themed icons
- Restored the classic proportions of the Windows tray icon while keeping accent coloring for running tray, taskbar, window, and macOS Dock icons
- Added light-mode contrast backdrops for transparent icons whose artwork is mostly white
- Switched the default accent to black in light mode and white in dark mode while preserving custom accent colors
- Made context app targets survive versioned and nightly app updates when publisher or developer evidence matches
- Reworked cleanup prompting around one shared template with clearer rule priority, multilingual preservation, safer number handling, and context-assisted disambiguation
- Added a model picker option for every text-capable model reported by a provider, while filtering out image, TTS, embedding, and other non-text models
- Added a Legacy pages toggle for the older App Mappings, Dictionary, and Snippets pages, with Contexts now serving as the primary home for location-aware settings
- Added website DNS validation, context name limits, character counts, compact icon color selection, and focused add-item animations
- Improved Insights responsive layout on narrow windows and fixed several settings, prompt-editor, and list-animation issues

The 0.18.1 release description, installer assets, hashes, and platform-specific download details are available on the [GitHub release page](https://github.com/MONKE2525E/Verenu/releases/tag/v0.18.1).

## 0.18.0

The 0.18.0 release notes, installer assets, hashes, and platform-specific download details are available on the [GitHub release page](https://github.com/MONKE2525E/Verenu/releases/tag/v0.18.0).

## 0.15.1 beta - Audio & Polish

- Added configurable dictation sound cues for start, stop, cancel, and error transitions.
- Added a Windows-only option to pause active media sessions (Spotify, YouTube, etc.) during dictation and resume them afterward.
- Added macOS-only exclusive microphone access so other apps can't capture audio while dictating.
- Switched the Windows main window to native OS title bar chrome, recolored to match the app theme.
- Fixed the Windows tray Relaunch action silently closing the app instead of restarting it.
- Hardened the dictation pill window against exposing native chrome in hands-free mode on Windows.
- Evened out the Dictionary and Snippets sort segmented controls' selection highlight.
- Fixed Caps Lock casing getting partially undone by contextual capitalization when dictating mid-sentence.
- Fixed the dictation pill clipping on the first recording shown after switching monitors.
- Tightened the hands-free start chime timing and animated the pill's move across monitors.
- Signed macOS release builds with a persistent self-signed identity so permission grants carry over between updates.
- Replaced the periodic HTTP connectivity probe with native OS connectivity checks, eliminating background network traffic for that check.
- Organized project documentation around `docs/` instead of scattering full policy files at the repository root.
- Added GitHub issue templates for bug reports and feature requests.
- Added public docs for architecture, testing, release process, troubleshooting, security, support, code of conduct, and RAM/reliability constraints.
- Added npm package metadata and macOS signing script aliases so documented commands resolve.
- Refactored oversized backend Rust modules (`commands/settings.rs`, `main.rs`) into focused modules with no behavior change.

Installer hashes:

| File | SHA-256 |
| --- | --- |
| `Verenu_0.15.1_Apple_Silicon.dmg` | `51eb5d40be4814d460efe9baf6c6214652961dd3abbeee2e32b19178899b1529` |
| `Verenu_0.15.1_Intel.dmg` | `6770f15a82809c09741d4ef2b64e4428798bc865f0b7aea2be87a964e8407c2f` |
| `Verenu_0.15.1_x64-setup.exe` | `ac36e31871a4919e08f0d0a17386df65c77b4374f96eab908d15643d83de0f8c` |
| `Verenu_0.15.1_x64_en-US.msi` | `f605697435d7ce75ea1e1f6ecebc0e370d61158e8c97e410a1c8ab4b8fbe7ba6` |

## 0.15.0 beta - Polish

- Target-monitor dictation pill: shows the recording pill on whichever monitor the user is typing on instead of always using the primary monitor.
- Sharper pill visualizer: renders even visualizer bar widths across monitors with different DPI scaling.
- Smarter contextual capitalization: fixes a per-app style leak and corrects casing in empty and Chromium text boxes.
- Cleaner casual injection: skips a clipboard sniff that could corrupt output in the Very Casual profile.
- Sharper cleanup intensity: tightens Verbatim, Light, Medium, and Direct cleanup contracts.
- Native update notifications: surfaces new versions inside the app and supports in-place installs.
- Hardened macOS reliability: uses `RegisterEventHotKey`, stable code signing, and more reliable paste behavior.
- Slimmed backend: moved to backend-owned settings, trimmed dependencies, hardened logging, and split oversized modules.

Installer hashes:

| File | SHA-256 |
| --- | --- |
| `Verenu_0.15.0_Apple_Silicon.dmg` | `e3e02168ffe50eb9b62fa71f7bef75c592686d4544bcdd137f5ec5fed3d1aeba` |
| `Verenu_0.15.0_Intel.dmg` | `f66b6135b33bac9c07149e82fe8928f2b17b80c8d71244b540f1e42e196e144c` |
| `Verenu_0.15.0_x64-setup.exe` | `806375b6d295a0e4a47c1c0efa4a9e4c2b8c75ad7549d3fa0c6a0acf4c7bdab4` |
| `Verenu_0.15.0_x64_en-US.msi` | `b6cd8cf038cbb8b999e3b043f799ed1e504958eaa704ad41dbf305674459f3f6` |

## 0.14.1 beta - macOS Installer Fix

- Rebuilt macOS installers with ad-hoc code signing so the app bundle has a valid structural signature on Apple Silicon and Intel Macs.
- Bumped Verenu to `0.14.1`.
- Enabled Tauri ad-hoc macOS signing with `signingIdentity: "-"`.
- Updated the installer workflow to derive DMG filenames from `package.json`.
- Added CI verification that mounts each macOS DMG and runs `codesign --verify --deep --strict --verbose=4`.
- Confirmed both macOS CI builds report `Signature=adhoc`.

Installer hashes:

| File | SHA-256 |
| --- | --- |
| `Verenu_0.14.1_Apple_Silicon.dmg` | `C26886D38C3E686118D43165C061177092064CE6CD6FEF2B417BD5FBC7B74B97` |
| `Verenu_0.14.1_Intel.dmg` | `F451FA4E0B41A61610354215B8ADDE0A6133771AA84630F6791740DD70BFC028` |
| `Verenu_0.14.1_x64-setup.exe` | `5D2A5E99AC4D0CA03036F6EE39C6477F65821881D88CA1724159AF3967` |
| `Verenu_0.14.1_x64_en-US.msi` | `546F1F3A4436649A51C6F5753FF6D95AED3E197E1EC17F77B2EB1B220E94DC21` |

## 0.14.0 beta - UI Refresh

- Modular setup wizard: refactored first-run setup into focused per-step components.
- Reorganized settings: subdivided settings into labeled subgroups with setup wizard toggles.
- Streamlined API keys row: Save and Clear now flip inline with status feedback and accurate Gemini model listings.
- Automatic Caps Lock detection: adjusts dictation casing when Caps Lock is active.
- Full-screen cleanup prompt editor: added a dedicated modal with stronger injection handling.
- Per-model cleanup prompt templates: added provider-specific templates, a refusal guard, and an Advanced Models UI.
- Redesigned pill error display: matched the in-app error toast styling.
- Virtualized history list: improves scrolling performance for long histories.
- History retention enforcement: enforces configured retention windows and confirms deletion of older entries.
- Fixed UI rough edges: hands-free pill flicker, scrollbar alignment, and history retention dropdown ellipsis during animation.
- Hardened auto-update and autostart reliability with safer DB backup, async I/O changes, Windows autostart registry fixes, and backend module splits.

## 0.13.0 beta - Verenu

- Per-app cleanup intensity: app mappings can override global Verbatim, Light, Medium, or Direct cleanup intensity.
- Smarter auto-learn promotion: distinctive corrections can promote after one high-confidence session, while everyday-word corrections stay safer and contextual.
- Database self-healing: repairs installs left with a missing column after interrupted migrations.
- Hardened data migration, snippet usage tracking, and SQLite WAL preservation across updates.
- Removed the last Open Flow to Verenu transition code.
- Added the Data & Privacy documentation for local storage and provider data flow.

## 0.12.1 beta - Verenu Transition

- Forward-compatible update checks: checks both Open Flow and upcoming Verenu release sources so updates and About links keep working through the rename.

## 0.12.0 beta - macOS

- JSON backup import and export for settings, dictionary entries, and snippets.
- Secure macOS API key storage using native Keychain APIs.
- Two-phase microphone calibration for normal speech and whispering.
- Advanced snippet triggering with comma-separated triggers, punctuation-tolerant matching, and cache isolation.
- macOS permissions overhaul with visual status rows, Keychain checks, and polling.
- Self-injection detection with clipboard fallback when the app itself has focus.
- Contextual capitalization hardening using Windows UI Automation and macOS process hints.
- Optimistic UI updates for dictionary and snippet interactions using atomic SQLite returning statements.
- All-in-one test runner with a unified harness and mock provider support.

## 0.11.0 beta - Polish

- Transcription retry from Home: failed transcriptions can be rerun from history.
- Encrypted API key storage with Windows Credential Manager.
- Model fallback reliability: fallback chains try all configured fallback models in order.
- Pipeline feedback improvements for quality-gate rejections and API key errors.
- Model settings UX redesign with clearer fallback controls and labels.
- Dictionary and snippet limit UX with counters and nudges.
- Injection behavior refinements for contextual capitalization and auto-spacing.
- Offline and settings UI polish.
- Corrected WPM metrics to use raw transcription word counts.

## 0.10.0 beta - Local-First AI Dictation

- Automatic microphone gain calibration during setup and from Settings.
- Smart output rejection: deleting dictated text shortly after injection prunes stale cleanup cache and related auto-learn substitutions.
- Developer mode with verbose pipeline logs, downloadable session logs, and Force Setup On Launch.
- Onboarding improvements with restored appearance selection and no-scroll layout.
- Snippet inspector polish for modal height, overflow, and long previews.
- Auto-learn reliability hardening with stable-text gates, session deduplication, and tighter candidate filtering.
- Numeric cache normalization so numeric and written forms share cache keys.
- Profanity handling precedence fix across cleanup intensity and tone.
- Dictionary input clamping with code-point-safe truncation.
- Unified scrollbar styling across scrollable surfaces.

## 0.9.0 beta - Caching

- Local cleanup cache for repeated transcription cleanup responses.
- Cache key normalization across punctuation, casing, and trailing periods.
- Settings reorganization: moved core behavior, API fallback, auto-learn, Audio, and Apps into clearer locations.
- Auto-learn hardening with per-session stable text gates, within-session deduplication, and tighter candidate filtering.
- App mappings search fixes and scrollbar polish.
- Auto-learn regression matrix with JSON fixtures and a PowerShell harness.

## 0.8.0 beta - Local-First AI Dictation

- Spoken language selector for better non-English transcription accuracy.
- Silent auto-update without console flash or PowerShell execution-policy prompts.
- Pre-update SQLite database backup.
- App mappings redesign with a dedicated editor component.
- Dynamic theme-aware tray icon.
- CI and dependency automation with GitHub Actions and Dependabot.

## 0.7.0 beta - Local-First AI Dictation

- Contextual capitalization that can inspect text before the cursor and lowercase mid-sentence dictation.
- Voice input for dictionary and snippet fields.
- Smarter auto-learn detection using anchored spans, better word alignment, duplicate-session guards, and safer promotion rules.
- Relevant dictionary prompting so cleanup prompts prioritize matching dictionary entries.
- Manual update controls in About and a home update banner.
- Offline awareness with connectivity checks and a home-screen indicator.
- Pipeline hardening across transcription, cleanup, snippets, dictionary substitution, and injection.
- Audio reliability improvements with reduced buffer churn and hardened mono mixing.
- Database migration cleanup with `user_version` gating and safer SQLite lock handling.
- Theme system cleanup in `theme.css`.
- Expanded Rust unit and Playwright smoke test coverage.

## 0.6.0 beta - Local-First AI Dictation

- Redesigned setup and quick settings layout with a wider two-column grid.
- Stronger prompt injection protection with strict `<raw_dictation>` tag isolation.
- Unified quota error handling for API fallback behavior.
- Enhanced auto-learn dictionary with Windows UI Automation COM guards, fallbacks, and a longer monitor window.
- Inline audio transcription request support.
- Clipboard and hotkey hardening for Windows Unicode handling and registration failure paths.
- Robust update parsing for normalized release tags and comparisons.

## 0.5.0 beta - Local-First AI Dictation

- In-app update checks and one-click install.
- Microphone gain control with adjustable boost for quiet microphones.
- Dictionary substitution fixes for empty patterns and UTF-8 mixed-case behavior.
- Snippet period deduplication fix.
- Pill UI processing animation upgrade.

## 0.4.2 beta - Local-First AI Dictation

- Initial public Open Flow beta release notes.
- Windows desktop dictation app with hold-to-record hotkey, multiple AI providers, real-time recording indicator, cleanup profiles, clipboard injection, snippets, transcription history, settings, themes, and local-first storage.

## Related Docs

<p align="center">
  <a href="RELEASE.md"><img alt="Release Process" src="https://img.shields.io/badge/Release-Process-c44632"></a>
  <a href="README.md"><img alt="Docs Index" src="https://img.shields.io/badge/Docs-Index-5b554a"></a>
  <a href="../installers/README.md"><img alt="Installers" src="https://img.shields.io/badge/Installers-Layout-7e7266"></a>
</p>
