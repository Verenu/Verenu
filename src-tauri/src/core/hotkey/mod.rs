static SUB_APP_SHORTCUT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn with_sub_app_shortcut_lock<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    with_shortcut_lock(&SUB_APP_SHORTCUT_LOCK, work)
}

fn with_shortcut_lock<T>(lock: &std::sync::Mutex<()>, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _guard = lock.lock().map_err(|_| "Shortcut state unavailable")?;
    work()
}

#[cfg(test)]
mod sub_app_lock_tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    #[test]
    fn reconnect_cannot_read_old_shortcut_during_a_save() {
        let lock = Arc::new(Mutex::new(()));
        let chord = Arc::new(Mutex::new("old"));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_lock = lock.clone();
        let writer_chord = chord.clone();
        let writer = std::thread::spawn(move || with_shortcut_lock(&writer_lock, || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            *writer_chord.lock().unwrap() = "new";
            Ok(())
        }).unwrap());
        started_rx.recv().unwrap();
        let reconnect = std::thread::spawn(move || with_shortcut_lock(&lock, || {
            Ok(*chord.lock().unwrap())
        }).unwrap());
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(reconnect.join().unwrap(), "new");
    }
}

pub fn is_known_key_code(code: &str) -> bool {
    match code {
        "" => false,
        "ShiftLeft" | "ShiftRight" | "ControlLeft" | "ControlRight" => true,
        "AltLeft" | "AltRight" | "MetaLeft" | "MetaRight" | "Fn" => true,
        "Space" | "Escape" | "Enter" | "Backspace" | "Tab" | "CapsLock" => true,
        "Minus" | "Equal" | "BracketLeft" | "BracketRight" | "Backslash" => true,
        "Semicolon" | "Quote" | "Comma" | "Period" | "Slash" | "Backquote" => true,
        "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" => true,
        "Insert" | "Delete" | "Home" | "End" | "PageUp" | "PageDown" => true,
        _ if code.starts_with("Key")
            && code.len() == 4
            && code.as_bytes()[3].is_ascii_uppercase() =>
        {
            true
        }
        _ if code.starts_with("Digit")
            && code.len() == 6
            && code.as_bytes()[5].is_ascii_digit() =>
        {
            true
        }
        _ if code.starts_with('F') && code.len() > 1 => code[1..]
            .parse::<u32>()
            .is_ok_and(|n| (1..=12).contains(&n)),
        _ => false,
    }
}

pub fn normalize_codes(codes: &[String]) -> Result<Vec<String>, String> {
    let legacy_single = codes.len() == 2 && codes[1].is_empty();
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, code) in codes.iter().enumerate() {
        if legacy_single && index == 1 {
            continue;
        }
        if !is_known_key_code(code) {
            return Err(format!("Unrecognized key code: {code}"));
        }
        let identity = modifier_name(code).unwrap_or(code);
        if !seen.insert(identity) {
            return Err("A shortcut cannot contain the same key twice".into());
        }
        result.push(code.clone());
    }
    if result.is_empty() {
        return Err("Press at least one key".into());
    }
    Ok(result)
}

pub fn modifier_name(code: &str) -> Option<&'static str> {
    match code {
        "ControlLeft" | "ControlRight" => Some("Ctrl"),
        "AltLeft" | "AltRight" => Some("Alt"),
        "ShiftLeft" | "ShiftRight" => Some("Shift"),
        "MetaLeft" | "MetaRight" => Some("Super"),
        _ => None,
    }
}

pub fn is_control_key(code: &str) -> bool {
    is_known_key_code(code) && modifier_name(code).is_none() && code != "Fn"
}

pub fn mapped_codes(codes: &[String]) -> Result<Vec<u32>, String> {
    normalize_codes(codes)?
        .iter()
        .map(|code| {
            let id = map_code_to_vk(code);
            if id == 0 {
                Err(format!("This platform does not support {code}"))
            } else {
                Ok(id)
            }
        })
        .collect()
}

pub fn conflicts_with_chord(codes: &[String], chord: chord::Chord) -> bool {
    let mut other = Vec::new();
    if chord.ctrl {
        other.push("Ctrl".to_string());
    }
    if chord.alt {
        other.push("Alt".to_string());
    }
    if chord.shift {
        other.push("Shift".to_string());
    }
    if chord.super_key {
        other.push("Super".to_string());
    }
    other.push(chord.web_code());
    let mut codes = codes
        .iter()
        .filter(|code| !code.is_empty())
        .map(|code| modifier_name(code).unwrap_or(code).to_string())
        .collect::<Vec<_>>();
    codes.sort();
    other.sort();
    codes == other
}

