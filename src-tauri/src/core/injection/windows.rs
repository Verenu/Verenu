use super::*;

fn clipboard_sequence() -> u32 {
    unsafe { ::windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

struct ChunkBackend<'a> {
    target_hwnd: usize,
    restore: &'a mut ClipboardRestoreGuard,
}
impl chunks::PasteBackend for ChunkBackend<'_> {
    async fn check(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(crate::core::window_context::get_foreground_hwnd() == self.target_hwnd, "Paste target lost focus");
        if let Some(expected) = self.restore.expected_sequence {
            anyhow::ensure!(clipboard_sequence() == expected, "Clipboard changed during paste");
        }
        anyhow::ensure!(!crate::core::hotkey::is_win_key_down(), "Release the Windows key before pasting");
        Ok(())
    }
    async fn write(&mut self, text: &str) -> anyhow::Result<()> {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        unsafe { write_clipboard_unicode_tracking(&wide, Some(&mut self.restore.expected_sequence)) }
    }
    async fn paste(&mut self) -> anyhow::Result<()> {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_LMENU, VK_V};
        crate::core::hotkey::begin_synthetic_paste_suppression(500);
        {
            let release = |vk| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: KEYEVENTF_KEYUP, ..Default::default() } },
            };
            let events = [release(VK_LMENU), release(VK_CONTROL)];
            unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
        }
        tokio::time::sleep(std::time::Duration::from_millis(MODIFIER_GAP_MS)).await;
        <Self as chunks::PasteBackend>::check(self).await?;
        let key = |vk, up| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
        };
        let events = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
        if sent != events.len() as u32 {
            let release = [key(VK_V, true), key(VK_CONTROL, true)];
            unsafe { SendInput(&release, std::mem::size_of::<INPUT>() as i32) };
            anyhow::bail!("Could not dispatch paste keys");
        }
        Ok(())
    }
}

struct SavedClipboard {
    entries: Vec<(u32, Vec<u8>)>,
}

struct ClipboardRestoreGuard {
    saved: Option<SavedClipboard>,
    expected_sequence: Option<u32>,
}
impl ClipboardRestoreGuard {
    fn new(saved: SavedClipboard) -> Self {
        Self { saved: Some(saved), expected_sequence: None }
    }
    fn restore_now(&mut self) {
        if self.expected_sequence.is_some_and(|expected| expected != clipboard_sequence()) {
            self.saved = None;
        }
        if let Some(saved) = self.saved.take() {
            unsafe {
                restore_clipboard_all(&saved);
            }
        }
    }
}
impl Drop for ClipboardRestoreGuard {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            unsafe {
                restore_clipboard_all(&saved);
            }
        }
    }
}

unsafe fn save_clipboard_all() -> SavedClipboard {
    use ::windows::Win32::Foundation::HGLOBAL;
    use ::windows::Win32::System::DataExchange::{
        CloseClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
    };
    use ::windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const MAX_FORMAT_BYTES: usize = 32 * 1024 * 1024;

    let mut entries = Vec::new();

    let opened = (0..3).any(|i| {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(CLIPBOARD_OPEN_RETRY_MS));
        }
        OpenClipboard(None).is_ok()
    });
    if !opened {
        return SavedClipboard { entries };
    }

    let mut fmt = 0u32;
    loop {
        fmt = EnumClipboardFormats(fmt);
        if fmt == 0 {
            break;
        }
        if matches!(
            fmt,
            CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_ENHMETAFILE
        ) {
            continue;
        }
        if let Ok(h) = GetClipboardData(fmt) {
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            if size > 0 && size <= MAX_FORMAT_BYTES {
                let ptr = GlobalLock(hg) as *const u8;
                if !ptr.is_null() {
                    let data = std::slice::from_raw_parts(ptr, size).to_vec();
                    let _ = GlobalUnlock(hg);
                    entries.push((fmt, data));
                }
            }
        }
    }

    CloseClipboard().ok();
    SavedClipboard { entries }
}

