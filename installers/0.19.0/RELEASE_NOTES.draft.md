<!-- Draft for the 0.19.0 GitHub release. Replace pending VirusTotal entries after scanning every asset. -->

# Verenu 0.19.0 - Arch Beta

**Verenu** is a free, open-source AI dictation app for Windows, macOS, and Linux. No subscriptions. Use your own provider key or choose local models.

## What's New in 0.19.0

- **Arch Linux beta for Omarchy** - Install the x86_64 pacman package with `sudo pacman -U ./verenu-0.19.0-1-x86_64.pkg.tar.zst`. It reuses the Linux AppImage, adds a desktop launcher, and declares the GTK, WebKitGTK, PipeWire, Secret Service, and desktop portal dependencies. The Hyprland portal is optional for other Arch desktops, and the package does not edit Omarchy-owned system files
- **Wayland dictation controls** - Register shortcuts through the XDG Desktop Portal. On Omarchy Quattro the default is Ctrl+Space, which avoids the Super+Space launcher binding. Escape cancels only while dictation is active; the pill accepts clicks for hands-free confirmation and errors. Portal hotkeys reconnect after service interruptions
- **Linux app and website context** - Match the focused app or browser site, then apply that context's tone, cleanup, vocabulary, and snippets. AT-SPI focus reads also power smart spacing, capitalization, and AutoLearn. Dictated text stays out of clipboard history, and Verenu restores the previous clipboard state when possible
- **Audio controls that follow the selected devices** - Mute PC Audio while dictating and restore only the output Verenu muted. Optional microphone mute-button triggers can toggle hands-free dictation on Windows and Linux, including USB mute buttons that report silence without a mute flag
- **Adaptive microphone capture** - Removed the calibration step and its Audio settings. Manual gain remains available; after quiet or speech-free captures, a bounded sensitivity adjustment helps the next attempt
- **One cleanup prompt across model fallbacks** - Edit one shared template for every model. Existing per-model edits carry over, and fallback models keep the same instructions. The model picker can show every text model a provider reports, and Gemini 3 no longer fails on an unsupported MINIMAL thinking level
- **Contexts that keep their library in order** - Move or duplicate context setup, scope AutoLearn corrections to the right group, and prevent one mistranscription variant from mapping to conflicting terms. App targets survive versioned app updates when publisher evidence matches, and website targets are checked before saving
- **More dependable Insights** - Word totals count spoken words rather than raw whitespace, exclude snippet triggers and punctuation-only tokens, and repair existing databases. Range totals stay aligned, the pace meter marks a personal best, and historical totals survive retention cleanup
- **Safer recovery and smoother Windows behavior** - Stop promptly when microphone streams fail, preserve dictations when recovery storage fails, and let a new Windows session take over cleanly. Window drag and resize work is coalesced to avoid CPU spikes
- **Icons and appearance** - Runtime icons follow light and dark mode, running tray and window icons keep the chosen accent, and light mode adds contrast behind mostly white transparent icons. The Windows tray icon keeps its original proportions
- **Experimental Android build path** - The shared app code now includes an adaptive Android shell, a keyboard-aware accessibility overlay, and Android Keystore-backed API key storage. Android local models and some desktop context features are not supported yet, and this release does not include an APK

## Features

### Transcription

- Hold a platform hotkey to record and insert the result into the focused app
- Use hands-free dictation for continuous recording
- Choose local Parakeet V3 transcription or a supported cloud provider
- On Linux, approve the requested XDG Desktop Portal shortcut; the default shortcut is Ctrl+Space

### Text cleanup

- Choose cleanup intensity, tone, and formatting behavior
- Use context-specific instructions for apps and websites
- Keep cleanup instructions consistent when a fallback model takes over
- Use local cleanup models or a supported cloud provider

### Contexts, vocabulary, and snippets

- Group apps and websites with their own tone, cleanup rules, custom instructions, vocabulary, and snippets
- Use AutoLearn to build correction mappings from repeated edits
- Keep Everywhere as the fallback context

### History and Insights

- Review local dictation history and retry failed transcriptions
- Track speaking pace, usage, and learned vocabulary on device
- Import and export app data using local backup files

### Platforms and privacy

- Windows 10/11, macOS on Apple Silicon or Intel, and Linux x86_64 AppImage
- Arch x86_64 pacman package is beta; Omarchy Quattro/Hyprland is the primary Arch target
- API keys use Windows Credential Manager, macOS Keychain, or Linux Secret Service
- Product analytics are pseudonymous and can be turned off in Settings → Privacy

## Getting Started

1. Download the installer for your platform. Arch users can install the beta package with the command above
2. Choose local models or add an API key for a supported cloud provider
3. Hold Ctrl+Win on Windows, Option+Space on macOS, or Ctrl+Space on Linux and start talking
4. Release the shortcut to transcribe, clean up, and insert the text in the focused app

## Shortcuts

- **Windows hold-to-record** - Hold **Ctrl+Win** to record, then release to transcribe and insert
- **macOS hold-to-record** - Hold **Option+Space** to record, then release to transcribe and insert
- **Linux hold-to-record** - Hold **Ctrl+Space** to record, then release to transcribe and insert

## Lightweight & Local

- Around 200 MB idle target, using native Tauri rather than Electron
- History, contexts, and settings live in local SQLite storage
- API keys stay in the operating system's credential store
- Optional pseudonymous analytics can be disabled in Settings → Privacy

## VirusTotal Review

- [Verenu_0.19.0_x64-setup.exe](VirusTotal report pending)
- [Verenu_0.19.0_x64_en-US.msi](VirusTotal report pending)
- [Verenu_0.19.0_Apple_Silicon.dmg](VirusTotal report pending)
- [Verenu_0.19.0_Intel.dmg](VirusTotal report pending)
- [Verenu_0.19.0_x86_64.AppImage](VirusTotal report pending)
- [verenu-0.19.0-1-x86_64.pkg.tar.zst](VirusTotal report pending, Arch beta)
