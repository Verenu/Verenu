//! XDG Desktop Portal global shortcut implementation for Wayland/Hyprland.
//!
//! The portal owns conflict detection and consent UI. We intentionally do not
//! read `/dev/input`, install X11 hooks, or use a privileged input helper.

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[path = "linux_conflicts.rs"]
mod conflicts;
#[path = "linux_shortcuts.rs"]
mod shortcuts;

const CTRL: u32 = 1;
const ALT: u32 = 2;
const SHIFT: u32 = 3;
const SUPER: u32 = 4;
static KEYS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static EFFECTIVE_KEYS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
fn configured_keys(slot: &Mutex<Vec<u32>>) -> Vec<u32> {
    slot.lock()
        .map(|keys| {
            if keys.is_empty() {
                vec![CTRL, SUPER]
            } else {
                keys.clone()
            }
        })
        .unwrap_or_else(|_| vec![CTRL, SUPER])
}
static CANCEL_KEY: Mutex<String> = Mutex::new(String::new());
static HANDSFREE_KEY: Mutex<String> = Mutex::new(String::new());
static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(0);
static HANDLESS: AtomicBool = AtomicBool::new(false);
static PROCESSING: AtomicU64 = AtomicU64::new(0);
/// Mirrors `PortalGesture::chord_active` across threads so Escape arming can
/// see a held chord. Updated by the portal thread on every edge; read by
/// `refresh_escape_listening` from any thread.
static CHORD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Whether the dynamic bare-Escape Hyprland bind is currently installed.
/// The Lua handle disables only Verenu's binding, preserving user bindings.
static ESCAPE_ARMED: AtomicBool = AtomicBool::new(false);
/// App-scoped portal id of the trigger-less "cancel" shortcut (see
/// `PORTAL_CANCEL_ID`), resolved at startup. Needed to build the dynamic
/// Escape bind's `hl.dsp.global(...)` action.
static CANCEL_PORTAL_ID: Mutex<Option<String>> = Mutex::new(None);
static HANDSFREE_PORTAL_ID: Mutex<Option<String>> = Mutex::new(None);
static SPACE_ARMED: AtomicBool = AtomicBool::new(false);
static GESTURE: OnceLock<Mutex<PortalGesture>> = OnceLock::new();

static PRESS: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static RELEASE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static HANDLESS_CB: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static CANCEL: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static CHORD_CANCEL: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static ESCAPE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static COPY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static SUB_APP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

const HANDSFREE_DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(350);
// A first tap must never reach the recording quality gate while a second
// tap can still promote its open capture. Match the compositor's tap limit.
const TAP_MAX_HOLD: Duration = Duration::from_millis(700);
const STALE_CHORD_TIMEOUT: Duration = Duration::from_secs(2);
const PORTAL_SHORTCUT_ID: &str = "dictate";
const PORTAL_SHORTCUT_DESCRIPTION: &str = "Verenu dictation";
/// Second portal shortcut: the Escape-to-cancel action. Bound with NO
/// preferred trigger (verified: the portal registers a trigger-less action
/// fine) so there is nothing to collide with and no consent UI for a key the
/// user never presses directly. Hyprland dispatches this action through a
/// bare-Escape bind that only exists while a dictation is active (see
/// `refresh_escape_listening`) — a static Escape bind would swallow Escape in
/// every app, all the time.
const PORTAL_CANCEL_ID: &str = "cancel";
const PORTAL_CANCEL_DESCRIPTION: &str = "Verenu cancel";
const PORTAL_CHORD_CANCEL_ID: &str = "cancel-chord";
const PORTAL_CHORD_CANCEL_DESCRIPTION: &str = "Verenu discard hold gesture";
const PORTAL_COPY_ID: &str = "copy-last";
const PORTAL_COPY_DESCRIPTION: &str = "Verenu copy last dictation";
const PORTAL_HANDSFREE_ID: &str = "handsfree";
const PORTAL_HANDSFREE_DESCRIPTION: &str = "Verenu hands-free";
/// Backoff between portal reconnect attempts. A dead portal stream used to
/// kill the hotkey thread silently (`else => break`), stranding any active
/// recording with no way to stop it short of killing the app.
const PORTAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortalGestureAction {
    Press,
    Release,
    Handsfree,
    Cancel,
}

/// Turns the portal's chord-level Activated/Deactivated stream into the same
/// gesture contract used by the Windows and macOS hotkey backends.
///
/// Hyprland may deliver another Activated notification while a key is still
/// held. Tracking the physical chord edge here prevents that repeat from being
/// mistaken for the second tap of a hands-free gesture. Once a real quick tap
/// is released, its matching recording stays open and exactly one following
/// activation inside the double-tap window enters hands-free. The release of
/// that second tap is consumed here, so it cannot immediately stop the session
/// it just started.
#[derive(Default)]
struct PortalGesture {
    chord_active: bool,
    pressed_at: Option<Instant>,
    pending_tap_at: Option<Instant>,
    consume_deactivation: bool,
    pending_release: Option<(Instant, PortalGestureAction)>,
}

impl PortalGesture {
    fn activated(&mut self, now: Instant) -> Option<PortalGestureAction> {
        if self.chord_active
            && self
                .pressed_at
                .is_some_and(|pressed_at| now.duration_since(pressed_at) > STALE_CHORD_TIMEOUT)
        {
            // Hyprland can emit another activation while a chord is still
            // held. Do not re-anchor the hold clock here. Doing that made a
            // multi-second hold look like a fresh quick tap when the eventual
            // deactivation arrived, which armed hands-free by accident on the
            // next Ctrl+Space press.
            if let Some(pressed_at) = self.pressed_at {
                log::info!(
                    "linux hotkey: ignored repeated activation after {}ms",
                    now.duration_since(pressed_at).as_millis()
                );
            }
            return None;
        }
        if self.chord_active || self.consume_deactivation {
            return None;
        }

        let is_double_tap = self.pending_tap_at.take().is_some_and(|released_at| {
            now.duration_since(released_at) <= HANDSFREE_DOUBLE_TAP_WINDOW
        });
        // This activation either consumes the pending tap as hands-free or
        // begins a new gesture. Never let its delayed cleanup affect that
        // new recording.
        self.pending_release = None;
        if is_double_tap {
            self.consume_deactivation = true;
            return Some(PortalGestureAction::Handsfree);
        }

        self.chord_active = true;
        self.pressed_at = Some(now);
        Some(PortalGestureAction::Press)
    }

