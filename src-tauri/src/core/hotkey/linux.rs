use ashpd::desktop::global_shortcuts::NewShortcut;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[path = "linux_conflicts.rs"]
mod conflicts;
#[path = "linux_portal.rs"]
mod portal;
#[path = "linux_shortcuts.rs"]
mod shortcuts;
#[cfg(test)]
#[path = "linux_secondary_tests.rs"]
mod secondary_tests;

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
static CONTROL_KEYS: Mutex<Option<[String; 2]>> = Mutex::new(None);

fn control_trigger(code: &str) -> Result<String, String> {
    if !super::is_control_key(code) {
        return Err("Choose a single key without modifiers".into());
    }
    let key = map_code_to_vk(code);
    match key {
        200 => Ok("Space".into()),
        101..=112 => Ok(format!("F{}", key - 100)),
        _ => REGULAR_KEYS.iter().find(|(_, _, id)| 1000 + id == key)
            .map(|(_, name, _)| (*name).into())
            .ok_or_else(|| "This key is not supported on Linux".into()),
    }
}

#[cfg(test)]
mod control_key_tests {
    #[test]
    fn saved_control_codes_resolve_to_compositor_keysyms() {
        for (code, trigger) in [("Escape", "Escape"), ("Space", "Space"), ("F8", "F8"),
            ("KeyK", "K"), ("ArrowUp", "Up"), ("Quote", "apostrophe")] {
            assert_eq!(super::control_trigger(code).unwrap(), trigger);
        }
        for code in ["ControlLeft", "MetaRight", "Fn", "Ctrl+F8", "", "Unknown"] {
            assert!(super::control_trigger(code).is_err());
        }
    }
}

pub fn update_control_keys(cancel: &str, handsfree: &str) -> Result<(), String> {
    let keys = [control_trigger(cancel)?, control_trigger(handsfree)?];
    let mut requested = CONTROL_KEYS.lock().map_err(|_| "Shortcut state unavailable")?;
    if requested.as_ref() != Some(&keys) {
        *requested = Some(keys);
        CONFIG_GENERATION.fetch_add(1, Ordering::SeqCst);
    }
    Ok(())
}

fn requested_control_keys() -> [String; 2] {
    CONTROL_KEYS.lock().ok().and_then(|keys| keys.clone())
        .unwrap_or_else(|| ["Escape".into(), "Space".into()])
}
static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(0);
static HANDLESS: AtomicBool = AtomicBool::new(false);
static PROCESSING: AtomicU64 = AtomicU64::new(0);
static CHORD_ACTIVE: AtomicBool = AtomicBool::new(false);
static ESCAPE_ARMED: AtomicBool = AtomicBool::new(false);
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
const TAP_MAX_HOLD: Duration = Duration::from_millis(700);
const STALE_CHORD_TIMEOUT: Duration = Duration::from_secs(2);
const PORTAL_SHORTCUT_ID: &str = "dictate";
const PORTAL_SHORTCUT_DESCRIPTION: &str = "Verenu dictation";
const PORTAL_CANCEL_ID: &str = "cancel";
const PORTAL_CANCEL_DESCRIPTION: &str = "Verenu cancel";
const PORTAL_CHORD_CANCEL_ID: &str = "cancel-chord";
const PORTAL_CHORD_CANCEL_DESCRIPTION: &str = "Verenu discard hold gesture";
const PORTAL_COPY_ID: &str = "copy-last";
const PORTAL_COPY_DESCRIPTION: &str = "Verenu copy last dictation";
const PORTAL_HANDSFREE_ID: &str = "handsfree";
const PORTAL_HANDSFREE_DESCRIPTION: &str = "Verenu hands-free";
const PORTAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortalGestureAction {
    Press,
    Release,
    Handsfree,
    Cancel,
}

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
        101..=110 => Some(key - 34),
        111..=112 => Some(key - 16),
        200 => Some(65),
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

fn shortcut_triggers(keys: &[String]) -> Result<Vec<String>, String> {
    let ids = super::mapped_codes(keys)?;
    let (_, triggers) = shortcut_configuration_for(&ids)
        .ok_or("Use modifiers plus one regular key, a function key, or a modifier-only combination on Linux")?;
    Ok(triggers)
}

pub fn validate_hotkey(keys: &[String]) -> Result<bool, String> {
    shortcut_triggers(keys).map(|_| true)
}