pub fn shortcut_conflicts(codes: &[String], chord: chord::Chord, windows: bool) -> bool {
    if !windows { return conflicts_with_chord(codes, chord); }
    if codes.iter().any(|code| code == &chord.web_code()) { return true; }
    !codes.is_empty() && codes.iter().all(|code| match modifier_name(code) {
        Some("Ctrl") => chord.ctrl,
        Some("Alt") => chord.alt,
        Some("Shift") => chord.shift,
        Some("Super") => chord.super_key,
        _ => false,
    })
}

pub fn copy_last_conflicts(chord: chord::Chord, windows: bool, macos: bool) -> bool {
    if windows {
        chord.ctrl && chord.alt && chord.key == chord::ChordKey::Letter('C')
    } else {
        chord == chord::Chord::parse(if macos { "Alt+Super+C" } else { "Ctrl+Alt+C" }).expect("copy shortcut")
    }
}

pub fn copy_last_conflicts_with_keys(codes: &[String]) -> bool {
    let base = if cfg!(target_os = "macos") { "Alt+Super+C" } else { "Ctrl+Alt+C" };
    let mut chord = chord::Chord::parse(base).expect("copy shortcut");
    for shift in [false, true] {
        for super_key in [false, true] {
            if cfg!(windows) {
                chord.shift = shift;
                chord.super_key = super_key;
            } else if shift || super_key {
                continue;
            }
            if shortcut_conflicts(codes, chord, cfg!(windows)) { return true; }
        }
    }
    false
}

static CAPTURE_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CAPTURE_TRANSITION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn capture_active() -> bool {
    CAPTURE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn set_capture_active(active: bool) -> Result<(), String> {
    with_capture_transition_lock(|| {
        transition_capture(&CAPTURE_ACTIVE, active, suspend_shortcuts, reset_chord_state)
    })
}

pub(crate) fn with_capture_transition_lock<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    with_shortcut_lock(&CAPTURE_TRANSITION_LOCK, work)
}

fn transition_capture(
    state: &std::sync::atomic::AtomicBool,
    active: bool,
    mut suspend: impl FnMut(bool) -> Result<(), String>,
    reset: impl FnOnce(),
) -> Result<(), String> {
    let previous = state.swap(active, std::sync::atomic::Ordering::SeqCst);
    if previous == active && active {
        return Ok(());
    }
    if let Err(error) = suspend(active) {
        if active {
            let _ = suspend(previous);
            state.store(previous, std::sync::atomic::Ordering::SeqCst);
        } else {
            reset();
        }
        return Err(error);
    }
    reset();
    Ok(())
}

