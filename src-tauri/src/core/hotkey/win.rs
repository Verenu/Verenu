use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const VK_BACK: u32 = 0x08;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_SHIFT: u32 = 0x10;
const VK_CTRL: u32 = 0x11;
const VK_ALT: u32 = 0x12;
const VK_RETURN: u32 = 0x0D;
const VK_C: u32 = 0x43;

static MODIFIER_VKS: &[u32] = &[
    0xA0, 0xA1,
    0xA2, 0xA3,
    0xA4, 0xA5,
    0x5B, 0x5C,
    0x14, 0x90, 0x91,
];
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, MapVirtualKeyExW, RegisterHotKey,
    ToUnicodeEx, UnregisterHotKey, HOT_KEY_MODIFIERS, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    SetWindowsHookExW, TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

unsafe fn modifier_held(vk: u32) -> bool {
    let held = |v: u32| -> bool { (GetAsyncKeyState(v as i32) & 0x8000u16 as i16) != 0 };
    match vk {
        160 | 161 => held(16),
        162 | 163 => held(17),
        164 | 165 => held(18),
        91 | 92 => held(91) || held(92),
        _ => held(vk),
    }
}

fn vk_matches(vk: u32, key: u32) -> bool {
    match key {
        160 | 161 => vk == 160 || vk == 161,
        162 | 163 => vk == 162 || vk == 163,
        164 | 165 => vk == 164 || vk == 165,
        91 | 92 => vk == 91 || vk == 92,
        _ => vk == key,
    }
}

fn is_hotkey_modifier_vk(vk: u32) -> bool {
    matches!(vk, 0xA0..=0xA5 | 0x5B | 0x5C)
}

fn is_cursor_movement_key(vk: u32) -> bool {
    matches!(
        vk,
        0x21..=0x28 |
        0x2D |
        0x2E
    )
}

#[inline]
fn map_vk_to_scan_code(vk: u32, layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL) -> u32 {
    unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout)) }
}

#[inline]
fn to_unicode_layout(
    vk: u32,
    scan: u32,
    state: &[u8; 256],
    buff: &mut [u16],
    layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
) -> i32 {
    unsafe { ToUnicodeEx(vk, scan, state, buff, 0, Some(layout)) }
}

fn vk_to_char(vk: u32) -> Option<char> {
    if vk == VK_RETURN {
        return Some('\n');
    }
    if vk == 0x20 {
        return Some(' ');
    }

    unsafe {
        let mut state = [0u8; 256];
        if GetAsyncKeyState(0xA0) < 0 {
            state[0xA0] |= 0x80;
            state[0x10] |= 0x80;
        }
        if GetAsyncKeyState(0xA1) < 0 {
            state[0xA1] |= 0x80;
            state[0x10] |= 0x80;
        }
        if GetAsyncKeyState(0xA2) < 0 {
            state[0xA2] |= 0x80;
            state[0x11] |= 0x80;
        }
        if GetAsyncKeyState(0xA3) < 0 {
            state[0xA3] |= 0x80;
            state[0x11] |= 0x80;
        }
        if GetAsyncKeyState(0xA4) < 0 {
            state[0xA4] |= 0x80;
            state[0x12] |= 0x80;
        }
        if GetAsyncKeyState(0xA5) < 0 {
            state[0xA5] |= 0x80;
            state[0x12] |= 0x80;
        }
        if (GetKeyState(0x14) & 0x0001) != 0 {
            state[0x14] |= 0x01;
        }

        let foreground = GetForegroundWindow();
        let thread_id = GetWindowThreadProcessId(foreground, None);
        let layout = GetKeyboardLayout(thread_id);
        let scan = map_vk_to_scan_code(vk, layout);
        if scan == 0 {
            return None;
        }

        let mut buff = [0u16; 8];
        let rc = to_unicode_layout(vk, scan, &state, &mut buff, layout);
        if rc < 0 {
            let neutral_vk = 0x20u32;
            let neutral_scan = map_vk_to_scan_code(neutral_vk, layout);
            for _ in 0..4 {
                if to_unicode_layout(neutral_vk, neutral_scan, &state, &mut buff, layout) >= 0 {
                    break;
                }
            }
            return None;
        }
        if rc == 0 {
            return None;
        }

        let s = String::from_utf16_lossy(&buff[..rc as usize]);
        s.chars()
            .next()
            .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
    }
}

