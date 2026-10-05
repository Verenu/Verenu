#[cfg(windows)]
use windows::Win32::Foundation::{HWND, MAX_PATH};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

#[cfg(windows)]
const BROWSER_EXES: &[(&str, &str)] = &[
    ("chrome.exe", "Google Chrome"),
    ("msedge.exe", "Microsoft Edge"),
    ("firefox.exe", "Firefox"),
    ("brave.exe", "Brave"),
    ("opera.exe", "Opera"),
    ("vivaldi.exe", "Vivaldi"),
    ("arc.exe", "Arc"),
    ("waterfox.exe", "Waterfox"),
    ("librewolf.exe", "LibreWolf"),
];

// Process name convention on macOS: "<localized name>.app" lowercased.
#[cfg(target_os = "macos")]
const BROWSER_EXES: &[(&str, &str)] = &[
    ("google chrome.app", "Google Chrome"),
    ("safari.app", "Safari"),
    ("microsoft edge.app", "Microsoft Edge"),
    ("firefox.app", "Firefox"),
    ("brave browser.app", "Brave"),
    ("arc.app", "Arc"),
];

#[cfg(target_os = "linux")]
const BROWSER_EXES: &[(&str, &str)] = &[
    ("google-chrome", "Google Chrome"), ("google-chrome-stable", "Google Chrome"),
    ("chromium", "Chromium"), ("brave-browser", "Brave"), ("firefox", "Firefox"),
    ("librewolf", "LibreWolf"), ("microsoft-edge", "Microsoft Edge"),
    ("vivaldi-stable", "Vivaldi"), ("zen", "Zen Browser"),
    ("chrome", "Google Chrome"), ("chromium-browser", "Chromium"),
    ("brave", "Brave"), ("brave-browser-beta", "Brave"),
    ("brave-browser-nightly", "Brave"), ("firefox-esr", "Firefox"),
    ("firefox-bin", "Firefox"), ("waterfox", "Waterfox"),
    ("microsoft-edge-stable", "Microsoft Edge"), ("microsoft-edge-beta", "Microsoft Edge"),
    ("microsoft-edge-dev", "Microsoft Edge"), ("vivaldi", "Vivaldi"),
    ("vivaldi-snapshot", "Vivaldi"), ("opera", "Opera"),
    ("google-chrome-beta", "Google Chrome"), ("google-chrome-unstable", "Google Chrome"),
    ("org.mozilla.firefox", "Firefox"), ("org.chromium.chromium", "Chromium"),
    ("com.google.chrome", "Google Chrome"), ("com.brave.browser", "Brave"),
    ("io.gitlab.librewolf-community", "LibreWolf"),
    ("app.zen_browser.zen", "Zen Browser"),
];

/// The focus target to refocus before paste. On Windows this is the foreground
/// `HWND`; on macOS it is the frontmost application's PID (both fit in a `usize`).
pub fn get_foreground_hwnd() -> usize {
    #[cfg(windows)]
    unsafe {
        let hwnd = GetForegroundWindow();
        hwnd.0 as usize
    }
    #[cfg(target_os = "macos")]
    {
        // Avoid CGWindowListCopyWindowInfo on the hotkey/keypress path — it
        // communicates synchronously with WindowServer for all on-screen windows
        // and introduces several ms of typing latency, which can trigger the
        // macOS event tap watchdog timeout. frontmost_pid() is a single fast
        // NSWorkspace call and sufficient for both focus-target tracking and the
        // "same window?" comparisons in injection.rs (pid & 0xFFFFFFFF).
        crate::system::mac_app::frontmost_pid()
            .map(|p| p as usize)
            .unwrap_or(0)
    }
    #[cfg(target_os = "linux")]
    {
        crate::core::hyprland::active_window()
            .map(|window| window.pid as usize)
            .unwrap_or(0)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    0
}

/// Whether `process_name` (as returned by `get_process_name_for_hwnd`) is a
/// known browser — used to gate the address-bar domain probe so it's never
/// attempted against a non-browser foreground window.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn is_browser_exe(process_name: &str) -> bool {
    BROWSER_EXES.iter().any(|(exe, _)| exe.eq_ignore_ascii_case(process_name))
}