pub fn is_hotkey_available(keys: &[String]) -> Result<bool, String> {
    let triggers = shortcut_triggers(keys)?;
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
    eval_hyprland(&suspend_shortcuts_snippet(active))
}

fn install_with_capture_suspended(work: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    super::with_capture_transition_lock(|| {
        maintain_capture_suspension(super::capture_active(), work, || suspend_shortcuts(true))
    })
}

fn maintain_capture_suspension(
    active: bool,
    work: impl FnOnce() -> Result<(), String>,
    suspend: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let result = work();
    if active { suspend()?; }
    result
}

#[cfg(test)]
mod capture_binding_tests {
    use super::*;
    #[test]
    fn reconnect_reapplies_capture_suspension_even_if_install_fails() {
        let calls = std::cell::Cell::new(0);
        let result = maintain_capture_suspension(true, || Err("install failed".into()), || {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
        maintain_capture_suspension(false, || Ok(()), || {
            calls.set(calls.get() + 1);
            Ok(())
        }).unwrap();
        assert_eq!(calls.get(), 1);
    }
}

fn suspend_shortcuts_snippet(active: bool) -> String {
    let enabled = if active { "false" } else { "true" };
    format!(
        "if _verenu_dictation_bindings then for _, handle in ipairs(_verenu_dictation_bindings) do if tostring(handle) ~= 'HL.Keybind(expired)' then handle:set_enabled({enabled}) end end end; for _, handle in pairs({{_verenu_copy_binding, _verenu_capture_binding}}) do if tostring(handle) ~= 'HL.Keybind(expired)' then handle:set_enabled({enabled}) end end"
    )
}

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

pub fn initialize_sub_app_capture_chord(chord: super::chord::Chord) -> Result<(), String> {
    initialize_sub_app_chord(&SUB_APP_CHORD, chord, || install_sub_app_capture_binding(false))
}

fn initialize_sub_app_chord(
    slot: &Mutex<Option<super::chord::Chord>>,
    chord: super::chord::Chord,
    install: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    *slot.lock().map_err(|_| "Shortcut state unavailable")? = Some(chord);
    install()
}

#[cfg(test)]
mod sub_app_startup_tests {
    use super::*;

    #[test]
    fn occupied_saved_shortcut_keeps_requested_chord_while_using_fallback() {
        let requested = super::super::chord::Chord::parse("Ctrl+Alt+F6").unwrap();
        let slot = Mutex::new(None);
        let occupied = vec![conflicts::Binding { modmask: 12, key: "F6".into(), ..Default::default() }];
        initialize_sub_app_chord(&slot, requested, || {
            let saved = slot.lock().unwrap().unwrap().to_string();
            assert_eq!(saved, requested.to_string());
            assert_eq!(conflicts::choose(&occupied, &saved, &["Ctrl+Alt+Shift+F7"], false).as_deref(), Some("Ctrl+Alt+Shift+F7"));
            Ok(())
        }).unwrap();
        assert_eq!(slot.lock().unwrap().unwrap().to_string(), requested.to_string());
    }
}

pub fn set_sub_app_capture_chord(chord: super::chord::Chord) -> Result<(), String> {
    let previous = {
        let mut slot = SUB_APP_CHORD.lock().map_err(|_| "Shortcut state unavailable")?;
        let previous = *slot;
        *slot = Some(chord);
        previous
    };
    if let Err(error) = install_sub_app_capture_binding(true) {
        *SUB_APP_CHORD.lock().map_err(|_| "Shortcut state unavailable")? = previous;
        if let Err(rollback) = install_sub_app_capture_binding(false) {
            return Err(format!("{error}; shortcut rollback failed: {rollback}"));
        }
        return Err(error);
    }
    Ok(())
}

fn install_sub_app_capture_binding(require_exact: bool) -> Result<(), String> {
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
        if require_exact { return Err("Sub-app shortcut is unavailable".into()); }
        let _ = shortcuts::remove_block(
            "-- >>> Verenu sub-app capture (do not edit) <<<",
            "-- <<< End Verenu sub-app capture >>>",
        );
        conflicts::status("capture", requested, None, vec![]);
        return Ok(());
    };
    if require_exact && active != requested { return Err("Sub-app shortcut is already assigned".into()); }
    let Some(chord) = super::chord::Chord::parse(&active) else {
        return Err("Invalid sub-app shortcut".into());
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
                publish_shortcut_failure(
                    "capture",
                    &requested,
                    portal::Failure::new("Installing sub-app shortcut", error.clone()),
                );
                return Err(error);
            }
            conflicts::status("capture", requested, Some(active), vec![]);
            ESCAPE_ARMED.store(false, Ordering::SeqCst);
            SPACE_ARMED.store(false, Ordering::SeqCst);
            register_temporary_controls();
            refresh_escape_listening();
        }
        Err(error) => {
            publish_shortcut_failure("capture", &requested, portal::Failure::new("Preparing sub-app shortcut", error.to_string()));
            return Err(error.to_string());
        }
    }
    Ok(())
}