    fn deactivated(&mut self, now: Instant) -> Option<PortalGestureAction> {
        if self.consume_deactivation {
            self.consume_deactivation = false;
            return None;
        }
        if !self.chord_active {
            return None;
        }

        self.chord_active = false;
        let held = self
            .pressed_at
            .take()
            .map(|pressed_at| now.duration_since(pressed_at))
            .unwrap_or_default();
        // Info-level by design: the held duration is the primary signal for
        // diagnosing stuck holds (a release measured from a stale repeat
        // edge classifies as a tap and cancels instead of transcribing).
        log::info!(
            "linux hotkey: chord deactivated after {}ms",
            held.as_millis()
        );
        if held < TAP_MAX_HOLD {
            self.pending_tap_at = Some(now);
            self.pending_release = Some((
                now + HANDSFREE_DOUBLE_TAP_WINDOW,
                PortalGestureAction::Cancel,
            ));
            None
        } else {
            self.pending_tap_at = None;
            Some(PortalGestureAction::Release)
        }
    }

    fn take_pending_release(&mut self, now: Instant) -> Option<PortalGestureAction> {
        let (deadline, action) = self.pending_release?;
        if now < deadline {
            return None;
        }
        self.pending_release = None;
        self.pending_tap_at = None;
        Some(action)
    }
}

// W3C code, XKB keysym and keycode. Function/Space ids remain compatible.
const REGULAR_KEYS: &[(&str, &str, u32)] = &[
    ("KeyA", "A", 38),
    ("KeyB", "B", 56),
    ("KeyC", "C", 54),
    ("KeyD", "D", 40),
    ("KeyE", "E", 26),
    ("KeyF", "F", 41),
    ("KeyG", "G", 42),
    ("KeyH", "H", 43),
    ("KeyI", "I", 31),
    ("KeyJ", "J", 44),
    ("KeyK", "K", 45),
    ("KeyL", "L", 46),
    ("KeyM", "M", 58),
    ("KeyN", "N", 57),
    ("KeyO", "O", 32),
    ("KeyP", "P", 33),
    ("KeyQ", "Q", 24),
    ("KeyR", "R", 27),
    ("KeyS", "S", 39),
    ("KeyT", "T", 28),
    ("KeyU", "U", 30),
    ("KeyV", "V", 55),
    ("KeyW", "W", 25),
    ("KeyX", "X", 53),
    ("KeyY", "Y", 29),
    ("KeyZ", "Z", 52),
    ("Escape", "Escape", 9),
    ("Enter", "Return", 36),
    ("Backspace", "BackSpace", 22),
    ("Tab", "Tab", 23),
    ("CapsLock", "Caps_Lock", 66),
    ("Minus", "minus", 20),
    ("Equal", "equal", 21),
    ("BracketLeft", "bracketleft", 34),
    ("BracketRight", "bracketright", 35),
    ("Backslash", "backslash", 51),
    ("Semicolon", "semicolon", 47),
    ("Quote", "apostrophe", 48),
    ("Comma", "comma", 59),
    ("Period", "period", 60),
    ("Slash", "slash", 61),
    ("Backquote", "grave", 49),
    ("ArrowUp", "Up", 111),
    ("ArrowDown", "Down", 116),
    ("ArrowLeft", "Left", 113),
    ("ArrowRight", "Right", 114),
    ("Insert", "Insert", 118),
    ("Delete", "Delete", 119),
    ("Home", "Home", 110),
    ("End", "End", 115),
    ("PageUp", "Prior", 112),
    ("PageDown", "Next", 117),
    ("Digit0", "0", 19),
    ("Digit1", "1", 10),
    ("Digit2", "2", 11),
    ("Digit3", "3", 12),
    ("Digit4", "4", 13),
    ("Digit5", "5", 14),
    ("Digit6", "6", 15),
    ("Digit7", "7", 16),
    ("Digit8", "8", 17),
    ("Digit9", "9", 18),
];
pub fn map_code_to_vk(code: &str) -> u32 {
    match code {
        "ControlLeft" | "ControlRight" => CTRL,
        "AltLeft" | "AltRight" => ALT,
        "ShiftLeft" | "ShiftRight" => SHIFT,
        "MetaLeft" | "MetaRight" => SUPER,
        "F5" => 105,
        "F1" => 101,
        "F2" => 102,
        "F3" => 103,
        "F4" => 104,
        "F6" => 106,
        "F7" => 107,
        "F8" => 108,
        "F9" => 109,
        "F10" => 110,
        "F11" => 111,
        "F12" => 112,
        "Space" => 200,
        _ => REGULAR_KEYS
            .iter()
            .find(|(name, _, _)| *name == code)
            .map(|(_, _, keycode)| 1000 + keycode)
            .unwrap_or(0),
    }
}

fn portal_modifier_name(modifier: u32) -> Option<&'static str> {
    match modifier {
        CTRL => Some("CTRL"),
        ALT => Some("ALT"),
        SHIFT => Some("SHIFT"),
        SUPER => Some("SUPER"),
        _ => None,
    }
}

fn shortcut_code(key: u32) -> String {
    match key {
        CTRL => "ControlLeft".into(),
        ALT => "AltLeft".into(),
        SHIFT => "ShiftLeft".into(),
        SUPER => "MetaLeft".into(),
        200 => "Space".into(),
        101..=112 => format!("F{}", key - 100),
        _ => REGULAR_KEYS
            .iter()
            .find(|(_, _, code)| 1000 + code == key)
            .map(|(name, _, _)| (*name).to_string())
            .unwrap_or_default(),
    }
}

fn shortcut_display(keys: &[u32]) -> String {
    keys.iter()
        .map(|key| match *key {
            CTRL => "Ctrl".into(),
            ALT => "Alt".into(),
            SHIFT => "Shift".into(),
            SUPER => "Super".into(),
            _ => shortcut_code(*key),
        })
        .collect::<Vec<String>>()
        .join("+")
}

fn hyprland_modifier_name(modifier: u32) -> Option<&'static str> {
    match modifier {
        CTRL => Some("Control_L"),
        ALT => Some("Alt_L"),
        SHIFT => Some("Shift_L"),
        SUPER => Some("Super_L"),
        _ => None,
    }
}

fn hyprland_keycode(key: u32) -> Option<u32> {
    match key {
        101..=110 => Some(key - 34), // F1–F10: XKB 67–76
        111..=112 => Some(key - 16), // F11–F12: XKB 95–96
        200 => Some(65),             // Space
        _ => REGULAR_KEYS
            .iter()
            .find(|(_, _, code)| 1000 + code == key)
            .map(|(_, _, code)| *code),
    }
}

fn hyprland_modifier_keycodes(modifier: u32) -> &'static [u32] {
    match modifier {
        CTRL => &[37, 105],
        ALT => &[64, 108],
        SHIFT => &[50, 62],
        SUPER => &[133, 134],
        _ => &[],
    }
}

fn release_keycodes_for(keys: &[u32]) -> Vec<u32> {
    keys.iter()
        .flat_map(|key| {
            hyprland_modifier_keycodes(*key)
                .iter()
                .copied()
                .chain(hyprland_keycode(*key))
        })
        .collect()
}
#[cfg(test)]
fn release_keycodes(first: u32, second: u32) -> Vec<u32> {
    release_keycodes_for(&[first, second])
}
#[cfg(test)]
fn shortcut_configuration(first: u32, second: u32) -> Option<(Option<String>, Vec<String>)> {
    shortcut_configuration_for(&[first, second])
}