pub fn is_hotkey_available(keys: &[String]) -> Result<bool, String> {
    let ids = super::mapped_codes(keys)?;
    let mut modifiers = 0;
    let mut regular = Vec::new();
    for (code, id) in keys.iter().zip(&ids) {
        modifiers |= match super::modifier_name(code) {
            Some("Ctrl") => MOD_CONTROL.0,
            Some("Alt") => MOD_ALT.0,
            Some("Shift") => MOD_SHIFT.0,
            Some("Super") => MOD_WIN.0,
            _ => {
                regular.push(*id);
                0
            }
        };
    }
    if regular.is_empty() {
        return Ok(false);
    }
    // Windows exposes reservations only for modifiers plus one trigger key.
    // Arbitrary multi-key chords are recognized by the low-level hook.
    if regular.len() > 1 {
        return Ok(true);
    }
    unsafe {
        if RegisterHotKey(None, 0x5A8E, HOT_KEY_MODIFIERS(modifiers), regular[0]).is_err() {
            return Ok(false);
        }
        let _ = UnregisterHotKey(None, 0x5A8E);
    }
    Ok(true)
}

pub fn validate_hotkey(keys: &[String]) -> Result<bool, String> {
    let ids = super::mapped_codes(keys)?;
    Ok(keys
        .iter()
        .zip(&ids)
        .any(|(code, _)| super::modifier_name(code).is_none()))
}

static KEYS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(0);
static SYNTHETIC_PASTE_SUPPRESSION_UNTIL: AtomicU64 = AtomicU64::new(0);
fn is_menu_trigger_vk(vk: u32) -> bool {
    matches!(vk, 164 | 165 | 18 | 91 | 92)
}

use super::gesture::{ChordAction, ChordKey, ChordStateMachine, KeyDisposition, KeyEdge};

