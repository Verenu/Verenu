# Release Process

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