#[cfg(test)]
mod capture_transition_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn failed_exit_does_not_suspend_shortcuts_already_restored() {
        let state = AtomicBool::new(true);
        let mut calls = Vec::new();
        let reset = std::cell::Cell::new(false);
        let result = transition_capture(&state, false, |active| {
            calls.push(active);
            Err("Sub-app shortcut is unavailable".into())
        }, || reset.set(true));
        assert!(result.is_err());
        assert_eq!(calls, vec![false]);
        assert!(!state.load(Ordering::SeqCst));
        assert!(reset.get());
    }

    #[test]
    fn failed_capture_entry_restores_previous_state() {
        let state = AtomicBool::new(false);
        let mut calls = Vec::new();
        assert!(transition_capture(&state, true, |active| {
            calls.push(active);
            if active { Err("Could not suspend shortcuts".into()) } else { Ok(()) }
        }, || {}).is_err());
        assert_eq!(calls, vec![true, false]);
        assert!(!state.load(Ordering::SeqCst));
    }

    #[test]
    fn failed_exit_can_be_retried_while_capture_is_already_off() {
        let state = AtomicBool::new(true);
        let mut attempts = 0;
        let mut restore = |active: bool| {
            assert!(!active);
            attempts += 1;
            if attempts == 1 { Err("Shortcut unavailable".into()) } else { Ok(()) }
        };
        assert!(transition_capture(&state, false, &mut restore, || {}).is_err());
        assert!(transition_capture(&state, false, &mut restore, || {}).is_ok());
        assert_eq!(attempts, 2);
        assert!(!state.load(Ordering::SeqCst));
    }

    #[test]
    fn overlapping_capture_transitions_complete_in_order() {
        use std::sync::{Arc, Mutex, mpsc};
        let state = Arc::new(AtomicBool::new(true));
        let lock = Arc::new(Mutex::new(()));
        let native = Arc::new(AtomicBool::new(true));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (first_state, first_lock, first_native) = (state.clone(), lock.clone(), native.clone());
        let first = std::thread::spawn(move || with_shortcut_lock(&first_lock, || {
            transition_capture(&first_state, false, |active| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                first_native.store(active, Ordering::SeqCst);
                Ok(())
            }, || {})
        }).unwrap());
        started_rx.recv().unwrap();
        let (next_state, next_lock, next_native) = (state.clone(), lock.clone(), native.clone());
        let second = std::thread::spawn(move || with_shortcut_lock(&next_lock, || {
            transition_capture(&next_state, true, |active| {
                next_native.store(active, Ordering::SeqCst);
                Ok(())
            }, || {})
        }).unwrap());
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        assert!(state.load(Ordering::SeqCst));
        assert!(native.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
mod known_key_code_tests {
    use super::is_known_key_code;

    #[test]
    fn auxiliary_conflicts_ignore_modifier_order_and_sides() {
        let codes = ["KeyC", "AltRight", "ControlLeft"].map(String::from);
        assert!(super::conflicts_with_chord(
            &codes,
            super::chord::Chord::parse("Ctrl+Alt+C").unwrap()
        ));
        assert!(!super::conflicts_with_chord(
            &codes,
            super::chord::Chord::parse("Ctrl+Alt+Shift+C").unwrap()
        ));
        let mac_codes = ["F7", "MetaLeft", "AltRight"].map(String::from);
        assert!(super::conflicts_with_chord(
            &mac_codes,
            super::chord::Chord::parse("Alt+Super+F7").unwrap()
        ));
    }

    #[test]
    fn windows_copy_shortcut_intercepts_additional_modifiers() {
        for chord in ["Ctrl+Alt+C", "Ctrl+Alt+Shift+C", "Ctrl+Alt+Super+C", "Ctrl+Alt+Shift+Super+C"] {
            assert!(super::copy_last_conflicts(super::chord::Chord::parse(chord).unwrap(), true, false));
        }
        assert!(!super::copy_last_conflicts(super::chord::Chord::parse("Ctrl+Shift+C").unwrap(), true, false));
        assert!(!super::copy_last_conflicts(super::chord::Chord::parse("Ctrl+Alt+Shift+C").unwrap(), false, false));
    }

    #[test]
    fn windows_dictation_owns_regular_keys_and_modifier_only_chords() {
        let capture = super::chord::Chord::parse("Ctrl+Shift+A").unwrap();
        let keys = ["ControlLeft", "AltLeft", "KeyA"].map(String::from);
        assert!(super::shortcut_conflicts(&keys, capture, true));
        assert!(!super::shortcut_conflicts(&keys, capture, false));
        let copy = super::chord::Chord::parse("Ctrl+Alt+C").unwrap();
        assert!(super::shortcut_conflicts(&["KeyA", "KeyC"].map(String::from), copy, true));
        assert!(super::shortcut_conflicts(&["ControlLeft", "AltLeft"].map(String::from), copy, true));
        assert!(!super::shortcut_conflicts(&["ControlLeft", "ShiftLeft"].map(String::from), copy, true));
    }

    #[test]
    fn recognizes_supported_hotkey_codes() {
        for code in [
            "ControlLeft",
            "MetaLeft",
            "AltLeft",
            "ShiftLeft",
            "Space",
            "Fn",
            "F5",
            "KeyA",
            "Digit7",
        ] {
            assert!(is_known_key_code(code), "{code} should be known");
        }
    }

    #[test]
    fn rejects_unknown_and_malformed_codes() {
        for code in ["Foo", "Bar", "Control", "Key", "F13", "Digit10", "", "meta"] {
            assert!(!is_known_key_code(code), "{code:?} should be rejected");
        }
    }
}

#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
mod gesture;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

pub mod chord;
pub mod shortcut_status;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod noop {
    pub fn suspend_shortcuts(_active: bool) -> Result<(), String> {
        Ok(())
    }
    pub fn is_hotkey_available(_keys: &[String]) -> Result<bool, String> {
        Err("Global shortcuts are unavailable on this platform".into())
    }
    pub fn validate_hotkey(_keys: &[String]) -> Result<bool, String> {
        Err("Global shortcuts are unavailable on this platform".into())
    }
    pub fn update_keys(_keys: &[u32]) -> Result<(), String> {
        Err("Global shortcuts are unavailable on this platform".into())
    }
    pub fn set_sub_app_capture_chord(_chord: super::chord::Chord) -> Result<(), String> { Ok(()) }
    pub fn reset_chord_state() {}
    pub fn set_handless_active(_v: bool) {}
    pub fn begin_synthetic_paste_suppression(_duration_ms: u64) {}
    pub fn set_processing_generation(_generation: u64) {}
    pub fn clear_processing_generation(_expected_generation: u64) {}
    pub fn is_win_key_down() -> bool {
        false
    }
    pub fn force_release_win_key() {}
    pub fn caps_lock_is_on() -> bool {
        false
    }
    pub fn map_code_to_vk(_code: &str) -> u32 {
        0
    }
    #[allow(clippy::too_many_arguments)]
    pub fn start<P, R, H, C, E, L, S>(
        _on_press: P,
        _on_release: R,
        _on_handless: H,
        _on_cancel: C,
        _on_escape: E,
        _on_copy_last: L,
        _on_capture_sub_app: S,
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
        Ok(std::thread::spawn(|| {}))
    }
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub use noop::*;
