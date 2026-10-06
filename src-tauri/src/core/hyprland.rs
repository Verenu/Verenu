//! Small, bounded Hyprland IPC seam.
//!
//! Hyprland deliberately exposes window identity through its authenticated
//! compositor IPC. Keeping the one `hyprctl -j` fallback here prevents string
//! addresses from leaking into the dictation pipeline. It is used only at
//! recording start/retry and insertion, never while audio callbacks run.

#[cfg(target_os = "linux")]
use serde::de::DeserializeOwned;
#[cfg(target_os = "linux")]
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::fs;

#[cfg(target_os = "linux")]
const WINDOW_APP_ID: &str = "com.verenu.app";

/// GTK's Wayland window class defaults to GLib's program name, even when the
/// GtkApplication has an explicit app ID. Set it before GTK initializes.
#[cfg(target_os = "linux")]
pub(crate) fn initialize_app_identity() {
    gtk::glib::set_prgname(Some(WINDOW_APP_ID));
}

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
fn hyprctl_json<T: DeserializeOwned>(args: &[&str]) -> Option<T> {
    if let Some(response) = socket_request(&format!("j/{}", args[1..].join(" "))) {
        return serde_json::from_slice(&response.ok()?).ok();
    }
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

/// Use the same command socket and five-second timeout as hyprctl. Only fall
/// back before sending anything: retrying a dispatch after a read failure can
/// repeat a visible window operation.
#[cfg(target_os = "linux")]
pub(crate) fn socket_request(command: &str) -> Option<std::io::Result<Vec<u8>>> {
    let path = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
        .join("hypr")
        .join(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?)
        .join(".socket.sock");
    let stream = std::os::unix::net::UnixStream::connect(path).ok()?;
    Some(socket_exchange(stream, command))
}

#[cfg(target_os = "linux")]
fn socket_exchange(mut stream: std::os::unix::net::UnixStream, command: &str) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Write};
    let timeout = Some(std::time::Duration::from_secs(5));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(command.as_bytes())?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(response)
}

#[cfg(target_os = "linux")]
fn monitors() -> Option<Vec<HyprMonitor>> {
    hyprctl_json(&["-j", "monitors"])
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
pub(crate) fn session_available() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        && hyprctl_json::<serde_json::Value>(&["-j", "version"]).is_some()
}

#[cfg(target_os = "linux")]
pub(crate) fn active_window() -> Option<ActiveWindow> {
    hyprctl_json(&["-j", "activewindow"])
}

#[cfg(target_os = "linux")]
pub(crate) fn window_by_address(address: &str) -> Option<ActiveWindow> {
    let windows: Vec<serde_json::Value> = hyprctl_json(&["-j", "clients"])?;
    windows
        .into_iter()
        .find(|window| window.get("address").and_then(|v| v.as_str()) == Some(address))
        .and_then(|window| serde_json::from_value(window).ok())
}

/// The client owned by `pid`, preferring the most recently focused one. Used
/// to resolve a captured target's app after focus has moved elsewhere.
#[cfg(target_os = "linux")]
pub(crate) fn window_by_pid(pid: u32) -> Option<ActiveWindow> {
    let windows: Vec<serde_json::Value> = hyprctl_json(&["-j", "clients"])?;
    windows
        .into_iter()
        .filter(|window| window.get("pid").and_then(|v| v.as_u64()) == Some(u64::from(pid)))
        .min_by_key(|window| window.get("focusHistoryID").and_then(|v| v.as_i64()).unwrap_or(i64::MAX))
        .and_then(|window| serde_json::from_value(window).ok())
}

