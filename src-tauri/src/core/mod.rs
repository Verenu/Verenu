pub mod browser_probe;
pub mod context;
pub mod context_probe;
pub mod hotkey;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod hyprland;
pub mod injection;
pub mod text_context;
pub mod window_context;
pub mod window_geometry;

#[cfg(target_os = "macos")]
mod context_probe_macos;