thread_local! {
    static CHORD_MACHINE: std::cell::RefCell<ChordStateMachine> =
        std::cell::RefCell::new(ChordStateMachine::default());
    static LOCAL_KEYS: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(vec![162, 91]);
    static LOCAL_GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

static RESET_REQUESTED: AtomicBool = AtomicBool::new(false);

static FORCE_KEY_RELEASE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn update_keys(keys: &[u32]) -> Result<(), String> {
    if keys.is_empty() || keys.contains(&0) {
        return Err("Unsupported Windows shortcut".into());
    }
    *KEYS.lock().map_err(|_| "Shortcut state unavailable")? = keys.to_vec();
    CONFIG_GENERATION.fetch_add(1, Ordering::SeqCst);
    reset_chord_state();
    Ok(())
}

pub fn suspend_shortcuts(_active: bool) -> Result<(), String> {
    Ok(())
}

pub fn reset_chord_state() {
    RESET_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn set_handless_active(v: bool) {
    HANDLESS_ACTIVE.store(v, Ordering::SeqCst);
}

pub fn begin_synthetic_paste_suppression(duration_ms: u64) {
    let until = unsafe { GetTickCount64() }.saturating_add(duration_ms);
    SYNTHETIC_PASTE_SUPPRESSION_UNTIL.fetch_max(until, Ordering::SeqCst);
}

static PROCESSING_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn set_processing_generation(generation: u64) {
    PROCESSING_GENERATION.store(generation, Ordering::SeqCst);
}

pub fn clear_processing_generation(expected_generation: u64) {
    let _ = PROCESSING_GENERATION.compare_exchange(
        expected_generation,
        0,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

pub fn caps_lock_is_on() -> bool {
    CAPS_LOCK_ON.load(Ordering::SeqCst)
}

pub fn is_win_key_down() -> bool {
    unsafe { modifier_held(91) }
}

pub fn force_release_win_key() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LWIN, VK_RWIN,
    };

    let ki = |vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_KEYUP.0 | KEYEVENTF_EXTENDEDKEY.0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let release = [ki(VK_LWIN), ki(VK_RWIN)];
    unsafe { SendInput(&release, std::mem::size_of::<INPUT>() as i32) };

    FORCE_KEY_RELEASE_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn map_code_to_vk(code: &str) -> u32 {
    match code {
        "ShiftLeft" => 160,
        "ShiftRight" => 161,
        "ControlLeft" => 162,
        "ControlRight" => 163,
        "AltLeft" => 164,
        "AltRight" => 165,
        "MetaLeft" => 91,
        "MetaRight" => 92,
        "Space" => 32,
        "Escape" => 27,
        "Enter" => VK_RETURN,
        "Backspace" => 8,
        "Tab" => 9,
        "CapsLock" => 20,
        "Minus" => 189,
        "Equal" => 187,
        "BracketLeft" => 219,
        "BracketRight" => 221,
        "Backslash" => 220,
        "Semicolon" => 186,
        "Quote" => 222,
        "Comma" => 188,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "ArrowUp" => 38,
        "ArrowDown" => 40,
        "ArrowLeft" => 37,
        "ArrowRight" => 39,
        "Insert" => 45,
        "Delete" => 46,
        "Home" => 36,
        "End" => 35,
        "PageUp" => 33,
        "PageDown" => 34,
        c if c.starts_with("Key") && c.len() == 4 => c.as_bytes()[3] as u32,
        c if c.starts_with("Digit") && c.len() == 6 => c.as_bytes()[5] as u32,
        c if c.starts_with("F") && c.len() > 1 => {
            if let Ok(n) = c[1..].parse::<u32>() {
                if (1..=12).contains(&n) {
                    111 + n
                } else {
                    0
                }
            } else {
                0
            }
        }
        c if c.starts_with("Numpad") && c.len() == 7 => {
            let b = c.as_bytes()[6];
            if b.is_ascii_digit() {
                96 + (b - b'0') as u32
            } else {
                0
            }
        }
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        _ => 0,
    }
}

static PRESS_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static RELEASE_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static HANDLESS_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static CANCEL_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

static CAPS_LOCK_ON: AtomicBool = AtomicBool::new(false);

static HANDLESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static ESCAPE_CANCELLED: AtomicBool = AtomicBool::new(false);
static ESCAPE_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static ESCAPE_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

static COPY_LAST_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static COPY_LAST_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

static SUB_APP_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static SUB_APP_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static SUB_APP_VK: AtomicU32 = AtomicU32::new(0x53);
static SUB_APP_MODS: AtomicU32 = AtomicU32::new(1 | 2 | 4);

pub fn set_sub_app_capture_chord(chord: super::chord::Chord) {
    let mods = u32::from(chord.ctrl)
        | (u32::from(chord.alt) << 1)
        | (u32::from(chord.shift) << 2)
        | (u32::from(chord.super_key) << 3);
    SUB_APP_MODS.store(mods, Ordering::SeqCst);
    SUB_APP_VK.store(chord.windows_vk(), Ordering::SeqCst);
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if super::capture_active() {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    if code == HC_ACTION as i32 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let msg = wparam.0 as u32;
        let vk = kb.vkCode;
        let is_injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        if should_suppress_synthetic_paste_event(
            is_injected,
            GetTickCount64(),
            SYNTHETIC_PASTE_SUPPRESSION_UNTIL.load(Ordering::SeqCst),
        ) {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        CAPS_LOCK_ON.store((GetKeyState(0x14) & 0x0001) != 0, Ordering::SeqCst);

        let generation = CONFIG_GENERATION.load(Ordering::Acquire);
        if LOCAL_GENERATION.with(|local| local.get() != generation) {
            if let Ok(keys) = KEYS.try_lock() {
                let keys = if keys.is_empty() {
                    vec![162, 91]
                } else {
                    keys.clone()
                };
                CHORD_MACHINE.with(|m| {
                    *m.borrow_mut() = ChordStateMachine::with_partial_passthrough(
                        keys.iter().map(|key| is_hotkey_modifier_vk(*key)).collect(),
                    )
                });
                LOCAL_KEYS.with(|local| *local.borrow_mut() = keys);
                LOCAL_GENERATION.with(|local| local.set(generation));
            }
        }
        let chord_key = LOCAL_KEYS.with(|keys| {
            let keys = keys.borrow();
            keys.iter()
                .position(|key| *key == vk)
                .or_else(|| keys.iter().position(|key| vk_matches(vk, *key)))
        });
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        if RESET_REQUESTED.swap(false, Ordering::SeqCst) {
            CHORD_MACHINE.with(|m| m.borrow_mut().reset_gesture_state());
            ESCAPE_CANCELLED.store(false, Ordering::SeqCst);
            ESCAPE_KEY_DOWN.store(false, Ordering::SeqCst);
        }

        if FORCE_KEY_RELEASE_REQUESTED.swap(false, Ordering::SeqCst) {
            CHORD_MACHINE.with(|m| {
                let mut machine = m.borrow_mut();
                machine.clear_physical_keys();
            });
        }

        if let Some(index) = chord_key.filter(|_| is_down || is_up) {
            let key = ChordKey(index);
            let edge = if is_down { KeyEdge::Down } else { KeyEdge::Up };
            let now = GetTickCount64();
            let (outcome, menu_key_passed_through) = LOCAL_KEYS.with(|keys| {
                let keys = keys.borrow();
                CHORD_MACHINE.with(|m| {
                    let mut machine = m.borrow_mut();
                    for (other, id) in keys.iter().enumerate().filter(|(other, _)| *other != index)
                    {
                        machine.reconcile_stale_key(ChordKey(other), modifier_held(*id), now);
                    }
                    let menu_key_passed_through = keys.iter().enumerate().any(|(other, id)| {
                        other != index
                            && machine.keys[other].passed_through
                            && is_menu_trigger_vk(*id)
                    });
                    (
                        machine.on_key_event(key, edge, now),
                        menu_key_passed_through,
                    )
                })
            });
            let mut action = outcome.action;
            let mut disposition = outcome.disposition;

            if matches!(
                action,
                Some(ChordAction::FireRelease) | Some(ChordAction::FireCancel)
            ) && ESCAPE_CANCELLED.swap(false, Ordering::SeqCst)
            {
                action = None;
            }

            if edge == KeyEdge::Down
                && disposition == KeyDisposition::Suppress
                && menu_key_passed_through
                && LOCAL_KEYS.with(|keys| keys.borrow().iter().all(|id| MODIFIER_VKS.contains(id)))
            {
                disposition = KeyDisposition::Passthrough;
                CHORD_MACHINE.with(|m| m.borrow_mut().mark_key_passed_through(key));
            }

            match action {
                Some(ChordAction::FirePress) => {
                    if let Some(cb) = PRESS_CB.get() {
                        cb();
                    }
                }
                Some(ChordAction::FireRelease) => {
                    if let Some(cb) = RELEASE_CB.get() {
                        cb();
                    }
                }
                Some(ChordAction::FireCancel) => {
                    if let Some(cb) = CANCEL_CB.get() {
                        cb();
                    }
                }
                Some(ChordAction::FireHandless) => {
                    if let Some(cb) = HANDLESS_CB.get() {
                        cb();
                    }
                }
                None => {}
            }

            if disposition == KeyDisposition::Suppress {
                return LRESULT(1);
            }
        }

        if vk == VK_SPACE && chord_key.is_none() && (is_down || is_up) {
            let edge = if is_down { KeyEdge::Down } else { KeyEdge::Up };
            let outcome = CHORD_MACHINE.with(|m| m.borrow_mut().on_space_event(edge));
            if outcome.action == Some(ChordAction::FireHandless) {
                if let Some(cb) = HANDLESS_CB.get() {
                    cb();
                }
            }
            if outcome.disposition == KeyDisposition::Suppress {
                return LRESULT(1);
            }
        }

        if vk == VK_ESCAPE {
            let chord_down = CHORD_MACHINE.with(|m| m.borrow().chord_down);
            if is_down
                && (chord_down
                    || HANDLESS_ACTIVE.load(Ordering::SeqCst)
                    || PROCESSING_GENERATION.load(Ordering::SeqCst) != 0)
            {
                if chord_down {
                    ESCAPE_CANCELLED.store(true, Ordering::SeqCst);
                }
                ESCAPE_KEY_DOWN.store(true, Ordering::SeqCst);
                if let Some(cb) = ESCAPE_CB.get() {
                    cb();
                }
                return LRESULT(1);
            }
            if is_up && ESCAPE_KEY_DOWN.swap(false, Ordering::SeqCst) {
                return LRESULT(1);
            }
        }

        if vk == VK_C {
            if is_down && unsafe { modifier_held(VK_CTRL) && modifier_held(VK_ALT) } {
                // Only intercept (swallow) the chord when there's a callback
                // to act on it — otherwise the user's Ctrl+Alt+C would be
                // lost entirely, so let it fall through to the target app.
                // Windows auto-repeat sends repeated WM_KEYDOWN/WM_SYSKEYDOWN
                // while the chord is held; fire the callback only on the
                // first physical keydown while still intercepting the repeats.
                if COPY_LAST_CB.get().is_some() {
                    if !COPY_LAST_KEY_DOWN.swap(true, Ordering::SeqCst) {
                        if let Some(cb) = COPY_LAST_CB.get() {
                            cb();
                        }
                    }
                    return LRESULT(1);
                }
            }
            if is_up
                && COPY_LAST_KEY_DOWN.swap(false, Ordering::SeqCst)
                && unsafe { modifier_held(VK_CTRL) && modifier_held(VK_ALT) }
            {
                return LRESULT(1);
            }
        }

        if vk == SUB_APP_VK.load(Ordering::Relaxed) {
            let mods = SUB_APP_MODS.load(Ordering::Relaxed);
            let chord = unsafe {
                modifier_held(VK_CTRL) == (mods & 1 != 0)
                    && modifier_held(VK_ALT) == (mods & 2 != 0)
                    && modifier_held(VK_SHIFT) == (mods & 4 != 0)
                    && modifier_held(91) == (mods & 8 != 0)
            };
            if is_down && chord && SUB_APP_CB.get().is_some() {
                if !SUB_APP_KEY_DOWN.swap(true, Ordering::SeqCst) {
                    if let Some(cb) = SUB_APP_CB.get() {
                        cb();
                    }
                }
                return LRESULT(1);
            }
            if is_up && SUB_APP_KEY_DOWN.swap(false, Ordering::SeqCst) && chord {
                return LRESULT(1);
            }
        }

        if !is_injected && is_down && !MODIFIER_VKS.contains(&vk) {
            if vk == VK_BACK {
                if unsafe { modifier_held(VK_CTRL) || modifier_held(VK_ALT) } {
                    crate::core::injection::reset_injection_history();
                } else {
                    let hwnd = unsafe { GetForegroundWindow().0 as usize };
                    crate::core::injection::backspace_injection_history(hwnd);
                }
            } else if (vk == VK_RETURN
                && !unsafe {
                    modifier_held(VK_SHIFT) || modifier_held(VK_CTRL) || modifier_held(VK_ALT)
                })
                || is_cursor_movement_key(vk)
                || unsafe { modifier_held(VK_CTRL) || modifier_held(VK_ALT) }
            {
                crate::core::injection::reset_injection_history();
            } else if let Some(ch) = vk_to_char(vk) {
                let hwnd = unsafe { GetForegroundWindow().0 as usize };
                crate::core::injection::append_or_reset_injection_history(hwnd, ch);
            } else {
                crate::core::injection::reset_injection_history();
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

fn should_suppress_synthetic_paste_event(is_injected: bool, now: u64, suppress_until: u64) -> bool {
    is_injected && now < suppress_until
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_only_bindings_are_unavailable() {
        for key in ["ControlLeft", "AltLeft", "ShiftLeft", "MetaLeft"] {
            assert!(!is_hotkey_available(&[key.to_string()]).unwrap());
        }
    }

    #[test]
    fn paste_suppression_only_skips_injected_events_before_deadline() {
        assert!(should_suppress_synthetic_paste_event(true, 499, 500));
        assert!(!should_suppress_synthetic_paste_event(true, 500, 500));
        assert!(!should_suppress_synthetic_paste_event(false, 499, 500));
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
    let _ = PRESS_CB.set(Box::new(on_press));
    let _ = RELEASE_CB.set(Box::new(on_release));
    let _ = HANDLESS_CB.set(Box::new(on_handless));
    let _ = CANCEL_CB.set(Box::new(on_cancel));
    let _ = ESCAPE_CB.set(Box::new(on_escape));
    let _ = COPY_LAST_CB.set(Box::new(on_copy_last));
    let _ = SUB_APP_CB.set(Box::new(on_capture_sub_app));

    unsafe {
        let probe = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)
            .map_err(|e| format!("Failed to install keyboard hook: {e}"))?;
        windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(probe).ok();
    }

    let handle = std::thread::spawn(|| unsafe {
        let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
            Ok(h) => h,
            Err(e) => {
                log::error!("SetWindowsHookExW failed on hook thread: {e}");
                return;
            }
        };

        CAPS_LOCK_ON.store((GetKeyState(0x14) & 0x0001) != 0, Ordering::SeqCst);

        let mut msg = MSG::default();
        loop {
            let status = GetMessageW(&mut msg, None, 0, 0).0;
            if status == -1 {
                log::error!("GetMessageW failed in hotkey hook thread");
                break;
            }
            if status == 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook).ok();
    });

    Ok(handle)
}