unsafe fn restore_clipboard_all(saved: &SavedClipboard) {
    use ::windows::Win32::Foundation::{GlobalFree, HANDLE};
    use ::windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use ::windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    if saved.entries.is_empty() {
        return;
    }

    for attempt in 0..CLIPBOARD_RESTORE_ATTEMPTS {
        let opened = (0..3).any(|i| {
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(CLIPBOARD_OPEN_RETRY_MS));
            }
            OpenClipboard(None).is_ok()
        });
        if !opened {
            if attempt + 1 == CLIPBOARD_RESTORE_ATTEMPTS {
                log::warn!("clipboard restore failed: OpenClipboard unavailable");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(CLIPBOARD_RESTORE_RETRY_MS));
            continue;
        }

        EmptyClipboard().ok();
        let mut restored = 0usize;
        for (fmt, data) in &saved.entries {
            if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, data.len()) {
                let ptr = GlobalLock(hg) as *mut u8;
                if ptr.is_null() {
                    let _ = GlobalFree(Some(hg));
                    continue;
                }
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                let _ = GlobalUnlock(hg);
                if SetClipboardData(*fmt, Some(HANDLE(hg.0))).is_ok() {
                    restored += 1;
                } else {
                    let _ = GlobalFree(Some(hg));
                }
            }
        }
        CloseClipboard().ok();

        if restored == saved.entries.len() {
            return;
        }

        log::warn!(
            "clipboard restore incomplete: restored {} of {} formats",
            restored,
            saved.entries.len()
        );
        if attempt + 1 < CLIPBOARD_RESTORE_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(CLIPBOARD_RESTORE_RETRY_MS));
        }
    }
}

unsafe fn write_clipboard_unicode(data: &[u16]) -> anyhow::Result<()> {
    write_clipboard_unicode_tracking(data, None)
}

unsafe fn write_clipboard_unicode_tracking(
    data: &[u16],
    owned_sequence: Option<&mut Option<u32>>,
) -> anyhow::Result<()> {
    use ::windows::Win32::Foundation::{GlobalFree, HANDLE};
    use ::windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use ::windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    let mut changed = false;
    let result = (|| {
        if OpenClipboard(None).is_ok() {
            EmptyClipboard().ok();
            changed = true;
            let hg = match GlobalAlloc(GMEM_MOVEABLE, data.len() * 2) {
                Ok(hg) => hg,
                Err(e) => {
                    CloseClipboard().ok();
                    return Err(anyhow::anyhow!("GlobalAlloc failed: {e}"));
                }
            };
            let ptr = GlobalLock(hg) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(hg));
                CloseClipboard().ok();
                return Err(anyhow::anyhow!("GlobalLock failed"));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            let _ = GlobalUnlock(hg);
            if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hg.0))) {
                let _ = GlobalFree(Some(hg));
                CloseClipboard().ok();
                return Err(anyhow::anyhow!("SetClipboardData failed: {e}"));
            }
            CloseClipboard().ok();
            Ok(())
        } else {
            Err(anyhow::anyhow!("OpenClipboard failed"))
        }
    })();
    if changed {
        if let Some(sequence) = owned_sequence {
            *sequence = Some(clipboard_sequence());
        }
    }
    result
}

pub(crate) async fn read_clipboard_text() -> Option<String> {
    use ::windows::Win32::Foundation::HGLOBAL;
    use ::windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use ::windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    let mut opened = false;
    for i in 0..3 {
        if i > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(CLIPBOARD_OPEN_RETRY_MS)).await;
        }
        if unsafe { OpenClipboard(None) }.is_ok() {
            opened = true;
            break;
        }
    }
    if !opened {
        return None;
    }

    let mut text: Option<String> = None;
    unsafe {
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let hg = HGLOBAL(h.0);
            let size = GlobalSize(hg);
            if size == 0 {
                text = Some(String::new());
            } else {
                let ptr = GlobalLock(hg) as *const u16;
                if !ptr.is_null() {
                    let units = size / 2;
                    let slice = std::slice::from_raw_parts(ptr, units);
                    let end = slice.iter().position(|&c| c == 0).unwrap_or(units);
                    text = Some(String::from_utf16_lossy(&slice[..end]));
                    let _ = GlobalUnlock(hg);
                }
            }
        }
        CloseClipboard().ok();
    }
    text
}