fn shortcut_configuration_for(keys: &[u32]) -> Option<(Option<String>, Vec<String>)> {
    let mut modifiers = Vec::new();
    let mut regular = Vec::new();
    for key in keys {
        if portal_modifier_name(*key).is_some() {
            if modifiers.contains(key) {
                return None;
            }
            modifiers.push(*key);
        } else {
            regular.push(*key);
        }
    }
    modifiers.sort_unstable();
    if regular.is_empty() {
        if modifiers.len() < 2 {
            return None;
        }
        let mut bindings = Vec::new();
        for activating in modifiers.iter().rev() {
            let mask = modifiers
                .iter()
                .filter(|id| *id != activating)
                .filter_map(|id| portal_modifier_name(*id))
                .collect::<Vec<_>>()
                .join(" + ");
            let key = hyprland_modifier_name(*activating)?;
            bindings.push(format!("{mask} + {key}"));
            if *activating != SUPER {
                bindings.push(format!("{mask} + {}", key.replace("_L", "_R")));
            }
        }
        return Some((None, bindings));
    }
    if regular.len() != 1 {
        return None;
    }
    let key = regular[0];
    if modifiers.is_empty() && !(101..=112).contains(&key) {
        return None;
    }
    let key_name = match key {
        101..=112 => format!("F{}", key - 100),
        200 => "SPACE".to_string(),
        _ => REGULAR_KEYS
            .iter()
            .find(|(_, _, code)| 1000 + code == key)?
            .1
            .to_string(),
    };
    let mut parts = modifiers
        .iter()
        .filter_map(|id| portal_modifier_name(*id))
        .map(str::to_string)
        .collect::<Vec<_>>();
    parts.push(key_name);
    let trigger = parts.join("+");
    Some((Some(trigger.clone()), vec![trigger.replace('+', " + ")]))
}