#[cfg(target_os = "linux")]
fn dispatch(expression: &str, unavailable: &str, failed: &str) -> Result<(), String> {
    if let Some(response) = socket_request(&format!("/dispatch {expression}")) {
        return response
            .ok()
            .filter(|reply| !reply.starts_with(b"error:"))
            .map(|_| ())
            .ok_or_else(|| failed.to_string());
    }
    let status = std::process::Command::new("hyprctl")
        .args(["dispatch", expression])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|err| format!("{unavailable}: {err}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| failed.to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn focus(address: &str) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression = format!("hl.dsp.focus({{ window = '{selector}' }})");
    dispatch(
        &expression,
        "Hyprland IPC is unavailable",
        "Hyprland could not focus the original target window",
    )
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
        dispatch(
            &expression,
            "Hyprland paste IPC unavailable",
            "Hyprland could not dispatch paste to the original target",
        )?;
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
    dispatch(
        &expression,
        "Hyprland move IPC unavailable",
        "Hyprland could not position the dictation pill",
    )
}

/// Raises a floating window without focusing it. Wayland does not guarantee
/// that Tauri's always-on-top hint reaches the compositor, so the pill needs
/// an explicit Hyprland z-order update each time it is revealed.
/// Floating clients cannot resize themselves reliably under Hyprland (the
/// non-resizable GTK pill stayed at Hyprland's 200x200 default), so size is
/// set by the compositor, like position. Units are layout (logical) pixels.
#[cfg(target_os = "linux")]
pub(crate) fn resize_window(address: &str, width: i32, height: i32) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression = format!(
        "hl.dsp.window.resize({{ x = {width}, y = {height}, relative = false, window = '{selector}' }})"
    );
    dispatch(
        &expression,
        "Hyprland resize IPC unavailable",
        "Hyprland could not size the dictation pill",
    )
}

