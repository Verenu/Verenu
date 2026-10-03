use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const VK_BACK: u32 = 0x08; // Backspace
const VK_ESCAPE: u32 = 0x1B; // Escape
const VK_SPACE: u32 = 0x20; // Spacebar
const VK_SHIFT: u32 = 0x10; // VK_SHIFT
const VK_CTRL: u32 = 0x11; // VK_CONTROL (generic, used with modifier_held)
const VK_ALT: u32 = 0x12; // VK_MENU (generic, used with modifier_held)
const VK_RETURN: u32 = 0x0D; // Enter
const VK_C: u32 = 0x43; // 'C' — used by the Ctrl+Alt+C copy-last-dictation shortcut

// Side-specific modifier VK codes that should never trigger a history update.
// Generic codes (0x10/0x11/0x12) are omitted: the !is_injected guard already
// filters all synthetic input where those codes appear, so only side-specific
// codes reach this path from real physical key presses.
static MODIFIER_VKS: &[u32] = &[
    0xA0, 0xA1, // VK_LSHIFT, VK_RSHIFT
    0xA2, 0xA3, // VK_LCONTROL, VK_RCONTROL
    0xA4, 0xA5, // VK_LMENU, VK_RMENU
    0x5B, 0x5C, // VK_LWIN, VK_RWIN
    0x14, 0x90, 0x91, // VK_CAPITAL, VK_NUMLOCK, VK_SCROLL
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

// Returns true if the given specific-side VK (or its mirror) is currently held.
// Uses generic VKs for Shift/Ctrl/Alt so either side satisfies the check.
// Win key has no generic VK, so both sides are checked explicitly.
unsafe fn modifier_held(vk: u32) -> bool {
    let held = |v: u32| -> bool { (GetAsyncKeyState(v as i32) & 0x8000u16 as i16) != 0 };
    match vk {
        160 | 161 => held(16),           // L/RShift -> VK_SHIFT
        162 | 163 => held(17),           // L/RControl -> VK_CONTROL
        164 | 165 => held(18),           // L/RMenu -> VK_MENU
        91 | 92 => held(91) || held(92), // LWin / RWin (no generic VK_WIN)
        _ => held(vk),
    }
}

// Returns true if the hook vkCode matches the configured key, including the
// mirror side for modifiers (so LCtrl binding also matches RCtrl events).
fn vk_matches(vk: u32, key: u32) -> bool {
    match key {
        160 | 161 => vk == 160 || vk == 161,
        162 | 163 => vk == 162 || vk == 163,
        164 | 165 => vk == 164 || vk == 165,
        91 | 92 => vk == 91 || vk == 92,
        _ => vk == key,
    }
}

fn is_cursor_movement_key(vk: u32) -> bool {
    matches!(
        vk,
        0x21..=0x28 | // PgUp, PgDn, End, Home, Left, Up, Right, Down
        0x2D |        // Insert
        0x2E // Delete (forward)
    )
}

#[inline]
fn map_vk_to_scan_code(vk: u32, layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL) -> u32 {
    // windows crate (0.61.x) wraps MapVirtualKeyExW with Option<HKL>.
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
    // windows crate (0.61.x) wrapper derives cchBuff from buff.len().
    unsafe { ToUnicodeEx(vk, scan, state, buff, 0, Some(layout)) }
}

/// Maps a VK code to the character it produces (US QWERTY layout).
/// Letters are always returned lowercase - case doesn't affect sentence-ender
/// or whitespace checks downstream. Returns None for keys with no stable
/// printable character (numpad, function keys, etc.); those reset history.
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
            state[0xA0] |= 0x80; // VK_LSHIFT
            state[0x10] |= 0x80; // VK_SHIFT
        }
        if GetAsyncKeyState(0xA1) < 0 {
            state[0xA1] |= 0x80; // VK_RSHIFT
            state[0x10] |= 0x80;
        }
        if GetAsyncKeyState(0xA2) < 0 {
            state[0xA2] |= 0x80; // VK_LCONTROL
            state[0x11] |= 0x80; // VK_CONTROL
        }
        if GetAsyncKeyState(0xA3) < 0 {
            state[0xA3] |= 0x80; // VK_RCONTROL
            state[0x11] |= 0x80;
        }
        if GetAsyncKeyState(0xA4) < 0 {
            state[0xA4] |= 0x80; // VK_LMENU
            state[0x12] |= 0x80; // VK_MENU
        }
        if GetAsyncKeyState(0xA5) < 0 {
            state[0xA5] |= 0x80; // VK_RMENU (AltGr)
            state[0x12] |= 0x80;
        }
        // Caps Lock is a toggle key; low-order bit indicates toggle state.
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
            // Flush dead-key compose state with a neutral key so subsequent
            // translations are not polluted by stale composition state.
            let neutral_vk = 0x20u32; // VK_SPACE
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
    // Windows exposes reservations only for modifiers plus one trigger key.
    // Arbitrary multi-key chords are recognized by the low-level hook.
    if regular.len() != 1 {
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

static KEYS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(0);
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

// Cross-thread reset request: `reset_chord_state()` is called from the async
// pipeline task, which cannot reach the hook thread's thread-local directly.
static RESET_REQUESTED: AtomicBool = AtomicBool::new(false);

// Cross-thread request to force both chord keys' down/ownership bookkeeping
// to false, regardless of gesture state. Unlike RESET_REQUESTED (gesture
// state only — see reset_gesture_state's own doc comment on why it never
// touches physical-key state), this is a deliberately blunter reset: it's
// only ever set by force_release_win_key(), after GetAsyncKeyState has
// already confirmed the Win key reads stuck down at the OS level, so there's
// no risk of prematurely forgetting a legitimately-still-suppressed key.
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

// Requests that mid-chord gesture/timing state be cleared. Called from the
// Tokio handler after a handsfree stop-via-cancel so the still-open
// double-tap window can't accidentally start a fresh handsfree session on a
// stray second key press. Applied by the hook thread at the top of its next
// invocation (only gesture/timing state — see `reset_gesture_state`).
pub fn reset_chord_state() {
    RESET_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn set_handless_active(v: bool) {
    HANDLESS_ACTIVE.store(v, Ordering::SeqCst);
}

// 0 = not processing. Set once, at Stopping -> Processing; cleared via
// compare-exchange (a no-op if the generation doesn't match, so a stale,
// superseded task's cleanup can never clobber a newer generation's flag).
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

/// Current Caps Lock toggle state, tracked from the hook thread.
pub fn caps_lock_is_on() -> bool {
    CAPS_LOCK_ON.load(Ordering::SeqCst)
}

/// Live OS-level check (not our own bookkeeping) — true if Windows currently
/// thinks either Win key is held. Reuses the same `GetAsyncKeyState` primitive
/// `modifier_held` already uses for Win (VK 91/92, no generic VK_WIN).
pub fn is_win_key_down() -> bool {
    unsafe { modifier_held(91) }
}

/// Recovery action for a stuck Win key (confirmed via `is_win_key_down()`
/// first) — called right before a paste so a leftover "Win held" OS state
/// can't turn the paste's Ctrl+V into a Win-shortcut. Synthesizes a real
/// keyup for both Win keys (Windows should honor this as authoritative
/// regardless of why its internal state was wrong) and asks the hook thread
/// to forget its own chord-key ownership bookkeeping too, in case the hook
/// itself still thinks it's holding/suppressing the key.
pub fn force_release_win_key() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LWIN, VK_RWIN,
    };

    // Win is an "extended key" per SendInput's own contract — omitting
    // KEYEVENTF_EXTENDEDKEY can leave the OS's shell-hotkey state machine
    // out of sync even when GetAsyncKeyState reports the key up.
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

// Seeded once at hook-thread startup and updated on every hook callback (see
// `start` and `hook_proc`) - both writes happen on the hook's dedicated
// message-pumping thread, where `GetKeyState`'s toggle bit is reliably in
// sync. Querying `GetKeyState` directly from elsewhere (e.g. the Tokio
// pipeline thread, which has no message queue of its own) would not reflect
// real toggle state. Backs `caps_lock_is_on()` for the optional caps-lock
// output-uppercasing setting.
static CAPS_LOCK_ON: AtomicBool = AtomicBool::new(false);

static HANDLESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static ESCAPE_CANCELLED: AtomicBool = AtomicBool::new(false);
static ESCAPE_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static ESCAPE_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

// Ctrl+Alt+C: always-available fallback to re-copy the last dictation to the
// clipboard, in case paste failed in a way the pipeline's own detection
// missed. Not chord-based (no hold/release) — a plain keydown fires it,
// mirroring how Escape is handled below.
static COPY_LAST_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static COPY_LAST_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

// Invoked by the configured sub-app capture chord in the keyboard hook.
static SUB_APP_CB: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static SUB_APP_KEY_DOWN: AtomicBool = AtomicBool::new(false);
// Sub-app capture chord: key VK plus modifier bits (1 Ctrl, 2 Alt, 4 Shift, 8 Win).
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

        // This thread pumps messages (see `start`'s GetMessageW loop), so
        // GetKeyState's toggle bit is reliably in sync here.
        CAPS_LOCK_ON.store((GetKeyState(0x14) & 0x0001) != 0, Ordering::SeqCst);

        let generation = CONFIG_GENERATION.load(Ordering::Acquire);
        if LOCAL_GENERATION.with(|local| local.get() != generation) {
            // Never wait for another thread inside WH_KEYBOARD_LL.
            if let Ok(keys) = KEYS.try_lock() {
                let keys = if keys.is_empty() {
                    vec![162, 91]
                } else {
                    keys.clone()
                };
                CHORD_MACHINE
                    .with(|m| *m.borrow_mut() = ChordStateMachine::with_key_count(keys.len()));
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
                // Escape already handled cancellation while the chord was
                // held - don't also fire release/cancel now that it's up.
                action = None;
            }

            if edge == KeyEdge::Down
                && disposition == KeyDisposition::Suppress
                && menu_key_passed_through
                && LOCAL_KEYS.with(|keys| keys.borrow().iter().all(|id| MODIFIER_VKS.contains(id)))
            {
                // Keep the modifier-only Win/Alt chord's OS edges balanced.
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
            // Passthrough falls through to the rest of hook_proc below.
        }

        // While the hold-to-talk chord is active, Space converts it into the
        // existing hands-free mode. Consume both Space edges so the trigger
        // does not leak into the focused application.
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
            // Swallow the C keyup only while the chord modifiers are still
            // held (the normal release order: C first, then Ctrl/Alt). If the
            // user released Ctrl/Alt first, letting the C keyup pass keeps
            // the target app's modifier/keyup bookkeeping consistent — the
            // down was already suppressed, so it's just an orphaned keyup.
            if is_up
                && COPY_LAST_KEY_DOWN.swap(false, Ordering::SeqCst)
                && unsafe { modifier_held(VK_CTRL) && modifier_held(VK_ALT) }
            {
                return LRESULT(1);
            }
        }

        if vk == SUB_APP_VK.load(Ordering::Relaxed) {
            // The sub-app capture chord (default Ctrl+Alt+Shift+S). Modifiers
            // must match exactly so Ctrl+Alt+S never fires Ctrl+Alt+Shift+S.
            // Same handling as copy-last: fire once per physical press, and
            // swallow the chord only because a callback consumes it. The
            // capture itself runs on the async hotkey loop, not in the hook.
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

        // Update injection history for real user keystrokes only.
        // Synthetic events (LLKHF_INJECTED) are skipped - this prevents our own
        // Ctrl+V paste and any app-generated keyboard events from corrupting the
        // history that backs backspace recovery.
        let is_injected = (kb.flags.0 & LLKHF_INJECTED.0) != 0;
        if !is_injected && is_down && !MODIFIER_VKS.contains(&vk) {
            if vk == VK_BACK {
                // Ctrl+Backspace and Alt+Backspace both delete a whole word -
                // unknown char count, so reset entirely. Plain Backspace pops
                // just the last character to keep context accurate.
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
                // Keyboard shortcut (Ctrl+Z, Ctrl+A, etc.) - context unknown.
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

    // Verify the hook can be installed before spawning the thread so the caller
    // gets a synchronous error instead of a silent panic on a background thread.
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

        // Seed the cached state immediately: SetWindowsHookExW just gave this
        // thread a message queue, so GetKeyState's toggle bit already reflects
        // the real current state here, before the first key event arrives.
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