pub fn is_hotkey_available(keys: &[String]) -> Result<bool, String> {
    let ids = super::mapped_codes(keys)?;
    let (_, triggers) = shortcut_configuration_for(&ids)
        .ok_or("Use modifiers plus one regular key, a function key, or a modifier-only combination on Linux")?;
    Ok(conflicts::free(
        &conflicts::bindings("dictation")?,
        &triggers,
        false,
    ))
}
pub fn update_keys(keys: &[u32]) -> Result<(), String> {
    shortcut_configuration_for(keys).ok_or("Unsupported Linux shortcut")?;
    *KEYS.lock().map_err(|_| "Shortcut state unavailable")? = keys.to_vec();
    CONFIG_GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

pub fn suspend_shortcuts(active: bool) -> Result<(), String> {
    let enabled = if active { "false" } else { "true" };
    eval_hyprland(&format!(
        "if not _verenu_dictation_bindings then error('Verenu shortcut is not ready') end; for _, handle in ipairs(_verenu_dictation_bindings) do if tostring(handle) ~= 'HL.Keybind(expired)' then handle:set_enabled({enabled}) end end; for _, handle in pairs({{_verenu_copy_binding, _verenu_capture_binding}}) do if tostring(handle) ~= 'HL.Keybind(expired)' then handle:set_enabled({enabled}) end end"
    ))
}

/// Called by the Linux single-instance handoff from Hyprland's keyboard-event
/// release watcher. This avoids the broken global-shortcuts release event for
/// modifier-only chords while preserving the same gesture classification.
pub fn notify_release() {
    if super::capture_active() {
        return;
    }
    let Some(gesture) = GESTURE.get() else {
        return;
    };
    let Ok(mut gesture) = gesture.lock() else {
        log::error!("linux hotkey: gesture state lock was poisoned");
        return;
    };
    let action = gesture.deactivated(Instant::now());
    let pending_release = gesture.pending_release.is_some();
    drop(gesture);
    CHORD_ACTIVE.store(false, Ordering::SeqCst);
    refresh_escape_listening();
    match action {
        Some(PortalGestureAction::Release) => {
            log::info!("linux hotkey: application release handoff fired");
            if let Some(cb) = RELEASE.get() {
                cb();
            }
        }
        None if pending_release => {
            log::info!("linux hotkey: retaining first tap capture for handsfree");
        }
        _ => log::info!("linux hotkey: consumed application release handoff"),
    }
}

fn flush_pending_release(now: Instant) {
    let action = GESTURE
        .get()
        .and_then(|gesture| gesture.lock().ok()?.take_pending_release(now));
    if action == Some(PortalGestureAction::Cancel) {
        log::info!("linux hotkey: lone tap expired; discarding hold gesture");
        if let Some(cb) = CANCEL.get() {
            cb();
        }
    }
}

fn should_cancel_hold(chord_active: bool, handsfree: bool) -> bool {
    chord_active && !handsfree
}

pub fn set_chord_cancel_callback(callback: impl Fn() + Send + Sync + 'static) {
    let _ = CHORD_CANCEL.set(Box::new(callback));
}
static SUB_APP_CHORD: Mutex<Option<super::chord::Chord>> = Mutex::new(None);

/// Rebinds the sub-app capture chord. Rewrites only the capture block in the
/// Hyprland config, so the dictation bind and portal session are untouched.
pub fn set_sub_app_capture_chord(chord: super::chord::Chord) {
    if let Ok(mut slot) = SUB_APP_CHORD.lock() {
        *slot = Some(chord);
    }
    install_sub_app_capture_binding();
}

fn install_sub_app_capture_binding() {
    let chord = SUB_APP_CHORD
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .unwrap_or_else(super::chord::Chord::default_for_platform);
    let requested = chord.to_string();
    let active = conflicts::bindings("capture").ok().and_then(|bindings| {
        conflicts::choose(
            &bindings,
            &requested,
            &["Ctrl+Alt+Shift+F7", "Ctrl+Alt+Shift+F8", "Alt+Super+F7"],
            false,
        )
    });
    let Some(active) = active else {
        let _ = shortcuts::remove_block(
            "-- >>> Verenu sub-app capture (do not edit) <<<",
            "-- <<< End Verenu sub-app capture >>>",
        );
        conflicts::status("capture", requested, None, vec![]);
        return;
    };
    let Some(chord) = super::chord::Chord::parse(&active) else {
        return;
    };
    match std::env::current_exe() {
        Ok(exe) => {
            let capture_command = format!(
                "{} --verenu-capture-sub-app",
                shell_quote(&exe.to_string_lossy())
            );
            if let Err(error) = crate::core::hyprland::ensure_sub_app_capture_binding(
                &chord.hyprland(),
                &capture_command,
            ) {
                log::warn!("linux hotkey: sub-app capture binding unavailable: {error}");
                conflicts::status("capture", requested, None, vec![]);
                return;
            }
            conflicts::status("capture", requested, Some(active), vec![]);
            ESCAPE_ARMED.store(false, Ordering::SeqCst);
            SPACE_ARMED.store(false, Ordering::SeqCst);
            register_temporary_controls();
            refresh_escape_listening();
        }
        Err(error) => log::warn!("linux hotkey: cannot resolve capture command: {error}"),
    }
}

/// Called by the single-instance handoff from the sub-app capture chord
/// (`--verenu-capture-sub-app`, bound in its own managed Hyprland block).
pub fn notify_capture_sub_app() {
    if let Some(cb) = SUB_APP.get() {
        cb();
    }
}

/// Called by the Hyprland raw-key gesture watcher when it has already
/// recognized the second quick tap. Reset the Rust-side edge tracker so the
/// release of that second tap cannot stop the hands-free session it starts.
pub fn notify_handless() {
    if let Some(gesture) = GESTURE.get() {
        match gesture.lock() {
            Ok(mut gesture) => *gesture = PortalGesture::default(),
            Err(_) => log::error!("linux hotkey: gesture state lock was poisoned"),
        }
    }
    CHORD_ACTIVE.store(false, Ordering::SeqCst);
    refresh_escape_listening();
    if let Some(cb) = HANDLESS_CB.get() {
        log::info!("linux hotkey: compositor double-tap handoff toggling handsfree");
        cb();
    }
}
pub fn update_capture_keys(_k1: u32, _k2: u32) {}
pub fn reset_chord_state() {
    HANDLESS.store(false, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn set_handless_active(value: bool) {
    HANDLESS.store(value, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn begin_synthetic_paste_suppression(_duration_ms: u64) {}
pub fn set_processing_generation(generation: u64) {
    PROCESSING.store(generation, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn clear_processing_generation(expected: u64) {
    let _ = PROCESSING.compare_exchange(expected, 0, Ordering::SeqCst, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn is_win_key_down() -> bool {
    false
}
pub fn force_release_win_key() {}
pub fn caps_lock_is_on() -> bool {
    false
}

/// Lua snippet installing the bare-Escape bind that dispatches the portal
/// cancel action. Pure so the trigger text is unit-testable.
fn escape_bind_snippet(cancel_portal_id: &str) -> String {
    let key = CANCEL_KEY.lock().map(|key| key.clone()).unwrap_or_default();
    shortcuts::temporary_binding(
        "escape",
        if key.is_empty() { "ESCAPE" } else { &key },
        Some(cancel_portal_id),
        "Verenu cancel dictation (while active)",
    )
}

fn escape_unbind_snippet() -> String {
    // Disabling uses the stored handle only; temporary_binding ignores the key
    // when no portal ID is supplied, including when cancellation uses F8–F12.
    shortcuts::temporary_binding("escape", "ESCAPE", None, "")
}

fn eval_hyprland(snippet: &str) -> Result<(), String> {
    let output = std::process::Command::new("hyprctl")
        .args(["eval", snippet])
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("hyprctl eval unavailable: {e}"))?;
    (output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok")
        .then_some(())
        .ok_or_else(|| "hyprctl eval rejected the snippet".to_string())
}

/// Arms or disarms the bare-Escape cancel bind to match live dictation state.
///
/// Mirrors the macOS backend (`refresh_escape_listening` there registers a
/// Carbon Escape hotkey only while recording/hands-free/processing): Escape
/// must cancel an in-flight dictation, but a permanently installed bare-Escape
/// bind would swallow Escape in every application, all the time. Idempotent —
/// only shells out to `hyprctl` on transitions.
fn refresh_escape_listening() {
    refresh_space_listening();
    let wanted = CHORD_ACTIVE.load(Ordering::SeqCst)
        || HANDLESS.load(Ordering::SeqCst)
        || PROCESSING.load(Ordering::SeqCst) != 0;
    if wanted == ESCAPE_ARMED.load(Ordering::SeqCst) {
        return;
    }
    if wanted {
        if CANCEL_KEY.lock().map(|key| key.is_empty()).unwrap_or(true) { return; }
        let cancel_id = CANCEL_PORTAL_ID.lock().ok().and_then(|id| id.clone());
        let Some(cancel_id) = cancel_id else {
            log::debug!("linux hotkey: Escape arming deferred — cancel action not resolved yet");
            return;
        };
        match eval_hyprland(&escape_bind_snippet(&cancel_id)) {
            Ok(()) => {
                ESCAPE_ARMED.store(true, Ordering::SeqCst);
                log::info!("linux hotkey: Escape-to-cancel armed for this dictation");
            }
            Err(e) => log::warn!("linux hotkey: could not arm Escape-to-cancel: {e}"),
        }
    } else if eval_hyprland(&escape_unbind_snippet()).is_ok() {
        ESCAPE_ARMED.store(false, Ordering::SeqCst);
        log::info!("linux hotkey: Escape-to-cancel disarmed");
    } else {
        // Stay armed so the next refresh retries the unbind instead of
        // leaking a live bare-Escape bind that swallows Escape everywhere.
        log::warn!("linux hotkey: could not disarm Escape-to-cancel, will retry");
    }
}

fn refresh_space_listening() {
    let key = HANDSFREE_KEY
        .lock()
        .map(|key| key.clone())
        .unwrap_or_default();
    let wanted = CHORD_ACTIVE.load(Ordering::SeqCst)
        && !HANDLESS.load(Ordering::SeqCst)
        && !(configured_keys(&EFFECTIVE_KEYS).contains(&200) && key.eq_ignore_ascii_case("Space"));
    if wanted == SPACE_ARMED.load(Ordering::SeqCst) {
        return;
    }
    let id = HANDSFREE_PORTAL_ID.lock().ok().and_then(|id| id.clone());
    if wanted && key.is_empty() { return; }
    if wanted && id.is_none() {
        return;
    }
    let snippet = shortcuts::temporary_binding(
        "space",
        &key,
        if wanted { id.as_deref() } else { None },
        "Verenu switch to hands-free (while holding dictation)",
    );
    if eval_hyprland(&snippet).is_ok() {
        SPACE_ARMED.store(wanted, Ordering::SeqCst);
    }
}

fn register_temporary_controls() {
    // Hyprland lists disabled controls too, so Super+K explains the gestures
    // while idle without consuming those keys in other applications.
    let cancel_key = CANCEL_KEY.lock().map(|key| key.clone()).unwrap_or_default();
    let handsfree_key = HANDSFREE_KEY.lock().map(|key| key.clone()).unwrap_or_default();
    for (name, key, id, description) in [
        (
            "escape",
            cancel_key.as_str(),
            &CANCEL_PORTAL_ID,
            "Verenu cancel dictation (while active)",
        ),
        (
            "space",
            handsfree_key.as_str(),
            &HANDSFREE_PORTAL_ID,
            "Verenu switch to hands-free (while holding dictation)",
        ),
    ] {
        if key.is_empty() { continue; }
        if let Some(id) = id.lock().ok().and_then(|id| id.clone()) {
            let snippet = format!(
                "{}; {}",
                shortcuts::temporary_binding(name, key, Some(&id), description),
                shortcuts::temporary_binding(name, key, None, "")
            );
            if let Err(error) = eval_hyprland(&snippet) {
                log::warn!("linux hotkey: temporary control registration failed: {error}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start<P, R, H, C, E, L, S>(
    on_press: P,
    on_release: R,
    on_handless: H,
    on_cancel: C,
    on_escape: E,
    on_copy_last: L,
    on_capture_sub_app: S,
) -> Result<std::thread::JoinHandle<()>, String>
where
    P: Fn() + Send + Sync + 'static,
    R: Fn() + Send + Sync + 'static,
    H: Fn() + Send + Sync + 'static,
    C: Fn() + Send + Sync + 'static,
    E: Fn() + Send + Sync + 'static,
    L: Fn() + Send + Sync + 'static,
    S: Fn() + Send + Sync + 'static,
{
    let _ = PRESS.set(Box::new(on_press));
    let _ = RELEASE.set(Box::new(on_release));
    let _ = HANDLESS_CB.set(Box::new(on_handless));
    let _ = CANCEL.set(Box::new(on_cancel));
    let _ = ESCAPE.set(Box::new(on_escape));
    let _ = COPY.set(Box::new(on_copy_last));
    let _ = SUB_APP.set(Box::new(on_capture_sub_app));
    shortcut_configuration_for(&configured_keys(&KEYS)).ok_or_else(|| {
        "Linux shortcut must use one modifier plus F1–F12/Space, or two different modifiers"
            .to_string()
    })?;
    Ok(std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("portal runtime failed: {e}");
                return;
            }
        };
        runtime.block_on(async move {
            // A dropped portal stream must never kill dictation input
            // silently: an earlier version `break`ed out of the select loop,
            // ending the thread with no error surface, which stranded any
            // active recording (release could never arrive) until restart.
            loop {
                let requested = configured_keys(&KEYS);
                let available = conflicts::bindings("dictation").ok().and_then(|bindings| {
                    std::iter::once(requested.clone())
                        .chain([
                            vec![CTRL, SUPER],
                            vec![CTRL, 105],
                            vec![ALT, 106],
                            vec![CTRL, 107],
                            vec![SUPER, 108],
                            vec![ALT, 109],
                        ])
                        .find(|keys| {
                            shortcut_configuration_for(keys).is_some_and(|(_, triggers)| {
                                conflicts::free(&bindings, &triggers, false)
                            })
                        })
                });
                let Some(keys) = available else {
                    conflicts::status("dictation", shortcut_display(&requested), None, vec![]);
                    let _ = shortcuts::remove_block(
                        "-- >>> Verenu managed global shortcut (do not edit) <<<",
                        "-- <<< End Verenu managed global shortcut >>>",
                    );
                    tokio::time::sleep(PORTAL_RECONNECT_DELAY).await;
                    continue;
                };
                if let Ok(mut effective) = EFFECTIVE_KEYS.lock() {
                    *effective = keys.clone();
                }
                let Some((preferred_trigger, bindings)) = shortcut_configuration_for(&keys) else {
                    log::error!("linux hotkey: stored shortcut is no longer supported");
                    tokio::time::sleep(PORTAL_RECONNECT_DELAY).await;
                    continue;
                };
                log::info!(
                    "linux hotkey: requesting XDG portal trigger {}",
                    preferred_trigger
                        .as_deref()
                        .unwrap_or("Hyprland modifier chord")
                );
                let generation = CONFIG_GENERATION.load(Ordering::SeqCst);
                if run_portal_session(preferred_trigger.as_deref(), &bindings, generation).await {
                    log::info!("linux hotkey: portal session ended, reconnecting");
                    tokio::time::sleep(PORTAL_RECONNECT_DELAY).await;
                } else {
                    log::info!("linux hotkey: shortcut changed, rebinding");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        });
    }))
}

/// One portal session: bind, resolve compositor actions, and dispatch edges
/// until a stream ends or the saved shortcut changes. The return value tells
/// the caller whether to use the normal failure backoff or a fast rebind.
async fn run_portal_session(
    preferred_trigger: Option<&str>,
    bindings: &[String],
    generation: u64,
) -> bool {
    let portal = match GlobalShortcuts::new().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("XDG GlobalShortcuts portal unavailable: {e}");
            return true;
        }
    };
    let session = match portal.create_session(Default::default()).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("XDG GlobalShortcuts session failed: {e}");
            return true;
        }
    };
    let before = list_portal_shortcuts();
    let mut dictate_shortcut = NewShortcut::new(PORTAL_SHORTCUT_ID, PORTAL_SHORTCUT_DESCRIPTION);
    if let Some(trigger) = preferred_trigger {
        dictate_shortcut = dictate_shortcut.preferred_trigger(trigger);
    }
    let shortcuts = [
        dictate_shortcut,
        // No preferred trigger: this action exists only so the dynamic
        // bare-Escape bind has something to dispatch (see
        // `refresh_escape_listening`). Requesting no trigger means nothing
        // to collide with and no consent prompt for a key the user never
        // presses directly.
        NewShortcut::new(PORTAL_CANCEL_ID, PORTAL_CANCEL_DESCRIPTION),
        NewShortcut::new(PORTAL_CHORD_CANCEL_ID, PORTAL_CHORD_CANCEL_DESCRIPTION),
        NewShortcut::new(PORTAL_COPY_ID, PORTAL_COPY_DESCRIPTION),
        NewShortcut::new(PORTAL_HANDSFREE_ID, PORTAL_HANDSFREE_DESCRIPTION),
    ];
    match portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await
        .and_then(|r| r.response())
    {
        Ok(_) => log::info!("linux hotkey: XDG portal shortcuts bound"),
        Err(e) => {
            log::error!("XDG portal rejected Verenu shortcuts (choose another shortcut): {e}");
            return true;
        }
    }
    // Hyprland requires a compositor-side bind to dispatch the portal
    // action. Discover its exact app-scoped ID, then update only the
    // marked block in the user's Lua bindings.
    let dictate_id = match discover_portal_id(&before, PORTAL_SHORTCUT_DESCRIPTION) {
        Some(id) => id,
        None => {
            log::error!("linux hotkey: Verenu portal ID was not visible in Hyprland");
            return true;
        }
    };
    log::info!("linux hotkey: discovered portal ID");
    // The managed Lua block references both dictation edge actions; the
    // cancel action is dispatched through runtime-evaluated binds that resolve
    // its id below on every arm. A reconnected session therefore needs the
    // managed block re-ensured so both dictation handles resolve to the live
    // portal session.
    let release_command = match std::env::current_exe() {
        Ok(exe) => format!(
            "{} --verenu-hotkey-release",
            shell_quote(&exe.to_string_lossy())
        ),
        Err(error) => {
            log::error!("linux hotkey: cannot resolve release command: {error}");
            return true;
        }
    };
    let handsfree_command = match std::env::current_exe() {
        Ok(exe) => format!(
            "{} --verenu-hotkey-handsfree",
            shell_quote(&exe.to_string_lossy())
        ),
        Err(error) => {
            log::error!("linux hotkey: cannot resolve handsfree command: {error}");
            return true;
        }
    };
    if let Err(error) = crate::core::hyprland::ensure_global_shortcut_binding(
        bindings,
        &dictate_id,
        &release_command,
        &handsfree_command,
        &release_keycodes_for(&configured_keys(&EFFECTIVE_KEYS)),
    ) {
        log::error!("linux hotkey: Hyprland binding setup failed: {error}");
        return true;
    }
    log::info!("linux hotkey: Hyprland binding installed");
    let keys = configured_keys(&EFFECTIVE_KEYS);
    conflicts::status(
        "dictation",
        shortcut_display(&configured_keys(&KEYS)),
        Some(shortcut_display(&keys)),
        keys.iter().map(|id| shortcut_code(*id)).collect(),
    );
    install_sub_app_capture_binding();
    match discover_portal_id(&before, PORTAL_CANCEL_DESCRIPTION) {
        Some(id) => {
            if let Ok(mut slot) = CANCEL_PORTAL_ID.lock() {
                *slot = Some(id);
            }
        }
        None => log::warn!(
            "linux hotkey: cancel portal action not visible — Escape-to-cancel unavailable this session"
        ),
    }
    if let Some(id) = discover_portal_id(&before, PORTAL_HANDSFREE_DESCRIPTION) {
        if let Ok(mut slot) = HANDSFREE_PORTAL_ID.lock() {
            *slot = Some(id);
        }
    }
    match discover_portal_id(&before, PORTAL_COPY_DESCRIPTION) {
        Some(id) => {
            let active = conflicts::bindings("copy").ok().and_then(|bindings| conflicts::choose(
                &bindings, "Ctrl+Alt+C", &["Ctrl+Alt+F6", "Ctrl+Shift+F6", "Alt+Super+F6"], false,
            ));
            if let Some(active) = active {
                if let Err(error) = shortcuts::ensure_copy_binding(&id, &active) {
                    log::warn!("linux hotkey: copy shortcut registration failed: {error}");
                    conflicts::status("copy", "Ctrl+Alt+C".into(), None, vec![]);
                } else {
                    conflicts::status("copy", "Ctrl+Alt+C".into(), Some(active), vec![]);
                }
            } else {
                let _ = shortcuts::remove_block(shortcuts::START, shortcuts::END);
                conflicts::status("copy", "Ctrl+Alt+C".into(), None, vec![]);
            }
        }
        None => log::warn!("linux hotkey: copy portal action is unavailable"),
    }
    // These controls ignore held modifiers, so reserve the key across every
    // modifier mask and submap before enabling it during dictation.
    for (id, requested, alternatives, key_slot) in [
        ("cancel", "Escape", &["F8", "F9", "F10", "F11", "F12"][..], &CANCEL_KEY),
        ("handsfree", "Space", &["F9", "F10", "F11", "F12", "F8"][..], &HANDSFREE_KEY),
    ] {
        let already_chosen = CANCEL_KEY.lock().map(|key| key.clone()).unwrap_or_default();
        let choices = alternatives.iter().copied().filter(|key| id != "handsfree" || !key.eq_ignore_ascii_case(&already_chosen)).collect::<Vec<_>>();
        let active = conflicts::bindings(id).ok().and_then(|bindings| conflicts::choose(&bindings, requested, &choices, true));
        if let Ok(mut key) = key_slot.lock() { *key = active.clone().unwrap_or_default(); }
        conflicts::status(id, requested.into(), active, vec![]);
    }
    // Config reloads remove runtime handles. Re-arm against this session.
    ESCAPE_ARMED.store(false, Ordering::SeqCst);
    SPACE_ARMED.store(false, Ordering::SeqCst);
    register_temporary_controls();
    refresh_escape_listening();
    let mut activated = match portal.receive_activated().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("portal activation stream failed: {e}");
            return true;
        }
    };
    // Keep the portal edge classifier as a fallback for normal chord events.
    // The Hyprland raw-key block recognizes the second quick tap before this
    // portal stream can be delayed by the single-instance handoff, then calls
    // `notify_handless` directly. This classifier still filters repeated
    // portal activations and consumes a portal-delivered second release when
    // a compositor sends the events in the slower order.
    let gesture = GESTURE.get_or_init(|| Mutex::new(PortalGesture::default()));
    if let Ok(mut state) = gesture.lock() {
        *state = PortalGesture::default();
    }
    let mut configuration_poll = tokio::time::interval(Duration::from_millis(100));
    let mut desktop_check = Instant::now();
    loop {
        tokio::select! {
            _ = configuration_poll.tick() => {
                flush_pending_release(Instant::now());
                let mut desktop_changed = false;
                if desktop_check.elapsed() >= Duration::from_secs(5)
                    && !CHORD_ACTIVE.load(Ordering::SeqCst)
                    && !HANDLESS.load(Ordering::SeqCst)
                    && PROCESSING.load(Ordering::SeqCst) == 0
                {
                    desktop_check = Instant::now();
                    desktop_changed = shortcuts_need_rebind();
                    if !desktop_changed {
                        // A user config reload clears runtime Lua handles.
                        // Restore the idle controls and their menu entries.
                        register_temporary_controls();
                    }
                }
                if CONFIG_GENERATION.load(Ordering::SeqCst) != generation || desktop_changed {
                    let was_holding = gesture
                        .lock()
                        .map(|state| state.chord_active)
                        .unwrap_or(false);
                    CHORD_ACTIVE.store(false, Ordering::SeqCst);
                    refresh_escape_listening();
                    if was_holding {
                        if let Some(cb) = RELEASE.get() {
                            cb();
                        }
                    }
                    return false;
                }
            },
            Some(event) = activated.next() => {
                if super::capture_active() { continue; }
                let id = event.shortcut_id();
                if id == PORTAL_SHORTCUT_ID {
                    flush_pending_release(Instant::now());
                    let Ok(mut gesture) = gesture.lock() else {
                        log::error!("linux hotkey: gesture state lock was poisoned");
                        continue;
                    };
                    let action = gesture.activated(Instant::now());
                    drop(gesture);
                    match action {
                        Some(PortalGestureAction::Press) => {
                            // Info-level: one line per dictation start is the
                            // cheapest reliable trace for stuck-hold reports.
                            log::info!("linux hotkey: portal press");
                            CHORD_ACTIVE.store(true, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = PRESS.get() { cb(); }
                        }
                        Some(PortalGestureAction::Handsfree) => {
                            log::info!("linux hotkey: portal double-tap, toggling handsfree");
                            // The second tap's release is consumed by the
                            // gesture tracker; it must not leave a held chord
                            // behind after hands-free stops.
                            CHORD_ACTIVE.store(false, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = HANDLESS_CB.get() { cb(); }
                        }
                        _ => log::debug!("linux hotkey: ignored duplicate portal activation"),
                    }
                } else if id == PORTAL_CANCEL_ID {
                    log::info!("linux hotkey: portal Escape-to-cancel fired");
                    if let Some(cb) = ESCAPE.get() { cb(); }
                } else if id == PORTAL_CHORD_CANCEL_ID {
                    // The compositor rejects modifier prefixes of other
                    // shortcuts. It must never cancel hands-free or processing.
                    if should_cancel_hold(CHORD_ACTIVE.load(Ordering::SeqCst), HANDLESS.load(Ordering::SeqCst)) {
                        log::info!("linux hotkey: discarding hold used by another shortcut");
                        if let Some(cb) = CHORD_CANCEL.get() { cb(); }
                    }
                } else if id == PORTAL_COPY_ID {
                    if let Some(cb) = COPY.get() { cb(); }
                } else if id == PORTAL_HANDSFREE_ID {
                    notify_handless();
                }
            },
            else => {
                log::error!("linux hotkey: portal event streams ended — reconnecting");
                CHORD_ACTIVE.store(false, Ordering::SeqCst);
                refresh_escape_listening();
                return true;
            },
        }
    }
}

fn shortcuts_need_rebind() -> bool {
    super::shortcut_status::get_shortcut_status()
        .iter()
        .any(|status| {
            let Ok(bindings) = conflicts::bindings(&status.id) else {
                return false;
            };
            let ignore_mods = matches!(status.id.as_str(), "cancel" | "handsfree");
            let active_triggers = if status.id == "dictation" {
                shortcut_configuration_for(&configured_keys(&EFFECTIVE_KEYS))
                    .map(|(_, triggers)| triggers)
                    .unwrap_or_default()
            } else {
                status.active.iter().cloned().collect()
            };
            let requested_triggers = if status.id == "dictation" {
                shortcut_configuration_for(&configured_keys(&KEYS))
                    .map(|(_, triggers)| triggers)
                    .unwrap_or_default()
            } else {
                vec![status.requested.clone()]
            };
            let Some(active) = &status.active else {
                if conflicts::free(&bindings, &requested_triggers, ignore_mods) {
                    return true;
                }
                return match status.id.as_str() {
                    "dictation" => [
                        (CTRL, SUPER),
                        (CTRL, 105),
                        (ALT, 106),
                        (CTRL, 107),
                        (SUPER, 108),
                        (ALT, 109),
                    ]
                    .iter()
                    .any(|(first, second)| {
                        shortcut_configuration_for(&[*first, *second]).is_some_and(
                            |(_, triggers)| conflicts::free(&bindings, &triggers, false),
                        )
                    }),
                    "copy" => conflicts::choose(
                        &bindings,
                        &status.requested,
                        &["Ctrl+Alt+F6", "Ctrl+Shift+F6", "Alt+Super+F6"],
                        false,
                    )
                    .is_some(),
                    "capture" => conflicts::choose(
                        &bindings,
                        &status.requested,
                        &["Ctrl+Alt+Shift+F7", "Ctrl+Alt+Shift+F8", "Alt+Super+F7"],
                        false,
                    )
                    .is_some(),
                    _ => conflicts::choose(
                        &bindings,
                        &status.requested,
                        &["F8", "F9", "F10", "F11", "F12"],
                        true,
                    )
                    .is_some(),
                };
            };
            !conflicts::free(&bindings, &active_triggers, ignore_mods)
                || (active != &status.requested
                    && conflicts::free(&bindings, &requested_triggers, ignore_mods))
        })
}

#[cfg(target_os = "linux")]
fn list_portal_shortcuts() -> Vec<(String, String)> {
    let output = std::process::Command::new("hyprctl")
        .arg("globalshortcuts")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    parse_portal_shortcuts(&String::from_utf8_lossy(&output.stdout))
}

fn parse_portal_shortcuts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (id, description) = line.split_once(" -> ")?;
            let id = id.trim();
            let description = description.trim();
            (!id.is_empty() && !description.is_empty())
                .then(|| (id.to_string(), description.to_string()))
        })
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Prefer a shortcut that appeared after we bound, with Verenu's description.
/// Never grab another app's leftover "Hold to dictate" ID just because it is
/// listed first — that is how Ctrl+Space started dispatching a dead t3code
/// action while the live portal sat unused.
fn discover_portal_id(before: &[(String, String)], description: &str) -> Option<String> {
    let after = list_portal_shortcuts();
    pick_portal_id(before, &after, description)
}

fn pick_portal_id(
    before: &[(String, String)],
    after: &[(String, String)],
    description: &str,
) -> Option<String> {
    let before_ids: Vec<&str> = before.iter().map(|(id, _)| id.as_str()).collect();
    let matching: Vec<&(String, String)> = after
        .iter()
        .filter(|(_, candidate)| candidate == description)
        .collect();
    matching
        .iter()
        .find(|(id, _)| !before_ids.contains(&id.as_str()))
        .or_else(|| matching.last())
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn multi_modifier_shortcuts_keep_every_key_and_release_code() {
        let keys = [
            super::CTRL,
            super::ALT,
            super::SHIFT,
            super::SUPER,
            super::map_code_to_vk("KeyK"),
        ];
        let (preferred, bindings) = super::shortcut_configuration_for(&keys).unwrap();
        assert_eq!(preferred.as_deref(), Some("CTRL+ALT+SHIFT+SUPER+K"));
        assert_eq!(bindings, ["CTRL + ALT + SHIFT + SUPER + K"]);
        assert_eq!(
            super::release_keycodes_for(&keys),
            [37, 105, 64, 108, 50, 62, 133, 134, 45]
        );
        assert_eq!(super::shortcut_code(keys[4]), "KeyK");
        let (_, modifiers) = super::shortcut_configuration_for(&keys[..4]).unwrap();
        assert_eq!(modifiers.len(), 7);
        assert!(modifiers.contains(&"CTRL + ALT + SUPER + Shift_R".to_string()));
        assert!(super::shortcut_configuration_for(&[
            super::map_code_to_vk("KeyA"),
            super::map_code_to_vk("KeyB")
        ])
        .is_none());
        assert!(super::shortcut_configuration_for(&[105]).is_some());
        assert!(super::shortcut_configuration_for(&[200]).is_none());
    }
    use super::{
        escape_bind_snippet, escape_unbind_snippet, parse_portal_shortcuts, pick_portal_id,
        shortcut_configuration, should_cancel_hold, PortalGesture, PortalGestureAction,
        PORTAL_CANCEL_DESCRIPTION, PORTAL_SHORTCUT_DESCRIPTION,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn hold_starts_on_press_and_processes_on_release() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(1000)),
            Some(PortalGestureAction::Release)
        );
    }

    #[test]
    fn quick_double_tap_enters_handsfree_and_consumes_second_release() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(gesture.deactivated(start + Duration::from_millis(80)), None);
        assert_eq!(
            gesture.activated(start + Duration::from_millis(180)),
            Some(PortalGestureAction::Handsfree)
        );
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(240)),
            None
        );
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn repeated_activation_during_one_hold_cannot_trigger_handsfree() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(gesture.activated(start + Duration::from_millis(100)), None);
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(1000)),
            Some(PortalGestureAction::Release)
        );
    }

    #[test]
    fn second_tap_after_window_is_a_fresh_hold() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(gesture.deactivated(start + Duration::from_millis(80)), None);
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_millis(431)),
            Some(PortalGestureAction::Cancel)
        );
        assert_eq!(
            gesture.activated(start + Duration::from_millis(500)),
            Some(PortalGestureAction::Press)
        );
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn unmatched_deactivation_is_ignored() {
        let mut gesture = PortalGesture::default();
        assert_eq!(gesture.deactivated(Instant::now()), None);
    }

    #[test]
    fn logged_double_tap_timings_keep_one_capture_without_release_or_cancel() {
        // First hold and gap from the reported failures, plus slower taps.
        for (held, gap) in [(312, 163), (311, 337), (448, 200), (699, 349)] {
            let start = Instant::now();
            let mut gesture = PortalGesture::default();
            assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
            let released = start + Duration::from_millis(held);
            assert_eq!(gesture.deactivated(released), None);
            assert_eq!(
                gesture.take_pending_release(released + Duration::from_millis(gap)),
                None
            );
            assert_eq!(
                gesture.activated(released + Duration::from_millis(gap)),
                Some(PortalGestureAction::Handsfree)
            );
            assert_eq!(
                gesture.deactivated(released + Duration::from_millis(gap + 80)),
                None
            );
            assert_eq!(
                gesture.take_pending_release(start + Duration::from_secs(3)),
                None
            );
        }
    }

    #[test]
    fn lone_tap_discards_once_only_after_the_second_tap_window() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();
        gesture.activated(start);
        assert_eq!(gesture.deactivated(start + Duration::from_millis(80)), None);
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_millis(429)),
            None
        );
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_millis(430)),
            Some(PortalGestureAction::Cancel)
        );
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_millis(500)),
            None
        );
    }

    #[test]
    fn compositor_handoff_retires_first_tap_and_late_second_release() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();
        gesture.activated(start);
        gesture.deactivated(start + Duration::from_millis(312));
        // notify_handless resets the tracker before promoting the capture.
        gesture = PortalGesture::default();
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(600)),
            None
        );
        assert_eq!(
            gesture.take_pending_release(start + Duration::from_secs(2)),
            None
        );
    }

    #[test]
    fn modifier_prefix_rejection_never_cancels_handsfree_or_processing() {
        assert!(should_cancel_hold(true, false));
        assert!(!should_cancel_hold(true, true));
        assert!(!should_cancel_hold(false, false));
        assert!(!should_cancel_hold(false, true));
    }

    #[test]
    fn repeated_activation_during_a_long_hold_keeps_the_original_hold_clock() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(gesture.activated(start + Duration::from_secs(3)), None);
        assert_eq!(
            gesture.deactivated(start + Duration::from_secs(3) + Duration::from_millis(400)),
            Some(PortalGestureAction::Release)
        );
    }

    #[test]
    fn modifier_only_ctrl_super_uses_both_press_orders_without_a_portal_preference() {
        let (preferred, bindings) = shortcut_configuration(1, 4).expect("Ctrl+Super");

        assert_eq!(preferred, None);
        assert_eq!(
            bindings,
            vec![
                "CTRL + Super_L".to_string(),
                "SUPER + Control_L".to_string(),
                "SUPER + Control_R".to_string(),
            ]
        );
    }

    #[test]
    fn releasing_either_side_of_either_chord_key_finishes_the_hold() {
        assert_eq!(super::release_keycodes(1, 4), vec![37, 105, 133, 134]);
        assert_eq!(super::release_keycodes(1, 105), vec![37, 105, 71]);
        for first in 1..=4 {
            for second in 1..=4 {
                assert_eq!(
                    super::shortcut_configuration(first, second).is_some(),
                    first != second
                );
            }
            for key in (101..=112).chain(std::iter::once(200)) {
                assert!(super::shortcut_configuration(first, key).is_some());
                assert_eq!(super::release_keycodes(first, key).len(), 3);
            }
        }
    }

    #[test]
    fn portal_id_prefers_the_new_verenu_description_over_a_stale_hold_to_dictate() {
        let before = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:capture-window -> Capture a window\n",
        );
        let after = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:capture-window -> Capture a window\ncom.t3tools.T3Code:dictate -> Verenu dictation\n",
        );

        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:dictate")
        );
        assert_eq!(PORTAL_SHORTCUT_DESCRIPTION, "Verenu dictation");
    }

    #[test]
    fn portal_id_does_not_grab_the_first_hold_to_dictate_leftover() {
        let listing = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:dictate -> Hold to dictate\n",
        );
        assert_eq!(
            pick_portal_id(&listing, &listing, PORTAL_SHORTCUT_DESCRIPTION),
            None
        );
    }

    #[test]
    fn cancel_action_is_discovered_independently_of_the_dictate_action() {
        let before = parse_portal_shortcuts("com.t3tools.T3Code:dictate -> Verenu dictation\n");
        let after = parse_portal_shortcuts(
            "com.t3tools.T3Code:dictate -> Verenu dictation\ncom.t3tools.T3Code:cancel -> Verenu cancel\n",
        );

        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_CANCEL_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:cancel")
        );
        // The dictate lookup must not grab the cancel action and vice versa.
        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:dictate")
        );
        assert_eq!(PORTAL_CANCEL_DESCRIPTION, "Verenu cancel");
    }

    #[test]
    fn escape_bind_targets_the_cancel_action_and_unbind_clears_bare_escape() {
        let snippet = escape_bind_snippet("com.t3tools.T3Code:cancel");
        assert!(snippet.contains("hl.bind(\"ESCAPE\""));
        assert!(snippet.contains("hl.dsp.global(\"com.t3tools.T3Code:cancel\")"));
        assert!(snippet.contains("ignore_mods = true"));
        assert!(snippet.contains("submap_universal = true"));
        assert!(escape_unbind_snippet().contains("_verenu_escape_binding:set_enabled(false)"));
        assert!(!escape_unbind_snippet().contains("hl.unbind"));
    }
}