pub fn notify_capture_sub_app() {
    if let Some(cb) = SUB_APP.get() {
        cb();
    }
}

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
pub fn reset_chord_state() {
    HANDLESS.store(false, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn set_handless_active(value: bool) {
    HANDLESS.store(value, Ordering::SeqCst);
    refresh_escape_listening();
}
#[allow(dead_code)]
pub fn begin_synthetic_paste_suppression(_duration_ms: u64) {}
pub fn set_processing_generation(generation: u64) {
    PROCESSING.store(generation, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn clear_processing_generation(expected: u64) {
    let _ = PROCESSING.compare_exchange(expected, 0, Ordering::SeqCst, Ordering::SeqCst);
    refresh_escape_listening();
}
#[allow(dead_code)]
pub fn is_win_key_down() -> bool {
    false
}
#[allow(dead_code)]
pub fn force_release_win_key() {}
pub fn caps_lock_is_on() -> bool {
    crate::core::hyprland::caps_lock_is_on().unwrap_or(false)
}

fn escape_bind_snippet(cancel_portal_id: &str) -> String {
    let key = CANCEL_KEY.lock().map(|key| key.clone()).unwrap_or_default();
    control_binding(
        "escape",
        if key.is_empty() { "ESCAPE" } else { &key },
        Some(cancel_portal_id),
        "Verenu cancel dictation (while active)",
        "cancel",
    )
}

fn escape_unbind_snippet() -> String {
    control_binding("escape", "ESCAPE", None, "", "cancel")
}

fn control_binding(name: &str, key: &str, id: Option<&str>, description: &str, action: &str) -> String {
    let bindings = if id.is_some() { conflicts::bindings(action).unwrap_or_else(|_| vec![conflicts::Binding { catch_all: true, ignore_mods: true, ..Default::default() }]) } else { vec![] };
    shortcuts::temporary_controls(name, key, id, description, &bindings)
}

fn eval_hyprland(snippet: &str) -> Result<(), String> {
    if let Some(response) = crate::core::hyprland::socket_request(&format!("/eval {snippet}")) {
        return response
            .ok()
            .filter(|reply| String::from_utf8_lossy(reply).trim() == "ok")
            .map(|_| ())
            .ok_or_else(|| "hyprctl eval rejected the snippet".to_string());
    }
    let output = std::process::Command::new("hyprctl")
        .args(["eval", snippet])
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("hyprctl eval unavailable: {e}"))?;
    (output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok")
        .then_some(())
        .ok_or_else(|| "hyprctl eval rejected the snippet".to_string())
}

fn refresh_escape_listening() {
    refresh_space_listening();
    let available = CANCEL_KEY.lock().map(|key| !key.is_empty()).unwrap_or(false)
        && CANCEL_PORTAL_ID.lock().map(|id| id.is_some()).unwrap_or(false);
    let wanted = !super::capture_active() && available && (CHORD_ACTIVE.load(Ordering::SeqCst)
        || HANDLESS.load(Ordering::SeqCst)
        || PROCESSING.load(Ordering::SeqCst) != 0);
    if wanted == ESCAPE_ARMED.load(Ordering::SeqCst) {
        return;
    }
    if wanted {
        if CANCEL_KEY.lock().map(|key| key.is_empty()).unwrap_or(true) {
            return;
        }
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
        log::warn!("linux hotkey: could not disarm Escape-to-cancel, will retry");
    }
}

fn refresh_space_listening() {
    let key = HANDSFREE_KEY
        .lock()
        .map(|key| key.clone())
        .unwrap_or_default();
    let id = HANDSFREE_PORTAL_ID.lock().ok().and_then(|id| id.clone());
    let wanted = !super::capture_active() && !key.is_empty() && id.is_some() && CHORD_ACTIVE.load(Ordering::SeqCst)
        && !HANDLESS.load(Ordering::SeqCst)
        && !(configured_keys(&EFFECTIVE_KEYS).contains(&200) && key.eq_ignore_ascii_case("Space"));
    if wanted == SPACE_ARMED.load(Ordering::SeqCst) {
        return;
    }
    if wanted && key.is_empty() {
        return;
    }
    if wanted && id.is_none() {
        return;
    }
    let snippet = control_binding(
        "space",
        &key,
        if wanted { id.as_deref() } else { None },
        "Verenu switch to hands-free (while holding dictation)",
        "handsfree",
    );
    if eval_hyprland(&snippet).is_ok() {
        SPACE_ARMED.store(wanted, Ordering::SeqCst);
    }
}

fn register_temporary_controls() {
    let cancel_key = CANCEL_KEY.lock().map(|key| key.clone()).unwrap_or_default();
    let handsfree_key = HANDSFREE_KEY
        .lock()
        .map(|key| key.clone())
        .unwrap_or_default();
    let [cancel, handsfree] = requested_control_keys();
    for (control, requested, name, key, id, armed, description) in [
        (
            "cancel",
            cancel.as_str(),
            "escape",
            cancel_key.as_str(),
            &CANCEL_PORTAL_ID,
            &ESCAPE_ARMED,
            "Verenu cancel dictation (while active)",
        ),
        (
            "handsfree",
            handsfree.as_str(),
            "space",
            handsfree_key.as_str(),
            &HANDSFREE_PORTAL_ID,
            &SPACE_ARMED,
            "Verenu switch to hands-free (while holding dictation)",
        ),
    ] {
        let id = id.lock().ok().and_then(|id| id.clone());
        let active_id = id.filter(|_| !key.is_empty());
        let snippet = match active_id.as_deref() {
            Some(id) => format!(
                "{}; {}",
                control_binding(name, key, Some(id), description, control),
                control_binding(name, key, None, "", control)
            ),
            None => control_binding(name, key, None, "", control),
        };
        match eval_hyprland(&snippet) {
            Ok(()) => {
                armed.store(false, Ordering::SeqCst);
                conflicts::status(control, requested.into(), active_id.map(|_| key.into()), vec![]);
            }
            Err(error) => publish_shortcut_failure(control, requested,
                portal::Failure::new("Installing secondary shortcut", error)),
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
    if shortcut_configuration_for(&configured_keys(&KEYS)).is_none() {
        let reason = "The saved key combination is not supported on Linux. Choose another dictation hotkey in Settings > General.";
        registration_failure(portal::Failure::new("Reading dictation shortcut", reason));
        return Err(reason.into());
    }
    Ok(std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(v) => v,
            Err(e) => {
                registration_failure(portal::Failure::new(
                    "Starting shortcut listener",
                    e.to_string(),
                ));
                return;
            }
        };
        runtime.block_on(async move {
            loop {
                let requested = configured_keys(&KEYS);
                let desktop_bindings = match conflicts::bindings("dictation") {
                    Ok(bindings) => bindings,
                    Err(error) => {
                        registration_failure(portal::Failure::new("Checking desktop shortcuts", error));
                        tokio::time::sleep(PORTAL_RECONNECT_DELAY).await;
                        continue;
                    }
                };
                let available = std::iter::once(requested.clone())
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
                                conflicts::free(&desktop_bindings, &triggers, false)
                            })
                        });
                let Some(keys) = available else {
                    registration_failure(portal::Failure::new("Finding available shortcut", "The saved shortcut and every fallback are already assigned on this desktop"));
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
                    registration_failure(portal::Failure::new("Reading dictation shortcut", "The saved key combination is not supported on Linux. Choose another dictation hotkey in Settings > General."));
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

async fn run_portal_session(
    preferred_trigger: Option<&str>,
    bindings: &[String],
    generation: u64,
) -> bool {
    let portal = match portal::connect().await {
        Ok(v) => v,
        Err(e) => {
            registration_failure(e);
            return true;
        }
    };
    let mut owner_changes = match portal::request(
        "Watching the desktop portal",
        portal.receive_owner_changed(),
    )
    .await
    {
        Ok(stream) => stream,
        Err(error) => {
            registration_failure(error);
            return true;
        }
    };
    let session = match portal::request(
        "Creating shortcut session",
        portal.create_session(Default::default()),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            registration_failure(e);
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
        NewShortcut::new(PORTAL_CANCEL_ID, PORTAL_CANCEL_DESCRIPTION),
        NewShortcut::new(PORTAL_CHORD_CANCEL_ID, PORTAL_CHORD_CANCEL_DESCRIPTION),
        NewShortcut::new(PORTAL_COPY_ID, PORTAL_COPY_DESCRIPTION),
        NewShortcut::new(PORTAL_HANDSFREE_ID, PORTAL_HANDSFREE_DESCRIPTION),
    ];
    match portal::request("Binding shortcuts", async {
        portal
            .bind_shortcuts(&session, &shortcuts, None, Default::default())
            .await?
            .response()
    })
    .await
    {
        Ok(_) => log::info!("linux hotkey: XDG portal shortcuts bound"),
        Err(e) => {
            registration_failure(e);
            return true;
        }
    }
    // Hyprland requires a compositor-side bind to dispatch the portal
    // action. Discover its exact app-scoped ID, then update only the
    // marked block in the user's Lua bindings.
    let dictate_id = match discover_portal_id(&before, PORTAL_SHORTCUT_DESCRIPTION) {
        Some(id) => id,
        None => {
            registration_failure(portal::Failure::new(
                "Finding compositor shortcut",
                "Verenu's registered shortcut was not visible in Hyprland",
            ));
            return true;
        }
    };
    log::info!("linux hotkey: discovered portal ID");
    let release_command = match std::env::current_exe() {
        Ok(exe) => format!(
            "{} --verenu-hotkey-release",
            shell_quote(&exe.to_string_lossy())
        ),
        Err(error) => {
            registration_failure(portal::Failure::new(
                "Preparing shortcut release",
                error.to_string(),
            ));
            return true;
        }
    };
    let handsfree_command = match std::env::current_exe() {
        Ok(exe) => format!(
            "{} --verenu-hotkey-handsfree",
            shell_quote(&exe.to_string_lossy())
        ),
        Err(error) => {
            registration_failure(portal::Failure::new(
                "Preparing hands-free shortcut",
                error.to_string(),
            ));
            return true;
        }
    };
    if let Err(error) = install_with_capture_suspended(|| crate::core::hyprland::ensure_global_shortcut_binding(
        bindings,
        &dictate_id,
        &release_command,
        &handsfree_command,
        &release_keycodes_for(&configured_keys(&EFFECTIVE_KEYS)),
    )) {
        registration_failure(portal::Failure::new("Installing desktop bindings", error));
        return true;
    }
    let mut activated =
        match portal::request("Listening for shortcuts", portal.receive_activated()).await {
            Ok(stream) => stream,
            Err(error) => {
                registration_failure(error);
                return true;
            }
        };
    log::info!("linux hotkey: Hyprland binding installed");
    let keys = configured_keys(&EFFECTIVE_KEYS);
    conflicts::status(
        "dictation",
        shortcut_display(&configured_keys(&KEYS)),
        Some(shortcut_display(&keys)),
        keys.iter().map(|id| shortcut_code(*id)).collect(),
    );
    if let Err(error) = install_with_capture_suspended(|| super::with_sub_app_shortcut_lock(|| install_sub_app_capture_binding(false))) {
        log::warn!("Could not install sub-app shortcut: {error}");
    }
    match discover_portal_id(&before, PORTAL_CANCEL_DESCRIPTION) {
        Some(id) => {
            if let Ok(mut slot) = CANCEL_PORTAL_ID.lock() {
                *slot = Some(id);
            }
        }
        None => {
            registration_failure(portal::Failure::new(
                "Finding cancel shortcut",
                "Verenu's cancel action was not visible in Hyprland",
            ));
            return true;
        }
    }
    if let Some(id) = discover_portal_id(&before, PORTAL_HANDSFREE_DESCRIPTION) {
        if let Ok(mut slot) = HANDSFREE_PORTAL_ID.lock() {
            *slot = Some(id);
        }
    } else {
        registration_failure(portal::Failure::new(
            "Finding hands-free shortcut",
            "Verenu's hands-free action was not visible in Hyprland",
        ));
        return true;
    }
    match discover_portal_id(&before, PORTAL_COPY_DESCRIPTION) {
        Some(id) => {
            let active = conflicts::bindings("copy").ok().and_then(|bindings| {
                conflicts::choose(
                    &bindings,
                    "Ctrl+Alt+C",
                    &["Ctrl+Alt+F6", "Ctrl+Shift+F6", "Alt+Super+F6"],
                    false,
                )
            });
            if let Some(active) = active {
                if let Err(error) = shortcuts::ensure_copy_binding(&id, &active) {
                    log::warn!("linux hotkey: copy shortcut registration failed: {error}");
                    publish_shortcut_failure(
                        "copy",
                        "Ctrl+Alt+C",
                        portal::Failure::new("Installing copy shortcut", error),
                    );
                } else {
                    conflicts::status("copy", "Ctrl+Alt+C".into(), Some(active), vec![]);
                }
            } else {
                let _ = shortcuts::remove_block(shortcuts::START, shortcuts::END);
                conflicts::status("copy", "Ctrl+Alt+C".into(), None, vec![]);
            }
        }
        None => publish_shortcut_failure(
            "copy",
            "Ctrl+Alt+C",
            portal::Failure::new(
                "Finding copy shortcut",
                "Verenu's copy action was not visible in Hyprland",
            ),
        ),
    }
    let [cancel, handsfree] = requested_control_keys();
    for (id, requested, key_slot) in [
        (
            "cancel",
            cancel.as_str(),
            &CANCEL_KEY,
        ),
        (
            "handsfree",
            handsfree.as_str(),
            &HANDSFREE_KEY,
        ),
    ] {
        let active = conflicts::bindings(id)
            .ok()
            .and_then(|bindings| conflicts::control_key(&bindings, requested));
        if let Ok(mut key) = key_slot.lock() {
            *key = active.clone().unwrap_or_default();
        }
        conflicts::status(id, requested.into(), active, vec![]);
    }
    register_temporary_controls();
    refresh_escape_listening();
    let gesture = GESTURE.get_or_init(|| Mutex::new(PortalGesture::default()));
    if let Ok(mut state) = gesture.lock() {
        *state = PortalGesture::default();
    }
    let mut configuration_poll = tokio::time::interval(Duration::from_millis(100));
    let mut desktop_check = Instant::now();
    loop {
        tokio::select! {
            _ = owner_changes.next() => {
                release_disconnected_hold();
                registration_failure(portal::Failure::new("Watching the desktop portal", "The desktop portal restarted or disconnected"));
                return true;
            },
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
            event = activated.next() => {
                let Some(event) = event else {
                    release_disconnected_hold();
                    registration_failure(portal::Failure::new("Listening for shortcuts", "The desktop portal disconnected"));
                    return true;
                };
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
                            log::info!("linux hotkey: portal press");
                            CHORD_ACTIVE.store(true, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = PRESS.get() { cb(); }
                        }
                        Some(PortalGestureAction::Handsfree) => {
                            log::info!("linux hotkey: portal double-tap, toggling handsfree");
                            CHORD_ACTIVE.store(false, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = HANDLESS_CB.get() { cb(); }
                        }
                        _ => log::debug!("linux hotkey: ignored duplicate portal activation"),
                    }
                } else if id == PORTAL_CANCEL_ID {
                    if CANCEL_KEY.lock().map(|key| key.is_empty()).unwrap_or(true) { continue; }
                    log::info!("linux hotkey: portal Escape-to-cancel fired");
                    if let Some(cb) = ESCAPE.get() { cb(); }
                } else if id == PORTAL_CHORD_CANCEL_ID {
                    if should_cancel_hold(CHORD_ACTIVE.load(Ordering::SeqCst), HANDLESS.load(Ordering::SeqCst)) {
                        log::info!("linux hotkey: discarding hold used by another shortcut");
                        if let Some(cb) = CHORD_CANCEL.get() { cb(); }
                    }
                } else if id == PORTAL_COPY_ID {
                    if let Some(cb) = COPY.get() { cb(); }
                } else if id == PORTAL_HANDSFREE_ID {
                    if HANDSFREE_KEY.lock().map(|key| key.is_empty()).unwrap_or(true) { continue; }
                    notify_handless();
                }
            },
        }
    }
}

fn publish_shortcut_failure(id: &str, requested: &str, failure: portal::Failure) {
    log::error!("linux hotkey: {}: {}", failure.stage, failure.reason);
    super::shortcut_status::publish(super::shortcut_status::ShortcutStatus {
        id: id.into(),
        requested: requested.into(),
        active: None,
        codes: vec![],
        note: Some(failure.note()),
        conflict: None,
    });
}

fn release_disconnected_hold() {
    if CHORD_ACTIVE.swap(false, Ordering::SeqCst) {
        if let Some(cb) = RELEASE.get() {
            cb();
        }
    }
}

fn registration_failure(failure: portal::Failure) {
    log::error!("linux hotkey: {}: {}", failure.stage, failure.reason);
    CHORD_ACTIVE.store(false, Ordering::SeqCst);
    for slot in [&CANCEL_KEY, &HANDSFREE_KEY] {
        if let Ok(mut key) = slot.lock() { key.clear(); }
    }
    refresh_escape_listening();
    register_temporary_controls();
    if let Ok(mut id) = CANCEL_PORTAL_ID.lock() {
        *id = None;
    }
    if let Ok(mut id) = HANDSFREE_PORTAL_ID.lock() {
        *id = None;
    }
    let previous = super::shortcut_status::get_shortcut_status();
    let statuses = failure_statuses(
        &failure,
        &shortcut_display(&configured_keys(&KEYS)),
        &previous,
    );
    super::shortcut_status::publish_many(statuses);
}

fn failure_statuses(
    failure: &portal::Failure,
    dictation: &str,
    previous: &[super::shortcut_status::ShortcutStatus],
) -> Vec<super::shortcut_status::ShortcutStatus> {
    let [cancel, handsfree] = requested_control_keys();
    [("dictation", dictation), ("copy", "Ctrl+Alt+C"), ("capture", "Ctrl+Alt+Shift+S"), ("cancel", cancel.as_str()), ("handsfree", handsfree.as_str())]
        .into_iter()
        .map(|(id, default)| super::shortcut_status::ShortcutStatus {
            id: id.into(),
            requested: previous.iter().find(|status| status.id == id && id != "dictation")
                .map(|status| status.requested.clone()).unwrap_or_else(|| default.into()),
            active: None,
            codes: vec![],
            note: Some(if id == "dictation" { failure.note() } else {
                "Unavailable until global shortcuts reconnect. See Dictation hotkey in Settings > General for details.".into()
            }),
            conflict: None,
        })
        .collect()
}

fn shortcuts_need_rebind() -> bool {
    super::shortcut_status::get_shortcut_status()
        .iter()
        .any(|status| {
            let Ok(bindings) = conflicts::bindings(&status.id) else {
                return false;
            };
            let ignore_mods = false;
            if matches!(status.id.as_str(), "cancel" | "handsfree")
                && conflicts::control_conflict(&status.id, &status.requested, &bindings, &configured_keys(&EFFECTIVE_KEYS)) != status.conflict {
                return true;
            }
            if matches!(status.id.as_str(), "cancel" | "handsfree") {
                return conflicts::control_key(&bindings, &status.requested) != status.active;
            }
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
                    _ => false,
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

/// Hyprland can retain entries from an editor-launched dev session.
fn discover_portal_id(before: &[(String, String)], description: &str) -> Option<String> {
    let after = list_portal_shortcuts();
    pick_portal_id(before, &after, description, portal::desktop_id())
}

fn pick_portal_id(
    before: &[(String, String)],
    after: &[(String, String)],
    description: &str,
    registered_app_id: &str,
) -> Option<String> {
    let before_ids: Vec<&str> = before.iter().map(|(id, _)| id.as_str()).collect();
    let matching: Vec<&(String, String)> = after
        .iter()
        .filter(|(id, candidate)| {
            candidate == description
                && id.split_once(':').map(|(scope, _)| scope) == Some(registered_app_id)
        })
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
    fn restarted_portal_actions_never_select_a_stale_editor_identity() {
        let listing = parse_portal_shortcuts(
            "verenu:dictate -> Verenu dictation\nverenu:cancel -> Verenu cancel\ncom.t3tools.T3Code:dictate -> Verenu dictation\ncom.t3tools.T3Code:cancel -> Verenu cancel\n",
        );
        for (description, expected) in [
            (PORTAL_SHORTCUT_DESCRIPTION, "verenu:dictate"),
            (PORTAL_CANCEL_DESCRIPTION, "verenu:cancel"),
        ] {
            assert_eq!(pick_portal_id(&listing, &listing, description, "verenu").as_deref(), Some(expected));
            assert_eq!(pick_portal_id(&[], &listing, description, "verenu").as_deref(), Some(expected));
        }
        let foreign = parse_portal_shortcuts("com.t3tools.T3Code:dictate -> Verenu dictation\n");
        assert_eq!(pick_portal_id(&[], &foreign, PORTAL_SHORTCUT_DESCRIPTION, "verenu"), None);
    }

    #[test]
    fn registration_failure_invalidates_all_shortcuts_and_preserves_preferences() {
        let previous = [crate::core::hotkey::shortcut_status::ShortcutStatus {
            id: "capture".into(),
            requested: "Ctrl+Alt+F7".into(),
            active: Some("Ctrl+Alt+F7".into()),
            codes: vec!["F7".into()],
            note: None,
            conflict: None,
        }];
        let failure =
            super::portal::Failure::new("Creating shortcut session", "An app id is required");
        let statuses = super::failure_statuses(&failure, "Ctrl + Super", &previous);
        assert_eq!(statuses.len(), 5);
        assert!(statuses.iter().all(|status| status.active.is_none()
            && status.codes.is_empty()
            && status.note.is_some()));
        let dictate = statuses
            .iter()
            .find(|status| status.id == "dictation")
            .unwrap();
        assert_eq!(dictate.requested, "Ctrl + Super");
        assert!(dictate
            .note
            .as_deref()
            .unwrap()
            .contains("An app id is required"));
        assert_eq!(
            statuses
                .iter()
                .find(|status| status.id == "capture")
                .unwrap()
                .requested,
            "Ctrl+Alt+F7"
        );
    }

    #[test]
    fn shortcut_suspension_tolerates_bindings_not_initialized_yet() {
        let snippet = super::suspend_shortcuts_snippet(true);
        assert!(snippet.contains("if _verenu_dictation_bindings then"));
        assert!(snippet.contains("handle:set_enabled(false)"));
        assert!(!snippet.contains("error("));
    }

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

    #[test]
    fn dev_session_validation_checks_chord_shape_without_compositor_access() {
        let chord = ["ControlLeft", "AltLeft", "ShiftLeft", "MetaLeft", "KeyK"].map(str::to_string);
        assert!(super::validate_hotkey(&chord).unwrap());

        let two_regular_keys = ["KeyA", "KeyB"].map(str::to_string);
        assert!(super::validate_hotkey(&two_regular_keys).is_err());
    }
    use super::{
        escape_unbind_snippet, parse_portal_shortcuts, pick_portal_id,
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
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION, "com.t3tools.T3Code").as_deref(),
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
            pick_portal_id(&listing, &listing, PORTAL_SHORTCUT_DESCRIPTION, "verenu"),
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
            pick_portal_id(&before, &after, PORTAL_CANCEL_DESCRIPTION, "com.t3tools.T3Code").as_deref(),
            Some("com.t3tools.T3Code:cancel")
        );
        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION, "com.t3tools.T3Code").as_deref(),
            Some("com.t3tools.T3Code:dictate")
        );
        assert_eq!(PORTAL_CANCEL_DESCRIPTION, "Verenu cancel");
    }

    #[test]
    fn escape_bind_targets_the_cancel_action_and_unbind_clears_bare_escape() {
        let snippet = super::shortcuts::temporary_controls("escape", "ESCAPE", Some("com.t3tools.T3Code:cancel"), "cancel", &[]);
        assert!(snippet.contains("hl.bind(\"ESCAPE\""));
        assert!(snippet.contains("hl.dsp.global(\"com.t3tools.T3Code:cancel\")"));
        assert!(!snippet.contains("ignore_mods = true"));
        assert!(snippet.contains("submap_universal = true"));
        assert!(escape_unbind_snippet().contains("_verenu_escape_0_binding:set_enabled(false)"));
        assert!(!escape_unbind_snippet().contains("hl.unbind"));
    }
}
