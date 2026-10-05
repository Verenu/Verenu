//! Global hold/release hotkey.
//!
//! Both platforms expose the same public contract consumed by `main.rs` and
//! `commands`:
//!   `start(on_press, on_release, on_handless, on_cancel, on_escape, on_copy_last)`,
//!   `update_keys`, `map_code_to_vk`, `is_hotkey_available`,
//!   `reset_chord_state`, `set_handless_active`,
//!   `begin_synthetic_paste_suppression`, `is_win_key_down`,
//!   `force_release_win_key`.
//!
//! Windows uses a `WH_KEYBOARD_LL` hook; macOS uses Carbon `RegisterEventHotKey`
//! via the `global-hotkey` crate. The numeric
//! key ids produced by `map_code_to_vk` are platform-private — only the matching
//! backend interprets them — so the two implementations never need to agree on a
//! shared numbering.

/// Whether `code` (a JS `KeyboardEvent.code`) is a key the backend can map to a
/// real key on either supported platform. Platform-independent: mirrors the
/// union of codes accepted by the Windows and macOS `map_code_to_vk`
/// implementations, so a hotkey that validates here will register on at least
/// one of them.
///
/// Used by the generic `save_setting`/`import_data` path, which must not let a
/// hand-edited or imported settings.json silently disable the dictation hotkey
/// by storing a code no backend recognizes (e.g. `["Foo", "Bar"]` — the startup
/// hook would then map both to VK 0 and never fire).
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

/// Accept old single-key settings with an empty second slot, then remove it.
/// Modifier sides have always matched either side in the native backends.
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

static CAPTURE_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) fn capture_active() -> bool {
    CAPTURE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn set_capture_active(active: bool) -> Result<(), String> {
    let previous = CAPTURE_ACTIVE.swap(active, std::sync::atomic::Ordering::SeqCst);
    if previous == active {
        return Ok(());
    }
    if let Err(error) = suspend_shortcuts(active) {
        let _ = suspend_shortcuts(previous);
        CAPTURE_ACTIVE.store(previous, std::sync::atomic::Ordering::SeqCst);
        return Err(error);
    }
    reset_chord_state();
    Ok(())
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

// Fallback for any other platform (e.g. Linux CI): inert no-op shims so the
// crate still builds. Mirrors the public contract above.
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod noop {
    pub fn suspend_shortcuts(_active: bool) -> Result<(), String> {
        Ok(())
    }
    pub fn is_hotkey_available(_keys: &[String]) -> Result<bool, String> {
        Err("Global shortcuts are unavailable on this platform".into())
    }
    pub fn update_keys(_keys: &[u32]) -> Result<(), String> {
        Err("Global shortcuts are unavailable on this platform".into())
    }
    pub fn set_sub_app_capture_chord(_chord: super::chord::Chord) {}
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
