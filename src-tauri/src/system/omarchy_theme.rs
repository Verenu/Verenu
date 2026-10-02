//! Omarchy theme integration. Omarchy writes the active palette to
//! `~/.local/state/omarchy/current/theme/colors.toml` (older releases used
//! `~/.config/omarchy/current/theme`). The "Omarchy" appearance mode maps that
//! palette onto Verenu's surface tokens and follows `omarchy theme set` live.

use serde::Serialize;
use std::collections::BTreeMap;

#[cfg(target_os = "linux")]
pub const THEME_CHANGED_EVENT: &str = "verenu:omarchy-theme-changed";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OmarchyTheme {
    pub name: String,
    /// "dark" or "light".
    pub mode: String,
    /// Validated `#rrggbb` colors keyed by their `colors.toml` names
    /// (background, foreground, accent, dark_background, red, ...).
    pub colors: BTreeMap<String, String>,
}

pub(crate) fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn luminance(hex: &str) -> f64 {
    if !is_hex_color(hex) {
        return 0.0;
    }
    let channel = |offset: usize| {
        let value = f64::from(u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap_or(0)) / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}

const CUSTOM_REQUIRED: [&str; 2] = ["background", "foreground"];
const CUSTOM_OPTIONAL: [&str; 2] = ["sidebar", "surface"];

/// The Custom appearance palette: required `background` and `foreground`
/// hex colors, optional `sidebar` and `surface` hex colors (or null), and
/// nothing else.
pub(crate) fn is_custom_theme(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let hex = |key: &str| object.get(key).and_then(|v| v.as_str()).is_some_and(is_hex_color);
    CUSTOM_REQUIRED.iter().all(|key| hex(key))
        && object.iter().all(|(key, value)| {
            CUSTOM_REQUIRED.contains(&key.as_str())
                || (CUSTOM_OPTIONAL.contains(&key.as_str())
                    && (value.is_null() || value.as_str().is_some_and(is_hex_color)))
        })
}

/// Whether the saved Custom palette is dark, judged by its background.
pub(crate) fn custom_is_dark(value: &serde_json::Value) -> Option<bool> {
    let background = value.get("background")?.as_str().filter(|v| is_hex_color(v))?;
    Some(luminance(background) <= 0.4)
}

/// Parses the flat `key = "value"` subset of TOML that `colors.toml` uses.
/// Returns `None` unless it defines a usable background and foreground.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_colors(contents: &str, name: &str, light_marker: bool) -> Option<OmarchyTheme> {
    let mut colors = BTreeMap::new();
    let mut mode = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.starts_with('#') || key.starts_with('[') {
            continue;
        }
        // `#` both starts comments and hex colors, so read a quoted value up
        // to its closing quote and a bare value up to whitespace.
        let value = value.trim();
        let value = match value.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(""),
            None => value.split_whitespace().next().unwrap_or(""),
        };
        if key == "mode" {
            mode = Some(value.to_ascii_lowercase());
        } else if is_hex_color(value) {
            colors.insert(key.to_string(), value.to_ascii_lowercase());
        }
    }
    let background = colors.get("background")?.clone();
    colors.get("foreground")?;
    let mode = match mode.as_deref() {
        Some("light") => "light",
        Some("dark") => "dark",
        _ if light_marker => "light",
        _ if luminance(&background) > 0.4 => "light",
        _ => "dark",
    };
    Some(OmarchyTheme {
        name: name.to_string(),
        mode: mode.to_string(),
        colors,
    })
}

#[cfg(target_os = "linux")]
fn theme_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    [
        state.join("omarchy/current/theme"),
        home.join(".config/omarchy/current/theme"),
    ]
    .into_iter()
    .find(|dir| dir.join("colors.toml").is_file())
}

#[cfg(target_os = "linux")]
pub fn current() -> Option<OmarchyTheme> {
    let dir = theme_dir()?;
    let contents = std::fs::read_to_string(dir.join("colors.toml")).ok()?;
    let name = dir
        .parent()
        .and_then(|current| std::fs::read_to_string(current.join("theme.name")).ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "omarchy".to_string());
    parse_colors(&contents, &name, dir.join("light.mode").exists())
}

