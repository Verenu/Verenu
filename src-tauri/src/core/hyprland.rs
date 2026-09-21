//! Small, bounded Hyprland IPC seam.
//!
//! Hyprland deliberately exposes window identity through its authenticated
//! compositor IPC. Keeping the one `hyprctl -j` fallback here prevents string
//! addresses from leaking into the dictation pipeline. It is used only at
//! recording start/retry and insertion, never while audio callbacks run.

#[cfg(target_os = "linux")]
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::fs;

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ActiveWindow {
    pub address: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default, rename = "class")]
    pub class_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub at: [i32; 2],
    #[serde(default)]
    pub size: [i32; 2],
    #[serde(default)]
    pub workspace: Workspace,
    #[serde(default)]
    pub monitor: i32,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Workspace {
    #[serde(default)]
    pub id: i32,
    #[serde(default)]
    pub name: String,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LogicalMonitor {
    pub work_x: f64,
    pub work_y: f64,
    pub work_width: f64,
    pub work_height: f64,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Deserialize)]
struct HyprMonitor {
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    #[serde(default)]
    reserved: [i32; 4],
    #[serde(default)]
    focused: bool,
}

#[cfg(target_os = "linux")]
impl HyprMonitor {
    fn logical_work_area(&self) -> LogicalMonitor {
        // Hyprland reports width/height as mode pixels, while x/y and window
        // geometry use compositor layout coordinates. Convert the mode size
        // before combining it with the layout origin. Dividing the final
        // absolute coordinate instead breaks vertically or horizontally offset
        // scaled monitors and was placing the pill below the visible desktop.
        let scale = self.scale.max(0.1);
        let logical_width = f64::from(self.width) / scale;
        let logical_height = f64::from(self.height) / scale;
        let [left, top, right, bottom] = self.reserved;
        LogicalMonitor {
            work_x: f64::from(self.x + left),
            work_y: f64::from(self.y + top),
            work_width: (logical_width - f64::from(left + right)).max(1.0),
            work_height: (logical_height - f64::from(top + bottom)).max(1.0),
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        let scale = self.scale.max(0.1);
        let right = f64::from(self.x) + f64::from(self.width) / scale;
        let bottom = f64::from(self.y) + f64::from(self.height) / scale;
        x >= f64::from(self.x) && x < right && y >= f64::from(self.y) && y < bottom
    }
}

#[cfg(target_os = "linux")]
fn hyprctl_json(args: &[&str]) -> Option<serde_json::Value> {
    // The command is synchronous but used on a non-audio path. Do not include
    // output in diagnostics: titles/classes can be private.
    let output = std::process::Command::new("hyprctl")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    serde_json::from_slice(&output.stdout).ok()
}

#[cfg(target_os = "linux")]
fn monitors() -> Option<Vec<HyprMonitor>> {
    serde_json::from_value(hyprctl_json(&["-j", "monitors"])?).ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn logical_monitor_for_point(x: f64, y: f64) -> Option<LogicalMonitor> {
    let monitors = monitors()?;
    monitors
        .iter()
        .find(|monitor| monitor.contains(x, y))
        .or_else(|| monitors.iter().find(|monitor| monitor.focused))
        .or_else(|| monitors.first())
        .map(HyprMonitor::logical_work_area)
}

#[cfg(target_os = "linux")]
pub(crate) fn logical_monitor_for_pill() -> Option<LogicalMonitor> {
    let monitor_id = pill_window()?.monitor;
    monitors()?
        .into_iter()
        .find(|monitor| monitor.id == monitor_id)
        .map(|monitor| monitor.logical_work_area())
}

#[cfg(target_os = "linux")]
pub(crate) fn session_available() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        && hyprctl_json(&["-j", "version"]).is_some()
}

#[cfg(target_os = "linux")]
pub(crate) fn active_window() -> Option<ActiveWindow> {
    serde_json::from_value(hyprctl_json(&["-j", "activewindow"])?).ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn window_by_address(address: &str) -> Option<ActiveWindow> {
    let windows = hyprctl_json(&["-j", "clients"])?;
    windows
        .as_array()?
        .iter()
        .find(|window| window.get("address").and_then(|v| v.as_str()) == Some(address))
        .cloned()
        .and_then(|window| serde_json::from_value(window).ok())
}

#[cfg(target_os = "linux")]
pub(crate) fn focus(address: &str) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression = format!("hl.dsp.focus({{ window = '{selector}' }})");
    let status = std::process::Command::new("hyprctl")
        .args(["dispatch", &expression])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|err| format!("Hyprland IPC is unavailable: {err}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Hyprland could not focus the original target window".to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn dispatch_paste_for_target(class_name: &str, tags: &[String]) -> Result<(), String> {
    let class = class_name.to_ascii_lowercase();
    let terminal = [
        "foot",
        "kitty",
        "alacritty",
        "ghostty",
        "wezterm",
        "konsole",
        "gnome-terminal",
        "org.gnome.console",
        "org.omarchy.",
        "tui.",
        "xterm",
    ]
    .iter()
    .any(|name| class.contains(name))
        || tags
            .iter()
            .any(|tag| tag.trim_end_matches('*').eq_ignore_ascii_case("terminal"));
    let (mods, key) = if terminal {
        ("SHIFT", "INSERT")
    } else {
        ("CTRL", "V")
    };
    log::debug!(
        "linux injection: selected {} paste gesture",
        if terminal { "terminal" } else { "standard" }
    );
    for state in ["down", "up"] {
        let expression = format!(
            "hl.dsp.send_key_state({{ mods = '{mods}', key = '{key}', state = '{state}' }})"
        );
        let status = std::process::Command::new("hyprctl")
            .args(["dispatch", &expression])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("Hyprland paste IPC unavailable: {e}"))?;
        if !status.success() {
            return Err("Hyprland could not dispatch paste to the original target".to_string());
        }
        if state == "down" {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn move_window(address: &str, x: i32, y: i32) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression = format!(
        "hl.dsp.window.move({{ x = {x}, y = {y}, relative = false, window = '{selector}' }})"
    );
    let status = std::process::Command::new("hyprctl")
        .args(["dispatch", &expression])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Hyprland move IPC unavailable: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Hyprland could not position the dictation pill".to_string())
}

/// Raises a floating window without focusing it. Wayland does not guarantee
/// that Tauri's always-on-top hint reaches the compositor, so the pill needs
/// an explicit Hyprland z-order update each time it is revealed.
#[cfg(target_os = "linux")]
pub(crate) fn raise_window(address: &str) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression =
        format!("hl.dsp.window.alter_zorder({{ mode = 'top', window = '{selector}' }})");
    let status = std::process::Command::new("hyprctl")
        .args(["dispatch", &expression])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Hyprland z-order IPC unavailable: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Hyprland could not raise the dictation pill".to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn pill_window() -> Option<ActiveWindow> {
    let clients = hyprctl_json(&["-j", "clients"])?;
    clients
        .as_array()?
        .iter()
        .find(|w| {
            w.get("class").and_then(|v| v.as_str()) == Some("verenu")
                && w.get("title").and_then(|v| v.as_str()) == Some("Verenu Dictation Pill")
        })
        .cloned()
        .and_then(|w| serde_json::from_value(w).ok())
}

/// `pill_window()` right after `show()` races Hyprland: the Wayland client is
/// mapped asynchronously, so `hyprctl clients` can still miss it for a beat.
/// A miss here used to fall through silently (no retry, `.ok()`-swallowed
/// error), leaving the pill wherever Hyprland's default floating placement
/// put it — which reads as "spawns in the middle of the screen" and made the
/// move look random. Poll briefly for the client to appear before giving up.
#[cfg(target_os = "linux")]
pub(crate) fn pill_window_after_show() -> Option<ActiveWindow> {
    for attempt in 0..10 {
        if let Some(window) = pill_window() {
            return Some(window);
        }
        if attempt < 9 {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    None
}

/// Install the portal action in the user's Lua bindings. Omarchy loads this
/// file after its defaults, so the marked block is the only text Verenu owns.
#[cfg(target_os = "linux")]
pub(crate) fn ensure_global_shortcut_binding(trigger: &str, portal_id: &str) -> Result<(), String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| "XDG config directory is unavailable".to_string())?;
    let path = config.join("hypr/bindings.lua");
    let current =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let start = "-- >>> Verenu managed global shortcut (do not edit) <<<";
    let end = "-- <<< End Verenu managed global shortcut >>>";
    let block = global_shortcut_binding_block(start, end, trigger, portal_id);
    let next = if let (Some(a), Some(b)) = (current.find(start), current.find(end)) {
        let b = b + end.len();
        format!("{}{}{}", &current[..a], block, &current[b..])
    } else {
        format!("{}\n\n{}\n", current.trim_end(), block)
    };
    if next != current {
        let tmp = path.with_extension("lua.verenu.tmp");
        fs::write(&tmp, next).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))?;
    }

    // The portal action is session-scoped. Reload even when the generated
    // text is unchanged so Hyprland resolves hl.dsp.global() against the new
    // portal session after an app restart instead of retaining a dead action.
    let status = std::process::Command::new("hyprctl")
        .arg("reload")
        .status()
        .map_err(|e| format!("Hyprland reload failed: {e}"))?;
    if !status.success() {
        return Err("Hyprland rejected the updated Verenu binding".to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn global_shortcut_binding_block(start: &str, end: &str, trigger: &str, portal_id: &str) -> String {
    // Lua dispatchers are registered with Hyprland as generic `__lua`
    // handlers. Unlike the legacy native `global` dispatcher, those handlers
    // are not called automatically for both key-down and key-up. Bind the same
    // portal action for each edge so XDG GlobalShortcuts emits Activated and
    // Deactivated and hold-to-dictate stops when the chord is released.
    format!(
        "{start}\n-- Portal action: {portal_id}\nlocal verenu_dictate = hl.dsp.global(\"{portal_id}\")\nhl.bind(\"{trigger}\", verenu_dictate, {{ description = \"Verenu dictation\" }})\nhl.bind(\"{trigger}\", verenu_dictate, {{ description = \"Verenu dictation release\", release = true }})\n{end}"
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn active_window() -> Option<()> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{global_shortcut_binding_block, HyprMonitor, LogicalMonitor};

    #[test]
    fn scaled_monitor_converts_mode_pixels_before_adding_layout_origin() {
        let monitor = HyprMonitor {
            id: 1,
            x: 0,
            y: 1080,
            width: 2560,
            height: 1440,
            scale: 1.25,
            reserved: [0, 26, 0, 0],
            focused: true,
        };

        assert_eq!(
            monitor.logical_work_area(),
            LogicalMonitor {
                work_x: 0.0,
                work_y: 1106.0,
                work_width: 2048.0,
                work_height: 1126.0,
            }
        );
        assert!(monitor.contains(1024.0, 2000.0));
        assert!(!monitor.contains(1024.0, 2300.0));
    }

    #[test]
    fn global_shortcut_binding_forwards_press_and_release() {
        let block = global_shortcut_binding_block("START", "END", "CTRL + SPACE", "app:dictate");

        assert!(block.contains("local verenu_dictate = hl.dsp.global(\"app:dictate\")"));
        assert!(block.contains(
            "hl.bind(\"CTRL + SPACE\", verenu_dictate, { description = \"Verenu dictation\" })"
        ));
        assert!(block.contains(
            "hl.bind(\"CTRL + SPACE\", verenu_dictate, { description = \"Verenu dictation release\", release = true })"
        ));
    }
}
