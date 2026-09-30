# Arch Linux beta package

The Arch package is in beta. It targets Omarchy Quattro on Hyprland and uses
the same tested Linux AppImage build inside a native pacman package. The
package adds an application launcher and declares the desktop libraries and
portals Verenu needs. It does not edit Omarchy's system files.

## Install from a release

Download `verenu-<version>-1-x86_64.pkg.tar.zst` from the GitHub release and
install it with:

```bash
sudo pacman -U ./verenu-<version>-1-x86_64.pkg.tar.zst
```

The release's `SHA256SUMS.txt` includes the package hash. Verify it before
installing:

```bash
sha256sum -c SHA256SUMS.txt
```

Keep PipeWire and the XDG desktop portal services running. A Secret Service
provider such as GNOME Keyring must be installed and unlocked for API key
storage. On Omarchy Quattro, Verenu registers its shortcuts in the user's
Hyprland bindings and follows the active Omarchy theme when **Appearance →
Omarchy** is selected. See [Linux installation](INSTALL.md#linux-omarchy-quattro--arch--hyprland)
for shortcut behavior and Hyprland notes.

## Build the package

The build consumes the matching Linux AppImage from the release. Download it
beside `PKGBUILD`, then build on an Arch Linux system:

```bash
cd packaging/arch
VERENU_PKGVER=<version> makepkg --cleanbuild --syncdeps --noconfirm
```

The AppImage must be named `Verenu_<version>_x86_64.AppImage`. The installer
workflow builds the package as a downloadable beta artifact. Attach it to a
release after Omarchy validation.
