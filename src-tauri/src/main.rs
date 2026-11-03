#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
fn use_wayland_for_appimage(
    appdir: Option<&std::ffi::OsStr>,
    schemas: Option<&std::ffi::OsStr>,
    desktop: &str,
    wayland: bool,
    executable: Option<&std::path::Path>,
) -> bool {
    let bundled_schemas = appdir.zip(schemas).is_some_and(|(appdir, schemas)| {
        let bundled = std::path::Path::new(appdir).join("usr/share/glib-2.0/schemas");
        executable.is_some_and(|exe| exe.starts_with(std::path::Path::new(appdir).join("usr/bin")))
            && std::env::split_paths(schemas).any(|path| path == bundled)
    });
    let hyprland = desktop
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("hyprland"));
    bundled_schemas && hyprland && wayland
}

fn main() {
    #[cfg(target_os = "linux")]
    if use_wayland_for_appimage(
        std::env::var_os("APPDIR").as_deref(),
        std::env::var_os("GSETTINGS_SCHEMA_DIR").as_deref(),
        &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::current_exe().ok().as_deref(),
    ) {
        std::env::set_var("GDK_BACKEND", "wayland");
        std::env::remove_var("GSETTINGS_SCHEMA_DIR");
    }
    #[cfg(target_os = "linux")]
    if verenu::run_internal_output_guardian() {
        return;
    }
    verenu::run();
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::use_wayland_for_appimage;
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn wayland_override_is_limited_to_hyprland_appimages() {
        let appdir = Some(OsStr::new("/tmp/app"));
        let bundled = Some(OsStr::new(
            "/tmp/app/usr/share/glib-2.0/schemas/:/tmp/app//usr/share/glib-2.0/schemas",
        ));
        let executable = Some(Path::new("/tmp/app/usr/bin/Verenu"));
        assert!(use_wayland_for_appimage(
            appdir, bundled, "Hyprland", true, executable
        ));
        assert!(!use_wayland_for_appimage(
            appdir, bundled, "Hyprland", false, executable
        ));
        assert!(!use_wayland_for_appimage(
            appdir, bundled, "GNOME", true, executable
        ));
        assert!(!use_wayland_for_appimage(
            None, bundled, "Hyprland", true, executable
        ));
        assert!(!use_wayland_for_appimage(
            appdir,
            bundled,
            "Hyprland",
            true,
            Some(Path::new("/tmp/other/Verenu"))
        ));
        assert!(!use_wayland_for_appimage(
            appdir,
            Some(OsStr::new("/usr/share/glib-2.0/schemas")),
            "Hyprland",
            true,
            executable
        ));
    }
}