/// Lets the pointer reach a window that a user rule marked `no_focus`.
///
/// Hyprland excludes `no_focus` windows from pointer hit-testing altogether, so
/// the pill's Cancel/Confirm/Dismiss/Retry/Copy buttons never saw a click when
/// a rule such as the one long suggested in the install notes was present. The
/// rule is applied at map time, which is what keeps the pill from taking
/// keyboard focus away from the dictation target, so clearing the runtime
/// property afterwards keeps that benefit and only restores the pointer.
/// Clicking the pill may focus it; text injection re-focuses the original
/// target before pasting.
#[cfg(target_os = "linux")]
pub(crate) fn set_pointer_input(address: &str, interactive: bool) -> Result<(), String> {
    let selector = format!("address:{address}");
    let no_focus = i32::from(!interactive);
    let expression = format!(
        "hl.dsp.window.set_prop({{ window = '{selector}', prop = 'no_focus', value = '{no_focus}' }})"
    );
    dispatch(
        &expression,
        "Hyprland property IPC unavailable",
        "Hyprland could not enable pointer input for the dictation pill",
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn raise_window(address: &str) -> Result<(), String> {
    let selector = format!("address:{address}");
    let expression =
        format!("hl.dsp.window.alter_zorder({{ mode = 'top', window = '{selector}' }})");
    dispatch(
        &expression,
        "Hyprland z-order IPC unavailable",
        "Hyprland could not raise the dictation pill",
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn pill_window() -> Option<ActiveWindow> {
    let clients: serde_json::Value = hyprctl_json(&["-j", "clients"])?;
    owned_pill_window(&clients, std::process::id())
}

#[cfg(target_os = "linux")]
fn owned_pill_window(clients: &serde_json::Value, pid: u32) -> Option<ActiveWindow> {
    clients
        .as_array()?
        .iter()
        .find(|w| {
            // PID and our non-localized title identify this process's pill.
            // Executable/AppImage renaming must never break placement.
            w.get("title").and_then(|v| v.as_str()) == Some("Verenu Dictation Pill")
                && w.get("pid").and_then(|v| v.as_u64()) == Some(u64::from(pid))
        })
        .cloned()
        .and_then(|w| serde_json::from_value(w).ok())
}

#[cfg(target_os = "linux")]
const WINDOW_RULE_MARKERS: (&str, &str) = (
    "-- >>> Verenu window rules (do not edit) <<<",
    "-- <<< End Verenu window rules >>>",
);

/// Installed before either window is mapped, including on a fresh profile.
/// Use Hyprland's own API rather than a version-specific Omarchy helper.
/// Tauri's GTK app ID comes from the bundle identifier, never the filename.
#[cfg(target_os = "linux")]
fn window_rule_block() -> String {
    let (start, end) = WINDOW_RULE_MARKERS;
    format!(r#"{start}
hl.window_rule({{ name = "verenu-main", match = {{ class = "^(com\\.verenu\\.app|[Vv]erenu)$", title = "^Verenu$" }}, float = true, center = true, min_size = {{ 1100, 700 }} }})
hl.window_rule({{ name = "verenu-pill", match = {{ class = "^(com\\.verenu\\.app|[Vv]erenu)$", title = "^Verenu Dictation Pill$" }}, float = true, pin = true, border_size = 0, no_initial_focus = true }})
{end}"#)
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_window_rules() -> Result<(), String> {
    if !session_available() {
        return Ok(());
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config")))
        .ok_or_else(|| "XDG config directory is unavailable".to_string())?;
    let path = config.join("hypr/bindings.lua");
    let current = fs::read_to_string(&path)
        .map_err(|e| format!("cannot read Hyprland user bindings: {e}"))?;
    let (start, end) = WINDOW_RULE_MARKERS;
    let next = replace_managed_block(&current, start, end, &window_rule_block())?;
    if next != current {
        let tmp = path.with_extension(format!("lua.verenu-windows-{}.tmp", std::process::id()));
        fs::write(&tmp, next).map_err(|e| format!("cannot write Verenu window rules: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot install Verenu window rules: {e}"))?;
        let output = std::process::Command::new("hyprctl").arg("reload").output()
            .map_err(|e| format!("Hyprland window rule reload failed: {e}"))?;
        if !output.status.success() {
            return Err("Hyprland rejected Verenu window rules".into());
        }
    }
    let output = std::process::Command::new("hyprctl").arg("configerrors").output()
        .map_err(|e| format!("cannot validate Hyprland window rules: {e}"))?;
    if !output.status.success() || !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        // Config error contents may contain private user commands or paths.
        return Err("Hyprland reports configuration errors. Run hyprctl configerrors to inspect them.".into());
    }
    Ok(())
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

/// Install the portal actions in the user's Lua bindings. Omarchy loads this
/// file after its defaults, so the marked block is the only text Verenu owns.
#[cfg(target_os = "linux")]
pub(crate) fn ensure_global_shortcut_binding(
    bindings: &[String],
    press_portal_id: &str,
    release_command: &str,
    handsfree_command: &str,
    release_keycodes: &[u32],
) -> Result<(), String> {
    ensure_window_rules()?;
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
    let block = global_shortcut_binding_block(
        start,
        end,
        bindings,
        press_portal_id,
        release_command,
        handsfree_command,
        release_keycodes,
    );
    let next = replace_managed_block(&current, start, end, &block)?;
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

/// Replaces only a complete, correctly ordered managed block.
#[cfg(target_os = "linux")]
fn replace_managed_block(current: &str, start: &str, end: &str, block: &str) -> Result<String, String> {
    match (current.find(start), current.find(end)) {
        (Some(a), Some(b)) if a < b => Ok(format!("{}{}{}", &current[..a], block, &current[b + end.len()..])),
        (None, None) => Ok(format!("{}\n\n{}\n", current.trim_end(), block)),
        _ => Err("Incomplete or inverted Verenu shortcut markers".into()),
    }
}

/// Installs the fixed sub-app capture chord in its own managed block. The
/// bind runs a single-instance handoff (`--verenu-capture-sub-app`); exec
/// does not move focus, so the running app captures the user's window.
#[cfg(target_os = "linux")]
pub(crate) fn ensure_sub_app_capture_binding(chord: &str, capture_command: &str) -> Result<(), String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| "XDG config directory is unavailable".to_string())?;
    let path = config.join("hypr/bindings.lua");
    let current =
        fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let block = sub_app_capture_binding_block(chord, capture_command);
    let (start, end) = SUB_APP_CAPTURE_MARKERS;
    let next = replace_managed_block(&current, start, end, &block)?;
    if next == current {
        return Ok(());
    }
    let tmp = path.with_extension("lua.verenu.tmp");
    fs::write(&tmp, next).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))?;
    let status = std::process::Command::new("hyprctl")
        .arg("reload")
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Hyprland reload failed: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Hyprland rejected the Verenu sub-app capture binding".to_string())
}

#[cfg(target_os = "linux")]
const SUB_APP_CAPTURE_MARKERS: (&str, &str) = (
    "-- >>> Verenu sub-app capture (do not edit) <<<",
    "-- <<< End Verenu sub-app capture >>>",
);

#[cfg(target_os = "linux")]
fn sub_app_capture_binding_block(chord: &str, capture_command: &str) -> String {
    let (start, end) = SUB_APP_CAPTURE_MARKERS;
    let command = capture_command.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{start}\n_verenu_capture_binding = hl.bind(\"{chord}\", hl.dsp.exec_cmd(\"{command}\"), {{ description = \"Verenu capture sub-app\" }})\n{end}"
    )
}

/// xkb keycodes that do not turn a held chord into a different shortcut:
/// Escape, Space, Caps Lock, modifiers, and hardware media controls. Linux
/// evdev media codes are translated to XKB codes by adding 8, just like the
/// compositor's modifier keycodes. These controls remain usable during a hold.
#[cfg(target_os = "linux")]
const NON_COMBO_KEYCODES: &[u32] = &[
    9, 65, 66, 37, 105, 50, 62, 64, 108, 133, 134,
    121, 122, 123, // mute, volume down/up
    171, 172, 173, 174, 208, 209, 215, // next/play/pause/previous/stop
    232, 233, 236, 237, 238, 251, 252, 256, // display/keyboard brightness, mic mute
];

#[cfg(target_os = "linux")]
const GLOBAL_SHORTCUT_TEMPLATE: &str = r#"{start}
-- Portal action: {press_portal_id}
local verenu_dictate = hl.dsp.global("{press_portal_id}")
_verenu_dictation_bindings = {}
local verenu_cancel = hl.dsp.global("{cancel_portal_id}")
local verenu_release = hl.dsp.exec_cmd("{release_command}")
local verenu_handsfree = hl.dsp.exec_cmd("{handsfree_command}")
local verenu_release_armed = false
local verenu_release_generation = 0
local verenu_press_generation = 0
local verenu_short_hold = false
local verenu_quick_tap_pending = false
local verenu_quick_tap_generation = 0
-- True once another key joins the held chord: the user is running some other
-- shortcut that happens to start with these modifiers (Omarchy has many
-- Super+Ctrl binds), not dictating.
local verenu_combo = false
local verenu_suppress_press = false
local verenu_release_keycodes = { {release_keycodes} }
local verenu_ignored_keycodes = {}
for _, ignored in ipairs({ {ignored_keycodes} }) do
  verenu_ignored_keycodes[ignored] = true
end
local verenu_release_key = function(generation)
  if not verenu_release_armed or generation ~= verenu_release_generation then
    return
  end
  verenu_release_armed = false
  hl.dispatch(verenu_release)
  hl.dispatch(hl.dsp.submap("reset"))
end
local verenu_press = function()
  -- A chord pressed right after a combo is almost always the next combo.
  if verenu_suppress_press then
    return
  end
  verenu_combo = false
  verenu_press_generation = verenu_press_generation + 1
  local press_generation = verenu_press_generation
  if verenu_quick_tap_pending then
    -- The second physical press can arrive before the first release helper
    -- finishes its single-instance handoff. Decide the gesture here, where
    -- both key edges are still ordered by Hyprland.
    verenu_quick_tap_pending = false
    verenu_quick_tap_generation = verenu_quick_tap_generation + 1
    verenu_release_generation = verenu_release_generation + 1
    verenu_release_armed = true
    hl.dispatch(verenu_handsfree)
  else
    verenu_release_generation = verenu_release_generation + 1
    verenu_release_armed = true
    hl.dispatch(verenu_dictate)
  end
  verenu_short_hold = true
  hl.timer(function()
    if press_generation == verenu_press_generation then
      verenu_short_hold = false
    end
  end, { timeout = 700, type = "oneshot" })
end
hl.on("input.keyboard.key", function(keycode, _, state)
  if not verenu_release_armed then
    return
  end
  if state == 1 then
    if not verenu_combo and not verenu_ignored_keycodes[keycode] then
      verenu_combo = true
      hl.dispatch(verenu_cancel)
    end
    return
  end
  if state ~= 0 then
    return
  end
  for _, release_keycode in ipairs(verenu_release_keycodes) do
    if keycode == release_keycode then
      if verenu_combo then
        -- Not a tap: it must not arm the double-tap hands-free gesture, and
        -- the next chord press is swallowed for a moment.
        verenu_short_hold = false
        verenu_suppress_press = true
        hl.timer(function()
          verenu_suppress_press = false
        end, { timeout = 400, type = "oneshot" })
      end
      if verenu_short_hold then
        verenu_quick_tap_pending = true
        verenu_quick_tap_generation = verenu_quick_tap_generation + 1
        local tap_generation = verenu_quick_tap_generation
        hl.timer(function()
          if tap_generation == verenu_quick_tap_generation then
            verenu_quick_tap_pending = false
          end
        end, { timeout = 350, type = "oneshot" })
      end
      local generation = verenu_release_generation
      -- Let the portal press event reach Verenu before handing off the
      -- release. This covers very short taps without blocking Hyprland.
      hl.timer(function()
        verenu_release_key(generation)
      end, { timeout = 25, type = "oneshot" })
      return
    end
  end
end)
{binds}
{end}"#;

#[cfg(target_os = "linux")]
fn global_shortcut_binding_block(
    start: &str,
    end: &str,
    bindings: &[String],
    press_portal_id: &str,
    release_command: &str,
    handsfree_command: &str,
    release_keycodes: &[u32],
) -> String {
    // Lua dispatchers are registered with Hyprland as generic `__lua`
    // handlers. Hyprland 0.56 can miss modifier-only release binds and the
    // matching global-shortcut release event. Watch the compositor's raw
    // keyboard event stream instead, but arm that watcher only after the
    // Verenu press bind fires so ordinary modifier releases remain untouched.
    let binds = bindings
        .iter()
        .map(|binding| {
            format!(
                "_verenu_dictation_bindings[#_verenu_dictation_bindings + 1] = hl.bind(\"{binding}\", verenu_press, {{ description = \"Verenu dictation\", submap_universal = true }})"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let release_list = release_keycodes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    // Rejecting another shortcut's modifier prefix is distinct from Escape:
    // the backend ignores this action during hands-free and processing.
    let cancel_portal_id = press_portal_id.rsplit_once(':').map_or_else(
        || "cancel-chord".to_string(),
        |(scope, _)| format!("{scope}:cancel-chord"),
    );
    // Keys that never mean "this is another shortcut": every modifier (the
    // chord is made of them), the chord's own keys, Space (hands-free) and
    // Escape (cancel).
    let ignored_keycodes = release_keycodes
        .iter()
        .chain(NON_COMBO_KEYCODES.iter())
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    // Commands are interpolated into Lua string literals, so escape both
    // backslashes and quotes before inserting them into the template.
    let release_command = release_command.replace('\\', "\\\\").replace('"', "\\\"");
    let handsfree_command = handsfree_command.replace('\\', "\\\\").replace('"', "\\\"");
    GLOBAL_SHORTCUT_TEMPLATE
        .replace("{start}", start)
        .replace("{end}", end)
        .replace("{press_portal_id}", press_portal_id)
        .replace("{cancel_portal_id}", &cancel_portal_id)
        .replace("{release_command}", &release_command)
        .replace("{handsfree_command}", &handsfree_command)
        .replace("{release_keycodes}", &release_list)
        .replace("{ignored_keycodes}", &ignored_keycodes)
        .replace("{binds}", &binds)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn active_window() -> Option<()> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    #[ignore = "requires this user's Hyprland command socket and hyprctl"]
    fn native_socket_queries_read_monitors_without_process_launches() {
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let bytes = super::socket_request("j/monitors").expect("Hyprland socket unavailable").unwrap();
            assert!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap().is_array());
        }
        let direct = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let output = std::process::Command::new("hyprctl").args(["-j", "monitors"]).output().unwrap();
            assert!(output.status.success());
            assert!(serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap().is_array());
        }
        eprintln!("Hyprland monitors, 20 requests: socket={direct:?}, hyprctl={:?}", start.elapsed());
    }

    #[test]
    fn socket_exchange_sends_exact_command_and_reads_all_response_fragments() {
        use std::io::{Read, Write};
        let (client, mut compositor) = std::os::unix::net::UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut request = [0; 9];
            compositor.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"j/clients");
            compositor.write_all(b"[{\"address\":").unwrap();
            compositor.write_all(b"\"pill\"}]").unwrap();
        });
        let response = super::socket_exchange(client, "j/clients").unwrap();
        server.join().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(value[0]["address"], "pill");
    }

    #[test]
    fn socket_exchange_preserves_compositor_errors_without_replaying_dispatch() {
        use std::io::{Read, Write};
        let (client, mut compositor) = std::os::unix::net::UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut request = [0; 14];
            compositor.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"/dispatch test");
            compositor.write_all(b"error: fixture").unwrap();
        });
        assert_eq!(super::socket_exchange(client, "/dispatch test").unwrap(), b"error: fixture");
        server.join().unwrap();
    }

    #[test]
    fn fresh_window_rules_are_installed_idempotently_and_upgrade_dev_rules() {
        let (start, end) = super::WINDOW_RULE_MARKERS;
        let block = super::window_rule_block();
        let personal = "-- personal bindings\nhl.bind('SUPER + A', 'custom')\n";
        let fresh = super::replace_managed_block(personal, start, end, &block).unwrap();
        assert!(fresh.starts_with(personal));
        assert_eq!(super::replace_managed_block(&fresh, start, end, &block).unwrap(), fresh);
        let old = format!("{personal}{start}\no.window({{ class = '^verenu$' }}, {{ float = true }})\n{end}\n-- personal tail");
        let upgraded = super::replace_managed_block(&old, start, end, &block).unwrap();
        assert!(upgraded.starts_with(personal));
        assert!(upgraded.ends_with("\n-- personal tail"));
        assert!(!upgraded.contains("^verenu$"));
        assert_eq!(upgraded.matches("name = \"verenu-pill\"").count(), 1);
        assert!(block.contains("no_initial_focus = true"));
        assert!(!block.contains("no_focus = true"));
    }

    #[test]
    fn window_policy_matches_release_identity_and_size_contract() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        // Keep GTK's default registration behavior: headless dev sessions
        // and the installed app must not share a remote GtkApplication.
        assert_ne!(config["app"]["enableGTKAppId"], true);
        assert_eq!(config["identifier"], super::WINDOW_APP_ID);
        super::initialize_app_identity();
        assert_eq!(gtk::glib::prgname().as_deref(), Some(super::WINDOW_APP_ID));
        assert_eq!(config["app"]["windows"][0]["minWidth"], 1100);
        assert_eq!(config["app"]["windows"][0]["minHeight"], 700);
        let block = super::window_rule_block();
        assert!(block.contains(r#"class = "^(com\\.verenu\\.app|[Vv]erenu)$""#));
        assert!(block.contains("center = true"));
        assert!(block.contains("min_size = { 1100, 700 }"));
    }

    #[test]
    fn pill_lookup_survives_renamed_release_binaries_and_rejects_other_processes() {
        for class in ["verenu", "Verenu", "verenu-0.20.0", "com.verenu.app"] {
            let clients = serde_json::json!([
                { "class": class, "title": "Verenu Dictation Pill", "pid": 99, "address": "other" },
                { "class": class, "title": "Verenu", "pid": 42, "address": "main" },
                { "class": class, "title": "Verenu Dictation Pill", "pid": 42, "address": "pill" }
            ]);
            assert_eq!(super::owned_pill_window(&clients, 42).unwrap().address, "pill");
            assert!(super::owned_pill_window(&clients, 43).is_none());
        }
    }

    #[test]
    fn media_controls_are_not_competing_chords_but_letters_are() {
        for key in [121, 122, 123, 171, 172, 173, 174, 232, 233, 256] {
            assert!(super::NON_COMBO_KEYCODES.contains(&key));
        }
        for key in [38, 39, 40, 41, 67] {
            assert!(!super::NON_COMBO_KEYCODES.contains(&key));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_blocks_reject_partial_or_inverted_markers() {
        for current in ["START", "END", "END\nSTART"] {
            assert!(super::replace_managed_block(current, "START", "END", "replacement").is_err());
        }
        assert_eq!(super::replace_managed_block("before START old END after", "START", "END", "new").unwrap(), "before new after");
    }
    use super::{global_shortcut_binding_block, HyprMonitor, LogicalMonitor};

    #[test]
    #[ignore = "requires a Lua interpreter; executes generated bindings with a fake compositor"]
    fn generated_lua_handles_double_taps_mouse_chords_and_combo_rejection() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let block = global_shortcut_binding_block(
            "-- START",
            "-- END",
            &["CTRL + Super_L".to_string(), "SUPER + Control_L".to_string()],
            "app:dictate",
            "release",
            "handsfree",
            &[37, 105, 133, 134],
        );
        let fixture = include_str!("../../../tests/fixtures/hyprland-gestures.lua")
            .replace("-- GENERATED_WINDOW_RULES", &super::window_rule_block())
            .replace("-- GENERATED_BINDINGS", &block);
        let mut child = Command::new("lua")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Lua interpreter is required for this native fixture");
        child.stdin.take().unwrap().write_all(fixture.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn scaled_monitor_converts_mode_pixels_before_adding_layout_origin() {
        let monitor = HyprMonitor {
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
    fn sub_app_capture_block_binds_the_fixed_chord_to_the_handoff() {
        let block = super::sub_app_capture_binding_block(
            "CTRL + ALT + SHIFT + S",
            "'/usr/bin/verenu' --verenu-capture-sub-app",
        );
        assert!(block.starts_with(super::SUB_APP_CAPTURE_MARKERS.0));
        assert!(block.ends_with(super::SUB_APP_CAPTURE_MARKERS.1));
        assert!(block.contains(
            "hl.bind(\"CTRL + ALT + SHIFT + S\", hl.dsp.exec_cmd(\"'/usr/bin/verenu' --verenu-capture-sub-app\"), { description = \"Verenu capture sub-app\" })"
        ));
    }

    #[test]
    fn global_shortcut_binding_forwards_press_and_release() {
        let block = global_shortcut_binding_block(
            "START",
            "END",
            &["CTRL + SPACE".to_string()],
            "app:dictate",
            "/usr/bin/verenu --verenu-hotkey-release",
            "/usr/bin/verenu --verenu-hotkey-handsfree",
            &[65],
        );

        assert!(block.contains("local verenu_dictate = hl.dsp.global(\"app:dictate\")"));
        assert!(block.contains(
            "local verenu_release = hl.dsp.exec_cmd(\"/usr/bin/verenu --verenu-hotkey-release\")"
        ));
        assert!(block.contains(
            "local verenu_handsfree = hl.dsp.exec_cmd(\"/usr/bin/verenu --verenu-hotkey-handsfree\")"
        ));
        assert!(block.contains("local verenu_quick_tap_pending = false"));
        assert!(block.contains("hl.dispatch(verenu_handsfree)"));
        assert!(block.contains(
            "hl.on(\"input.keyboard.key\", function(keycode, _, state)"
        ));
        assert!(block.contains("local verenu_release_keycodes = { 65 }"));
        assert!(block.contains("local verenu_cancel = hl.dsp.global(\"app:cancel-chord\")"));
        assert!(block.contains(
            "for _, ignored in ipairs({ 65, 9, 65, 66, 37, 105, 50, 62, 64, 108, 133, 134, 121, 122, 123,"
        ));
        assert!(block.contains("type = \"oneshot\""));
        assert!(block.contains(
            "hl.bind(\"CTRL + SPACE\", verenu_press, { description = \"Verenu dictation\", submap_universal = true })"
        ));
    }

    #[test]
    fn global_shortcut_binding_supports_both_modifier_press_orders() {
        let block = global_shortcut_binding_block(
            "START",
            "END",
            &[
                "CTRL + Super_L".to_string(),
                "SUPER + Control_L".to_string(),
            ],
            "app:dictate",
            "/usr/bin/verenu --verenu-hotkey-release",
            "/usr/bin/verenu --verenu-hotkey-handsfree",
            &[37, 133],
        );

        for binding in ["CTRL + Super_L", "SUPER + Control_L"] {
            assert!(block.contains(&format!(
                "hl.bind(\"{binding}\", verenu_press, {{ description = \"Verenu dictation\", submap_universal = true }})"
            )));
        }
        assert!(block.contains("local verenu_release_keycodes = { 37, 133 }"));
    }

    #[test]
    fn global_shortcut_binding_escapes_commands_for_lua_strings() {
        let block = global_shortcut_binding_block(
            "START",
            "END",
            &["CTRL + SPACE".to_string()],
            "app:dictate",
            r#"C:\Program Files\Verenu\verenu.exe --release "quoted""#,
            r#"C:\Program Files\Verenu\verenu.exe --handsfree"#,
            &[65],
        );

        assert!(block.contains(
            r#"local verenu_release = hl.dsp.exec_cmd("C:\\Program Files\\Verenu\\verenu.exe --release \"quoted\"")"#
        ));
        assert!(block.contains(
            r#"local verenu_handsfree = hl.dsp.exec_cmd("C:\\Program Files\\Verenu\\verenu.exe --handsfree")"#
        ));
    }
}
