# Release Process

## Android update publication contract

Android uses the existing startup, six-hour, and reconnect update checks and
stable/beta preference. No APK job is enabled in the nightly workflow yet.
Until a compatible APK is published, Android receives no update offer.

Publish signed, standalone APKs on the same GitHub Release as the desktop
assets, using these exact, case-sensitive names:

- `Verenu_<version>_android_arm64-v8a.apk` for ARM64 phones.
- `Verenu_<version>_android_x86_64.apk` for x86_64 devices.
- `Verenu_<version>_android_universal.apk` as an optional fallback containing
  every supported ABI and its native inference runtimes.

For example, `Verenu_0.21.0-nightly.20261006_android_arm64-v8a.apk` belongs
on a prerelease from `master`. A stable APK uses `Verenu_0.21.0_android_arm64-v8a.apk`
on a normal release. The filename version must match the app's semantic version.
Each device prefers its ABI-specific APK, then universal. Debug APKs, split APKs,
AABs, other ABIs, and desktop assets are not update candidates.

Include GitHub's `sha256:` asset digest or an entry for the exact filename in
`SHA256SUMS.txt`. The updater refreshes release metadata before downloading and
blocks stale offers, missing verification metadata, checksum mismatches, empty
downloads, and downloads over 1 GiB. Downloads stay in private cache storage;
the FileProvider exposes only the update subdirectory to Android's installer.

When enabling Android release CI later:

1. Build release APKs with the existing application ID and the same persistent
   signing key across stable and nightly builds. Keep signing secrets in CI.
   Debug builds signed with a development key cannot receive production updates.
   The current native preflight requires the same signer set; signing-key rotation
   requires a separate migration before publishing rotated APKs.
2. Assign a monotonically increasing Android `versionCode` across both channels.
   Use one persisted build counter, not a counter that resets per workflow or
   semantic version. Store-safe values must stay at or below 2,100,000,000.
   Verify the packaged `versionName` matches the filename and Rust version.
3. Verify the APK and bundled native runtimes on a device, rename it according
   to the contract, and upload the APK and checksum with the desktop assets.
4. Test the installed older release updating to the new release, source-permission
   denial and approval, cancellation/retry, offline recovery, and preserved data.

Users tap **Update Verenu** inside the app. If Android has not allowed Verenu
to install updates, Verenu opens the per-app source settings and asks the user
to return and retry. After download, native preflight checks the package ID,
signer set, and nondecreasing version code, then opens Android's installer.
Android owns signature verification and final installation approval. Opening
the installer does not mark the update installed; cancellation allows retry.
The staged APK remains available until the next attempt or Android clears cache.
No GitHub page or manual file handling is needed.

This path is for directly distributed Android APKs. A future Play Store build
must use Play-managed updates and omit the self-install permission. iOS would
need its own App Store/TestFlight update path.

This document is the practical release checklist for Verenu. For release note wording, use [`../Agent-Skills/Release_Description_Writing.md`](../Agent-Skills/Release_Description_Writing.md).

## Branch Flow

1. Merge reviewed pull requests directly into `master`.
2. Keep `master` green with the required CI checks.
3. The scheduled morning nightly release inspects `master` and publishes the
   next prerelease when enough changes have accumulated.
4. Run the manual installer workflow from `master` when an on-demand
   production build is needed; it builds the ref selected at dispatch.

## Version Bump

Update these three files together:

- [`../package.json`](../package.json)
- [`../src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)
- [`../src-tauri/Cargo.toml`](../src-tauri/Cargo.toml)

The version strings must match exactly. The frontend reads the version dynamically through Tauri, so do not hardcode release versions in Svelte files.

## Pre-Release Checks

Run the practical local gate:

```bash
npm install
npm run check
npm run lint
npm test
npm run test:rust
```

For UI-affecting work, also run the relevant Playwright smoke or integration tests. For provider, permission, injection, updater, hotkey, or installer changes, run the matching platform checks. Do not pretend a desktop integration was tested if it was only type-checked.

## Build Installers

The manual GitHub Actions workflow [`../.github/workflows/build-installers.yml`](../.github/workflows/build-installers.yml) builds the ref selected at dispatch:

- Windows NSIS installer
- Windows MSI installer
- macOS Apple Silicon DMG
- macOS Intel DMG
- Linux x86_64 AppImage (built on Ubuntu 22.04 for conservative glibc compatibility)
- Arch Linux x86_64 pacman package (beta, built from the same AppImage for Omarchy/Hyprland)

Linux nightly releases include both the AppImage and the Arch package. Arch package
versions replace the app version's prerelease hyphen with an underscore because
`pkgver` forbids hyphens. The updater converts that name back for comparison.
Both files must be present before publication and included in `SHA256SUMS.txt`.

The Linux updater detects ownership of the running executable and `APPIMAGE`.
The official `verenu` pacman package updates through `pkexec pacman -U` after a
verified download and SQLite backup. Polkit and a running desktop authentication
agent are required. Pacman's local signature policy, locks, and dependency checks
remain active. Verenu never runs a system update or removes a pacman lock.
AUR packages with other names remain under their package manager's control.

Portable AppImages update in their current folder with an atomic replacement and
retain `<image>.previous` for manual rollback. The folder must be writable and
support hard links. Restart Verenu after either installation succeeds. Download,
checksum, backup, or authorization failures keep the running app open and allow
another attempt. Releases without a GitHub SHA256 digest or a matching checksum
entry cannot be installed automatically. Other Linux installs offer a download.

The release folder should contain:

- `Verenu_<version>_x64-setup.exe`
- `Verenu_<version>_x64_en-US.msi`
- `Verenu_<version>_Apple_Silicon.dmg`
- `Verenu_<version>_Intel.dmg`
- `Verenu_<version>_x86_64.AppImage`
- `verenu-<version>-1-x86_64.pkg.tar.zst` (beta)
- `SHA256SUMS.txt`

Installer artifacts are committed under [`../installers/`](../installers/) with a versioned subfolder and also attached to the GitHub Release.

## Hashes

After placing the installers in the correct version folder under [`../installers/`](../installers/), regenerate hashes from that folder:

```bash
shasum -a 256 *.exe *.msi *.dmg *.AppImage *.pkg.tar.zst > SHA256SUMS.txt
```

On Windows PowerShell, generate the same file with:

```powershell
Get-ChildItem *.exe,*.msi,*.dmg,*.AppImage,*.pkg.tar.zst | ForEach-Object {
  $hash = (Get-FileHash $_ -Algorithm SHA256).Hash.ToLower()
  "{0} *{1}" -f $hash, $_.Name
} | Set-Content SHA256SUMS.txt
```

On Windows PowerShell, verify a file manually with:

```powershell
Get-FileHash .\Verenu_<version>_x64-setup.exe -Algorithm SHA256
```

The hashes in `SHA256SUMS.txt`, the committed files, GitHub Release assets, and release notes must agree. Mark the Arch package as a beta in release notes until its Omarchy/Hyprland install path has received user testing.

## Release Notes

Release notes should include:

- A plain title: `Verenu <version> - <tagline>`
- A short app blurb.
- Version-specific changes.
- Evergreen features.
- Getting started steps.
- Default shortcuts.
- Lightweight and local positioning.
- VirusTotal links for all four installer files.

Do not include API keys, local paths, private screenshots, private user text, or maintainer-only secrets in release notes.

## Post-Release Checks

- Download each uploaded installer from GitHub Releases.
- Verify each hash against `SHA256SUMS.txt`.
- Open the app on Windows and macOS when possible.
- Confirm first-run setup can save an API key without exposing it.
- Confirm the release docs and installer folder match the shipped version.

## Related Docs

<p align="center">
  <a href="CHANGELOG.md"><img alt="Changelog" src="https://img.shields.io/badge/Project-Changelog-c44632"></a>
  <a href="TESTING.md"><img alt="Testing" src="https://img.shields.io/badge/Testing-Guide-5b554a"></a>
  <a href="macos-code-signing.md"><img alt="macOS Signing" src="https://img.shields.io/badge/macOS-Signing-7e7266"></a>
  <a href="README.md"><img alt="Docs Index" src="https://img.shields.io/badge/Docs-Index-2b2422"></a>
</p>