#[cfg(not(target_os = "linux"))]
pub fn current() -> Option<OmarchyTheme> {
    None
}

/// Polls the theme files (cheap metadata reads) so `omarchy theme set`
/// updates every Verenu window without a restart.
#[cfg(target_os = "linux")]
fn start_watcher(app: &tauri::AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::Emitter;
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("omarchy_theme_watch".into())
        .spawn(move || {
            let mut last = current();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let next = current();
                if next != last {
                    last = next.clone();
                    let _ = app.emit(THEME_CHANGED_EVENT, next);
                    if matches!(
                        crate::app_tray::appearance_mode(&app).as_deref(),
                        None | Some("system" | "omarchy")
                    ) {
                        crate::apply_runtime_icons(&app, None);
                    }
                }
            }
        })
        .ok();
}

/// Whether the Omarchy palette is dark. Used by native surfaces (tray and
/// window icons) that cannot read CSS.
pub fn is_dark() -> Option<bool> {
    current().map(|theme| theme.mode == "dark")
}

#[tauri::command]
pub fn get_omarchy_theme(app: tauri::AppHandle) -> Option<OmarchyTheme> {
    #[cfg(target_os = "linux")]
    start_watcher(&app);
    let _ = app;
    current()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOLITUDE: &str = r##"
mode = "dark"
accent = "#798186"  # accent
background = "#101315"
dark_background = "#0c0e10"
foreground = "#cacccc"
hyprland_active_border = "rgba(798186ee) rgba(caccccee)"
bright_red = "#DE6145"
"##;

    #[test]
    fn parses_hex_colors_and_mode() {
        let theme = parse_colors(SOLITUDE, "solitude", false).expect("theme");
        assert_eq!(theme.mode, "dark");
        assert_eq!(theme.colors["accent"], "#798186");
        assert_eq!(theme.colors["bright_red"], "#de6145");
        assert!(!theme.colors.contains_key("hyprland_active_border"));
    }

    #[test]
    fn infers_light_mode_without_explicit_mode() {
        let light = "background = \"#f2efe9\"\nforeground = \"#2a2a2a\"\n";
        assert_eq!(parse_colors(light, "latte", false).unwrap().mode, "light");
        let dark = "background = \"#1e1e2e\"\nforeground = \"#cdd6f4\"\n";
        assert_eq!(parse_colors(dark, "mocha", false).unwrap().mode, "dark");
        assert_eq!(parse_colors(dark, "mocha", true).unwrap().mode, "light");
    }

    #[test]
    fn custom_theme_requires_hex_background_and_foreground() {
        let ok = serde_json::json!({"background": "#101315", "foreground": "#CACCCC"});
        assert!(is_custom_theme(&ok));
        let full = serde_json::json!({
            "background": "#101315", "foreground": "#cacccc",
            "sidebar": "#0c0e10", "surface": null
        });
        assert!(is_custom_theme(&full));
        assert!(!is_custom_theme(&serde_json::json!({"background": "#101315"})));
        assert!(!is_custom_theme(&serde_json::json!({"background": "red", "foreground": "#fff000"})));
        assert!(!is_custom_theme(&serde_json::json!({
            "background": "#101315", "foreground": "#cacccc", "accent": "#ffffff"
        })));
        assert!(!is_custom_theme(&serde_json::json!("#101315")));
    }

    #[test]
    fn custom_theme_darkness_follows_background() {
        let dark = serde_json::json!({"background": "#1e1e2e", "foreground": "#cdd6f4"});
        let light = serde_json::json!({"background": "#f2efe9", "foreground": "#2a2a2a"});
        assert_eq!(custom_is_dark(&dark), Some(true));
        assert_eq!(custom_is_dark(&light), Some(false));
        assert_eq!(custom_is_dark(&serde_json::json!({})), None);
    }

    #[test]
    fn rejects_themes_without_core_colors() {
        assert!(parse_colors("accent = \"#ffffff\"\n", "x", false).is_none());
    }
}
