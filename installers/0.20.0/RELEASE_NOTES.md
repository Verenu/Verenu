# Verenu 0.20.0 - Connected Dictation

**Verenu** is a free, open-source AI dictation app for Windows, macOS, and Linux. No subscriptions. Use your own provider keys or choose local models.

## What's new in 0.20.0

- **Device sync** - Added code-based pairing, saved Tailscale routes, copyable connection details, automatic synchronization, and relay support for multi-device setups. History and Insights refresh after sync
- **Sync settings** - Reworked paired-device rows and the connection editor, with clearer pairing progress and setup guidance
- **Paste in Chunks** - Added a desktop Context option that inserts smaller pieces with pauses for CLI prompts, preserves the clipboard, and stops remaining chunks when focus or clipboard ownership changes
- **Model choices** - Grouped cloud priorities and Local AI choices, with expandable model and fallback details. Cloud presets can prepare local recovery; cleanup failures preserve completed speech
- **Cleanup and caching** - Tightened cleanup instructions to preserve uncertainty and meaningful qualifiers. Exact request hashes, expiration, and privacy controls govern persisted cleanup results
- **Custom providers** - Added editable connection presets for OpenAI, Anthropic, and xAI compatible endpoints, including local servers. These presets prefill settings and do not certify vendor support
- **Appearance and instructions** - Added a theme gallery, saved custom themes, and editable cleanup and Personal Tone instructions
- **Linux controls** - Fixed Ctrl + Super shortcut capture, improved app icon resolution, and reduced the dictation pill's window and click area
- **Android source** - Added offline speech and cleanup models, app and website Context matching, and keyboard-aware layouts and pill controls. This desktop release does not include an APK

## Features

### Transcription

- Hold a shortcut to record, then release to insert the result in the focused app
- Use hands-free recording, supported cloud providers, or local speech models
- Configure fallback models for recovery

### Text cleanup

- Choose cleanup intensity and tone
- Edit preset instructions and use local or cloud cleanup models
- Apply vocabulary, snippets, and custom instructions from the matching Context

### Contexts and device sync

- Group apps and websites with their own dictation settings
- Use Everywhere as the fallback Context
- Pair devices and synchronize supported data over local or configured remote routes; provider and model choices remain local

### History and privacy

- Review local history, retry dictations, and track usage in Insights
- Import and export backups
- Store provider keys in the operating system's credential store
- Turn off optional pseudonymous analytics in Settings > Privacy

## Getting started

1. Download the installer for your platform. Arch users can install the beta package with `sudo pacman -U ./verenu-0.20.0-1-x86_64.pkg.tar.zst`
2. Choose local models or add a key for a supported cloud provider
3. Hold the shortcut listed below and start talking
4. Release the shortcut to transcribe, clean up, and insert the text

The Arch package remains beta. Linux's primary desktop target is Omarchy Quattro on Hyprland/Wayland.

The macOS apps use a self-signed certificate and are not Apple-notarized. Gatekeeper can block the first launch; review the file's scan and signing status before deciding whether to allow it.

## Shortcuts

- **Windows** - Hold **Ctrl+Win** to record, then release to insert
- **macOS** - Hold **Option+Space** to record, then release to insert
- **Linux** - Hold **Ctrl+Space** to record, then release to insert. Approve the requested desktop portal shortcut

## Lightweight and local

- Native Tauri desktop app with an idle memory target of around 200 MB
- Local SQLite history and Context storage
- API keys use Windows Credential Manager, macOS Keychain, or Linux Secret Service
- Local models support offline dictation; cloud models send requests to the selected provider

## VirusTotal review

- [Verenu_0.20.0_x64-setup.exe](https://www.virustotal.com/gui/file/de1a8adcddeaf545701acda90e2aeeb626150510eb23479288ef08f93115af96)
- [Verenu_0.20.0_x64_en-US.msi](https://www.virustotal.com/gui/file/777dee55a9a26c8813481bc3c4aef4cc6592b0623cb76c70992318b22ba864a1)
- [Verenu_0.20.0_Apple_Silicon.dmg](https://www.virustotal.com/gui/file/5367180e4640d8537fc986cc540b5af41f328941052ee724c89d5e3f57f2f6f5)
- [Verenu_0.20.0_Intel.dmg](https://www.virustotal.com/gui/file/46c940ab8a2b2b0349c7ed41bfa410c35da3e40931da2178a39aab77d37b5a36)
- [Verenu_0.20.0_x86_64.AppImage](https://www.virustotal.com/gui/file/7aa967c1dc71a65155acca23a4e101a91e33e8d1c6b7f1e5a3697b290870d193)
- [verenu-0.20.0-1-x86_64.pkg.tar.zst](https://www.virustotal.com/gui/file/5757bd4d5e27a0ffe85c965818bf7baac3d36e3079bc6210767493195aa176be)

At the initial scan, DeepInstinct flagged the Windows EXE; it showed 1/71 detections. The MSI, Intel DMG, AppImage, and Arch package showed no detections. The Apple Silicon scan was still running when these links were collected. Consult the live reports for current results; this release does not claim every scanner passed.

`SHA256SUMS.txt` accompanies all six installer assets. The packages are the unchanged files from [installer build 37256725742](https://github.com/Verenu/Verenu/actions/runs/37256725742), built from source commit `0253c95b6569f636b78f7252a2439087792df6ae`.