pub(super) async fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    for attempt in 0..3u32 {
        if unsafe { write_clipboard_unicode(&wide) }.is_ok() {
            return Ok(());
        }
        if attempt < 2 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }
    anyhow::bail!("copy_to_clipboard: clipboard held after 3 attempts")
}

const PASTE_VERIFY_ATTEMPTS: u32 = 6;
const PASTE_VERIFY_RETRY_MS: u64 = 100;
const PASTE_VERIFY_RETRY_GROWTH_MS: u64 = 40;
const PASTE_VERIFY_MAX_RETRY_MS: u64 = 260;
const PASTE_VERIFY_FULLTEXT_ATTEMPTS: u32 = 2;

#[allow(unused_variables)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn inject_text(
    text: &str,
    target_hwnd: usize,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
    paste_in_chunks: bool,
) -> anyhow::Result<InjectionOutcome> {
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_CONTROL, VK_LMENU, VK_LWIN, VK_RWIN, VK_V,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetShellWindow, GetWindowThreadProcessId,
        SetForegroundWindow, GUITHREADINFO,
    };

    let shell_hwnd = unsafe { GetShellWindow() }.0 as usize;
    let current_foreground_hwnd = unsafe { GetForegroundWindow() }.0 as usize;
    let is_desktop_shell = target_hwnd != 0 && shell_hwnd == target_hwnd;
    log::debug!(
        "inject: target_hwnd={} shell_hwnd={} current_foreground_hwnd={} is_desktop_shell={}",
        target_hwnd,
        shell_hwnd,
        current_foreground_hwnd,
        is_desktop_shell
    );
    if target_hwnd == 0 || is_desktop_shell {
        log::warn!("inject: aborting — no meaningful paste target (hwnd=0 or desktop/shell)");
        anyhow::bail!("Nothing was focused to paste into");
    }

    let target_thread_id =
        unsafe { GetWindowThreadProcessId(HWND(target_hwnd as *mut core::ffi::c_void), None) };
    let has_focus = target_thread_id != 0 && {
        let mut gti = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetGUIThreadInfo(target_thread_id, &mut gti) }.is_ok()
            && gti.hwndFocus.0 as usize != 0
    };
    log::debug!(
        "inject: target_thread_id={} has_focus={}",
        target_thread_id,
        has_focus
    );
    if !has_focus {
        log::warn!("inject: aborting — no control has keyboard focus in target thread");
        anyhow::bail!("Nothing was focused to paste into");
    }

    let _injection_guard = super::injection_lock().lock().await;

    let clipboard_before_snapshot = clipboard_sequence();
    let saved = unsafe { save_clipboard_all() };
    let mut restore_guard = ClipboardRestoreGuard::new(saved);
    if paste_in_chunks { restore_guard.expected_sequence = Some(clipboard_before_snapshot); }

    if target_hwnd != 0 {
        let _ = unsafe { SetForegroundWindow(HWND(target_hwnd as *mut core::ffi::c_void)) };
        tokio::time::sleep(tokio::time::Duration::from_millis(REFOCUS_SETTLE_MS)).await;
    }

    let ki = |vk, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    let mut injection_probe = if contextual_caps || auto_spacing {
        crate::core::context_probe::read_injection_context_probe().await
    } else {
        unavailable_injection_probe()
    };

    if (contextual_caps || auto_spacing)
        && crate::core::window_context::get_foreground_hwnd() != target_hwnd
    {
        log::debug!(
            "injection: rejecting stale UIA probe target_hwnd={} probe_pid={}",
            target_hwnd,
            injection_probe.target_id
        );
        injection_probe = unavailable_injection_probe();
    }
    let (adjusted, context_kind, case_decision) = apply_probe_adjustments(
        text,
        contextual_caps,
        auto_spacing,
        profile,
        language,
        protected_initial_case,
        &injection_probe,
    );

    if paste_in_chunks {
        for _ in 0..WIN_KEY_GRACE_POLL_ATTEMPTS {
            if !crate::core::hotkey::is_win_key_down() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(WIN_KEY_GRACE_POLL_MS)).await;
        }
        let mut backend = ChunkBackend { target_hwnd, restore: &mut restore_guard };
        let result = chunks::paste(&mut backend, &adjusted).await;
        restore_guard.restore_now();
        result?;
        if let Ok(mut history) = last_injection().lock() {
            let mut tail = adjusted.clone();
            trim_tail_to_limit(&mut tail);
            *history = CursorContextState::Known { hwnd: target_hwnd, tail, instant: Instant::now() };
        }
        return Ok(InjectionOutcome {
            text: adjusted, context_state: context_kind.as_str(), case_decision: case_decision.as_str(),
            probe_source: injection_probe.source.as_str(), selection_state: injection_probe.selection_state.as_str(),
        });
    }
    let text_to_inject = adjusted.as_str();
    let wide: Vec<u16> = text_to_inject
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut clipboard_written = false;
    for attempt in 0..3u32 {
        if unsafe { write_clipboard_unicode(&wide) }.is_ok() {
            clipboard_written = true;
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(tokio::time::Duration::from_millis(CLIPBOARD_WRITE_RETRY_MS)).await;
        }
    }
    if !clipboard_written {
        restore_guard.restore_now();
        return Err(anyhow::anyhow!(
            "OpenClipboard failed after 3 attempts - clipboard held by another process"
        ));
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(
        CLIPBOARD_WRITE_SETTLE_MS,
    ))
    .await;

    let clear = [
        ki(VK_CONTROL, 0),
        ki(VK_LMENU, KEYEVENTF_KEYUP.0),
        ki(VK_CONTROL, KEYEVENTF_KEYUP.0),
        ki(VK_LWIN, KEYEVENTF_KEYUP.0 | KEYEVENTF_EXTENDEDKEY.0),
        ki(VK_RWIN, KEYEVENTF_KEYUP.0 | KEYEVENTF_EXTENDEDKEY.0),
    ];
    unsafe { SendInput(&clear, std::mem::size_of::<INPUT>() as i32) };

    tokio::time::sleep(tokio::time::Duration::from_millis(MODIFIER_GAP_MS)).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(
        WIN_KEY_RELEASE_SETTLE_MS,
    ))
    .await;

    let win_down_after_clear = crate::core::hotkey::is_win_key_down();
    log::debug!("inject: win_key_down_after_clear={win_down_after_clear}");
    if win_down_after_clear {
        let mut still_down = true;
        let mut poll_attempts_used = 0u32;
        for _ in 0..WIN_KEY_GRACE_POLL_ATTEMPTS {
            tokio::time::sleep(tokio::time::Duration::from_millis(WIN_KEY_GRACE_POLL_MS)).await;
            poll_attempts_used += 1;
            if !crate::core::hotkey::is_win_key_down() {
                still_down = false;
                break;
            }
        }
        log::debug!(
            "inject: win_key grace poll attempts_used={poll_attempts_used} still_down={still_down}"
        );
        if still_down {
            log::warn!("inject: Win key reads stuck down before paste, attempting recovery");
            crate::core::hotkey::force_release_win_key();
            tokio::time::sleep(tokio::time::Duration::from_millis(MODIFIER_GAP_MS)).await;
            let still_stuck = crate::core::hotkey::is_win_key_down();
            log::debug!("inject: win_key_down_after_force_release={still_stuck}");
            if still_stuck {
                restore_guard.restore_now();
                log::warn!("inject: aborting paste — Win key still stuck after forced recovery");
                return Err(anyhow::anyhow!(
                    "Windows key appears stuck down — release it and try again"
                ));
            }
        }
    }

    if crate::core::hotkey::is_win_key_down() {
        restore_guard.restore_now();
        log::warn!("inject: aborting — Win key down at the last check before Ctrl+V");
        return Err(anyhow::anyhow!(
            "Windows key appears stuck down — release it and try again"
        ));
    }

    log::debug!("inject: sending Ctrl+V");
    let paste = [
        ki(VK_CONTROL, 0),
        ki(VK_V, 0),
        ki(VK_V, KEYEVENTF_KEYUP.0),
        ki(VK_CONTROL, KEYEVENTF_KEYUP.0),
    ];
    unsafe { SendInput(&paste, std::mem::size_of::<INPUT>() as i32) };
    tokio::time::sleep(tokio::time::Duration::from_millis(PASTE_SETTLE_MS)).await;

    restore_guard.restore_now();

    if !adjusted.trim_end().is_empty() {
        let pre_injection_tail = injection_probe.context_tail.as_str();
        let mut verified = false;
        let mut saw_fresh_mismatch = false;
        let mut saw_frozen = false;

        for attempt in 0..PASTE_VERIFY_ATTEMPTS {
            let post_probe = crate::core::context_probe::read_injection_context_probe().await;

            if post_probe.source != ContextProbeSource::CaretLocal {
                verified = true;
            } else {
                let tail_frozen = post_probe.context_tail == pre_injection_tail;
                saw_frozen |= tail_frozen;
                if paste_tail_matches(&adjusted, &post_probe.context_tail) {
                    verified = true;
                } else if !tail_frozen {
                    saw_fresh_mismatch = true;
                }
            }

            log::debug!(
                "inject: post-paste verify attempt={} probe_source={} probe_tail_len={} injected_len={} verified={}",
                attempt + 1,
                post_probe.source.as_str(),
                post_probe.context_tail.chars().count(),
                adjusted.chars().count(),
                verified
            );

            if verified {
                break;
            }
            if attempt + 1 < PASTE_VERIFY_ATTEMPTS {
                let grow = PASTE_VERIFY_RETRY_MS + (attempt as u64) * PASTE_VERIFY_RETRY_GROWTH_MS;
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    grow.min(PASTE_VERIFY_MAX_RETRY_MS),
                ))
                .await;
            }
        }

        if !verified {
            for _ in 0..PASTE_VERIFY_FULLTEXT_ATTEMPTS {
                if let Some(full_text) = crate::api::auto_learn::read_focused_text() {
                    if full_text_confirms_paste(&adjusted, &full_text) {
                        verified = true;
                        log::debug!("inject: post-paste verified via full-text match");
                        break;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(PASTE_VERIFY_RETRY_MS)).await;
            }
        }

        if !verified {
            if saw_fresh_mismatch {
                log::warn!("inject: aborting — post-paste verification tail mismatch");
                return Err(anyhow::anyhow!(
                    "Paste could not be verified — target text does not match"
                ));
            }
            log::warn!(
                "inject: paste unverified after {} attempts (saw_frozen={saw_frozen}) — treating as best-effort success",
                PASTE_VERIFY_ATTEMPTS,
            );
        }
    }

    if target_hwnd != 0 && !adjusted.is_empty() {
        if let Ok(mut guard) = last_injection().lock() {
            let mut tail = adjusted.clone();
            trim_tail_to_limit(&mut tail);
            *guard = CursorContextState::Known {
                hwnd: target_hwnd,
                tail,
                instant: Instant::now(),
            };
        }
    }

    Ok(InjectionOutcome {
        text: adjusted,
        context_state: context_kind.as_str(),
        case_decision: case_decision.as_str(),
        probe_source: injection_probe.source.as_str(),
        selection_state: injection_probe.selection_state.as_str(),
    })
}
