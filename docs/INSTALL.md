# Install Verenu

Verenu is a free, open-source dictation app for Windows, macOS, and Linux. There is no account or subscription. Cloud providers use API keys that you provide, while local transcription and cleanup need no key. See [Add Your API Key](API_KEYS.md).

## Linux (Omarchy Quattro / Arch / Hyprland)

Verenu supports native Wayland dictation on current Omarchy Quattro. Install `webkit2gtk-4.1`, `libappindicator-gtk3`, `pipewire`, `wireplumber`, `xdg-desktop-portal`, `xdg-desktop-portal-hyprland`, and a Secret Service provider such as `gnome-keyring` or KWallet. The installer is an AppImage; an Arch package can use the same release bundle.

On first launch, approve Verenu's **XDG Desktop Portal** shortcut request. The default is **Ctrl+Super**. On Hyprland, Verenu installs it as a modifier-only binding and keeps the portal action for authenticated dispatch. Keep the portal and Secret Service unlocked; Verenu never writes API keys to settings or SQLite.

On Omarchy Quattro, Verenu registers its shortcuts automatically in your user bindings. Open **Super+K** and search for **Verenu** to see dictation, **Ctrl+Alt+C** to copy the last dictation, the sub-app capture shortcut, and the conditional Space/Escape controls. Both left and right Ctrl/Super keys work, in either press order. Hold the dictation chord to record, release either key to finish, or double-tap the chord for hands-free. While holding a chord that does not already use Space, press **Space** to switch to hands-free. **Escape** cancels while recording or processing. Space and Escape remain listed with their conditions but are disabled when Verenu is idle, leaving those keys available to other apps. Changing Verenu's shortcuts updates the managed bindings without manual configuration. This integration requires Omarchy Quattro's Lua-based Hyprland configuration; older Omarchy versions and other desktops are not covered by automatic registration.

Before registering, Verenu checks the compositor's bindings. If a preferred shortcut is already assigned, it chooses a free alternative and shows the active shortcut and a conflict message in Settings. Home, setup, and Context capture hints follow the same active shortcuts. Conditional cancel/hands-free keys also avoid existing modified bindings, so your desktop may use function keys instead of Escape or Space. Desktop changes are checked while idle; removing a conflict restores the preferred shortcut. If every alternative is occupied, the action is marked unavailable rather than taking another binding. These checks cover compositor bindings, not shortcuts handled privately inside other applications. Saved preferences remain unchanged and do not sync a desktop-specific alternative to another device.

Verenu automatically installs its window rules in a separate managed block in your user bindings before showing its windows. The main window floats with a minimum size of 1100×700; the dictation pill floats above other windows without taking their initial focus. Both use the stable app ID **com.verenu.app**, including when the release binary or AppImage is renamed. Personal bindings outside Verenu's marked blocks are preserved. No manual window rules are required on Omarchy Quattro. If setup fails, inspect the shortcut status in Settings and run `hyprctl configerrors` for compositor configuration errors.

Volume, playback, and brightness keys remain usable while holding the dictation shortcut. A competing shortcut using a letter still discards the modifier-only dictation prefix.

Automatic spacing and capitalization use the focused field's AT-SPI accessibility text. Verenu requests accessibility at startup. Apps already launched with accessibility disabled may need a restart; fields that do not expose cursor text preserve the dictation without guessing spacing or sentence position.

## Windows

1. Download the latest installer from the [GitHub Releases](https://github.com/MONKE2525E/Verenu/releases) page:
   - `Verenu_X.Y.Z_x64_en-US.msi`, or
   - `Verenu_X.Y.Z_x64-setup.exe`
2. Run the installer and follow the prompts.
3. Launch Verenu. The first-run setup walks you through choosing local models or a cloud provider and adding a key when one is required.

Requires Windows 10 or 11. Verenu uses the built-in WebView2 runtime — no separate browser engine to install.

## macOS

1. Download the build that matches your Mac from the [GitHub Releases](https://github.com/MONKE2525E/Verenu/releases) page — Apple Silicon or Intel.
2. Move Verenu to your Applications folder and open it.
3. macOS may ask for permissions the first time you use Verenu. Dictation requires:
   - **Microphone** — to capture your voice while you hold the hotkey
   - **Accessibility** — to paste cleaned-up text into the app you're using and read text for corrections

Notifications are optional and only affect status and update alerts. The macOS hotkey does not require Input Monitoring.

If macOS prompts for your login password when Verenu first saves your API key to Keychain, choose **Always Allow** — this avoids repeated prompts later.

## Build from source

If you'd rather build Verenu yourself:

**Prerequisites**
- Node.js 24 LTS, matching CI
- Rust and Cargo
- Windows: WebView2 (usually already installed)
- macOS: Xcode Command Line Tools (recommended)

```bash
git clone https://github.com/MONKE2525E/Verenu.git
cd Verenu
npm install
npm run tauri build
```

For development and contribution setup, see [CONTRIBUTING.md](CONTRIBUTING.md).

## If your browser or OS blocks the install

Verenu isn't signed with a paid code-signing certificate, so Windows, macOS, and some browsers may warn you that the installer is from an "unknown publisher" or flag it as potentially unsafe. This is normal for open-source apps distributed outside an app store — here's how to get past each warning.

### Browser download blocks (Chrome / Edge)

When you download the installer, Chrome or Edge may say the file "isn't commonly downloaded" or "could be dangerous":

1. Click the small arrow or **`...`** next to the blocked download in your browser's download bar (or Downloads page).
2. Choose **Keep** (Chrome) or **Keep anyway** (Edge).
3. If prompted again with "Show more" details, choose **Keep anyway** / **Download anyway**.

### Windows SmartScreen

When you run the installer, you may see **"Windows protected your PC"** with a blue "Don't run" button:

1. Click **More info**.
2. Click **Run anyway**.

### macOS Gatekeeper

When you open Verenu for the first time, macOS may say it **"can't be opened because it is from an unidentified developer"**:

- **Easiest**: Right-click (or Control-click) the Verenu app and choose **Open**, then click **Open** again in the confirmation dialog.
- **If that option isn't available** (macOS Sequoia and later sometimes hide it): go to **System Settings → Privacy & Security**, scroll down to the security message about Verenu, and click **Open Anyway**. You may need to enter your password and confirm **Open Anyway** once more.

### Verifying the download yourself

If you want independent confirmation that an installer is clean, every release on the [GitHub Releases](https://github.com/MONKE2525E/Verenu/releases) page includes a **VirusTotal Review** section with a scan link for each installer file. Find the link matching the file you downloaded (e.g. `Verenu_X.Y.Z_x64-setup.exe`) and open it to see the scan results.

## Next step

Once Verenu is installed, continue with [Add Your API Key](API_KEYS.md) if you plan to use a cloud provider, or go straight to [Your First Dictation](FIRST_DICTATION.md) for a local setup.

## Related Docs

<p align="center">
  <a href="API_KEYS.md"><img alt="Add API Key" src="https://img.shields.io/badge/Next-Add%20API%20Key-c44632"></a>
  <a href="FIRST_DICTATION.md"><img alt="First Dictation" src="https://img.shields.io/badge/Then-First%20Dictation-5b554a"></a>
  <a href="TROUBLESHOOTING.md"><img alt="Troubleshooting" src="https://img.shields.io/badge/Help-Troubleshooting-7e7266"></a>
  <a href="README.md"><img alt="Docs Index" src="https://img.shields.io/badge/Docs-Index-2b2422"></a>
</p>