/// A Wayland app ID may differ from its executable. Check both exact
/// identities; never classify arbitrary Electron apps as browsers.
pub fn is_browser_target(process_name: &str, target_id: usize) -> bool {
    if is_browser_exe(process_name) {
        return true;
    }
    #[cfg(target_os = "linux")]
    return linux_executable_for_pid(target_id).is_some_and(|exe| is_browser_exe(&exe));
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target_id;
        false
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_executable_for_pid(target_id: usize) -> Option<String> {
    let pid = u32::try_from(target_id).ok().filter(|pid| *pid != 0)?;
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    // Running AppImages/package upgrades can unlink the old executable.
    Some(path.file_name()?.to_str()?.trim_end_matches(" (deleted)").to_ascii_lowercase())
}
#[cfg(target_os = "android")]
pub fn is_browser_exe(process_name: &str) -> bool {
    crate::android::is_browser_package(process_name)
}
#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn is_browser_exe(_process_name: &str) -> bool {
    false
}

#[cfg_attr(not(windows), allow(unused_variables))]
pub fn get_process_name_for_hwnd(hwnd: usize) -> Option<String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(hwnd as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return None;
        }

        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let process_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut size = buffer.len() as u32;

        if QueryFullProcessImageNameW(
            process_handle,
            windows::Win32::System::Threading::PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR::from_raw(buffer.as_mut_ptr()),
            &mut size,
        )
        .is_ok()
        {
            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            if let Some(name) = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
            {
                return Some(name.to_lowercase());
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        // On macOS WindowTarget stores a PID. Resolve that PID directly so a
        // later pipeline stage cannot accidentally inspect the current
        // frontmost app after focus has moved.
        crate::system::mac_app::app_name_for_pid(hwnd as i32)
            .map(|n| format!("{}.app", n.to_lowercase()))
    }
    #[cfg(target_os = "linux")]
    {
        // Resolve the captured client even if focus has since moved; the live
        // foreground may be a different app by the time processing runs.
        let pid = u32::try_from(hwnd).ok().filter(|pid| *pid != 0)?;
        crate::core::hyprland::active_window()
            .filter(|window| window.pid == pid)
            .or_else(|| crate::core::hyprland::window_by_pid(pid))
            .map(|window| window.class_name.to_ascii_lowercase())
            .filter(|class| !class.is_empty())
    }
    // Android has no window handles; Kotlin reports the foreground package
    // when a recording starts and that is the "executable" Contexts match on.
    #[cfg(target_os = "android")]
    {
        let _ = hwnd;
        crate::android::bridge::dictation_package()
    }
    #[cfg(not(any(
        windows,
        target_os = "macos",
        target_os = "linux",
        target_os = "android"
    )))]
    None
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn verenu_macos_focused_window_title(
        pid: i32,
        out: *mut std::ffi::c_char,
        capacity: usize,
    ) -> i32;
}

/// Title of the captured target window (HWND on Windows, PID on macOS/Linux).
/// Used for sub-app matching and the cleanup prompt; never logged.
pub fn get_window_title(target_id: usize) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(target_id).ok().filter(|pid| *pid > 0)?;
        let mut buffer = vec![0 as std::ffi::c_char; 1024];
        let ok = unsafe { verenu_macos_focused_window_title(pid, buffer.as_mut_ptr(), buffer.len()) };
        if ok == 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, len) };
        return Some(String::from_utf8_lossy(bytes).into_owned()).filter(|t| !t.is_empty());
    }
    #[allow(unreachable_code)]
    get_window_title_platform(target_id)
}

fn get_window_title_platform(target_id: usize) -> Option<String> {
    #[cfg(windows)]
    unsafe {
        let hwnd = HWND(target_id as *mut core::ffi::c_void);
        if hwnd.0.is_null() {
            return None;
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "linux")]
        if let Some(window) = u32::try_from(target_id)
            .ok()
            .and_then(crate::core::hyprland::window_by_pid)
        {
            return Some(window.title);
        }
        let _ = target_id;
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
mod browser_identity_tests {
    #[test]
    fn recognizes_linux_browser_classes_channels_and_flatpaks_only() {
        for browser in ["Google-Chrome", "chrome", "brave", "firefox", "org.mozilla.firefox", "com.brave.Browser", "vivaldi-snapshot", "zen", "microsoft-edge-dev", "opera"] {
            assert!(super::is_browser_exe(browser), "browser identity: {browser}");
        }
        for app in ["com.t3tools.T3Code", "electron", "chromium-editor", "firefox-notes", "unknown"] {
            assert!(!super::is_browser_target(app, 0), "non-browser identity: {app}");
        }
    }
}
