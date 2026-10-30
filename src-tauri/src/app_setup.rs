use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "windows")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "windows")]
use std::time::Duration;

#[derive(Clone, Default)]
pub(crate) struct FrontendReadiness {
    pub(crate) main: Arc<AtomicBool>,
    pub(crate) pill: Arc<AtomicBool>,
    #[cfg(target_os = "linux")]
    pub(crate) main_navigation: Arc<std::sync::atomic::AtomicU32>,
    #[cfg(target_os = "linux")]
    pub(crate) pill_navigation: Arc<std::sync::atomic::AtomicU32>,
}

#[cfg(target_os = "linux")]
impl FrontendReadiness {
    pub(crate) fn main_page_started(&self) {
        use std::sync::atomic::Ordering;
        let mut current = self.main_navigation.load(Ordering::SeqCst);
        loop {
            let next = ((current >> 2).wrapping_add(1) & 0x3fff_ffff) << 2;
            match self.main_navigation.compare_exchange_weak(
                current,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(state) => current = state,
            }
        }
    }

    pub(crate) fn main_settings_generation(&self) -> u32 {
        self.main_navigation.load(std::sync::atomic::Ordering::SeqCst) >> 2
    }

    pub(crate) fn mark_main_settings_ready(&self, generation: u32) -> Result<(), String> {
        self.mark_main_ready(generation, 1)
    }

    pub(crate) fn mark_home_ready(&self, generation: u32) -> Result<(), String> {
        self.mark_main_ready(generation, 2)
    }

    fn mark_main_ready(&self, generation: u32, flag: u32) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        if generation > 0x3fff_ffff {
            return Err("Settings readiness belongs to an earlier page.".into());
        }
        let mut state = self.main_navigation.load(Ordering::SeqCst);
        loop {
            if state >> 2 != generation {
                return Err("Settings readiness belongs to an earlier page.".into());
            }
            match self.main_navigation.compare_exchange_weak(
                state, state | flag, Ordering::SeqCst, Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => state = actual,
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn main_settings_ready(&self) -> bool {
        self.main_navigation.load(std::sync::atomic::Ordering::SeqCst) & 1 != 0
    }

    pub(crate) fn main_and_home_ready(&self) -> bool {
        self.main_navigation.load(std::sync::atomic::Ordering::SeqCst) & 3 == 3
    }

    pub(crate) fn pill_page_started(&self) {
        use std::sync::atomic::Ordering;
        let mut current = self.pill_navigation.load(Ordering::SeqCst);
        loop {
            let next = ((current >> 1).wrapping_add(1) & 0x7fff_ffff) << 1;
            match self.pill_navigation.compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(state) => current = state,
            }
        }
    }

    pub(crate) fn pill_generation(&self) -> u32 {
        self.pill_navigation.load(std::sync::atomic::Ordering::SeqCst) >> 1
    }

    pub(crate) fn mark_pill_ready(&self, generation: u32) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        if generation > 0x7fff_ffff { return Err("Pill readiness belongs to an earlier page.".into()); }
        let expected = generation << 1;
        match self.pill_navigation.compare_exchange(expected, expected | 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => Ok(()),
            Err(state) if state == (expected | 1) => Ok(()),
            Err(_) => Err("Pill readiness belongs to an earlier page.".into()),
        }
    }

    pub(crate) fn current_pill_ready(&self) -> bool {
        self.pill_navigation.load(std::sync::atomic::Ordering::SeqCst) & 1 != 0
    }
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn main_settings_readiness_rejects_an_earlier_page() {
    let readiness = FrontendReadiness::default();
    readiness.main_page_started();
    let old = readiness.main_settings_generation();
    assert!(!readiness.main_settings_ready());
    readiness.mark_main_settings_ready(old).unwrap();
    assert!(readiness.main_settings_ready());
    assert!(!readiness.main_and_home_ready());
    readiness.mark_home_ready(old).unwrap();
    assert!(readiness.main_and_home_ready());
    readiness.main_page_started();
    assert!(!readiness.main_settings_ready());
    assert!(readiness.mark_main_settings_ready(old).is_err());
    assert!(readiness.mark_home_ready(old).is_err());
    assert!(!readiness.main_and_home_ready());
    assert!(!readiness.main_settings_ready());
    readiness
        .mark_main_settings_ready(readiness.main_settings_generation())
        .unwrap();
    assert!(readiness.main_settings_ready());
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn navigation_cannot_inherit_a_racing_old_settings_acknowledgement() {
    for _ in 0..500 {
        let readiness = FrontendReadiness::default();
        readiness.main_page_started();
        let old = readiness.main_settings_generation();
        let ack = readiness.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let other_start = start.clone();
        let worker = std::thread::spawn(move || {
            other_start.wait();
            let _ = ack.mark_main_settings_ready(old);
        });
        start.wait();
        readiness.main_page_started();
        worker.join().unwrap();
        assert!(!readiness.main_settings_ready());
    }
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn pill_readiness_rejects_an_earlier_page() {
    let readiness = FrontendReadiness::default();
    readiness.pill_page_started();
    let old = readiness.pill_generation();
    readiness.mark_pill_ready(old).unwrap();
    assert!(readiness.current_pill_ready());
    readiness.pill_page_started();
    assert!(!readiness.current_pill_ready());
    assert!(readiness.mark_pill_ready(old).is_err());
    assert!(!readiness.current_pill_ready());
    readiness.mark_pill_ready(readiness.pill_generation()).unwrap();
    assert!(readiness.current_pill_ready());
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn navigation_cannot_inherit_a_racing_old_home_acknowledgement() {
    for _ in 0..500 {
        let readiness = FrontendReadiness::default();
        readiness.main_page_started();
        let old = readiness.main_settings_generation();
        let ack = readiness.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let other_start = start.clone();
        let worker = std::thread::spawn(move || {
            other_start.wait();
            let _ = ack.mark_home_ready(old);
        });
        start.wait();
        readiness.main_page_started();
        worker.join().unwrap();
        readiness.mark_main_settings_ready(readiness.main_settings_generation()).unwrap();
        assert!(!readiness.main_and_home_ready());
    }
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn listing_local_models_does_not_create_a_data_directory() {
    const CHILD_MARKER: &str = "VERENU_TEST_READ_ONLY_MODEL_LIST";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let root = app_data_dir();
        assert!(!root.exists());
        assert!(!crate::local_stt::LocalTranscriptionManager::new()
            .list_models()
            .unwrap()
            .is_empty());
        assert!(!crate::local_llm::LocalLlmManager::new()
            .list_models()
            .unwrap()
            .is_empty());
        assert!(!root.exists());
        return;
    }
    let root = std::env::temp_dir().join(format!("verenu-read-only-models-{}", uuid::Uuid::new_v4()));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app_setup::listing_local_models_does_not_create_a_data_directory",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("VERENU_APP_DATA_DIR_OVERRIDE", &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.exists());
}

#[cfg(target_os = "windows")]
impl FrontendReadiness {
    fn main_ready(&self) -> bool {
        self.main.load(Ordering::Acquire)
    }

    #[cfg(debug_assertions)]
    fn reset(&self) {
        self.main.store(false, Ordering::Release);
        self.pill.store(false, Ordering::Release);
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if crate::is_dev_session() {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        #[cfg(desktop)]
        if w.is_minimized().unwrap_or(false) {
            w.unminimize().ok();
        }
        #[cfg(target_os = "macos")]
        {
            crate::system::mac_app::set_regular_activation_policy_on_main_thread(app);
            crate::system::mac_app::activate_current_app_on_main_thread(app);
        }
        #[cfg(target_os = "windows")]
        log::info!("Windows main HWND first show requested");
        w.show().ok();
        w.set_focus().ok();
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn hide_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        crate::system::mac_app::set_accessory_activation_policy_on_main_thread(app);
        w.hide().ok();
    }
}

fn app_data_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("VERENU_APP_DATA_DIR_OVERRIDE").map(std::path::PathBuf::from)
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn startup_recovery_was_attempted() -> bool {
    std::env::args_os().any(|arg| arg == "--startup-recovery-attempted")
}

#[cfg(all(debug_assertions, target_os = "windows"))]
async fn wait_for_dev_frontend() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(500))
        .timeout(Duration::from_secs(1))
        .build()
    else {
        return false;
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        if let Ok(response) = client.get("http://127.0.0.1:1420/").send().await {
            if response.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[cfg(target_os = "windows")]
pub(crate) fn start_frontend_watchdog(app: &AppHandle, readiness: FrontendReadiness) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let reveal_windows = || {
            show_main_window(&app);
            if !crate::pipeline::failover::offer_restored_capture_pill(&app) {
                crate::pipeline::show_pill(&app, "idle");
            }
        };

        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        while tokio::time::Instant::now() < deadline {
            if readiness.main_ready() {
                log::debug!("startup handshake completed");
                reveal_windows();
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if readiness.main_ready() {
            reveal_windows();
            return;
        }

        #[cfg(debug_assertions)]
        if wait_for_dev_frontend().await {
            readiness.reset();
            if let Some(window) = app.get_webview_window("main") {
                window.reload().ok();
            }

            let retry_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
            while tokio::time::Instant::now() < retry_deadline {
                if readiness.main_ready() {
                    reveal_windows();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        #[cfg(debug_assertions)]
        log::error!(
            "startup handshake did not complete in development; keeping the dev process alive for diagnosis"
        );
        reveal_windows();

        #[cfg(not(debug_assertions))]
        {
            if startup_recovery_was_attempted() {
                log::error!(
                    "startup handshake did not complete after the automatic recovery attempt; leaving the app running for diagnosis"
                );
                reveal_windows();
                return;
            }

            log::warn!(
                "startup handshake timed out: main_ready={} pill_ready={}; relaunching once",
                readiness.main.load(Ordering::Acquire),
                readiness.pill.load(Ordering::Acquire)
            );
            crate::app_tray::relaunch_for_startup_recovery(&app);
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn start_frontend_watchdog(app: &AppHandle, _readiness: FrontendReadiness) {
    #[cfg(target_os = "linux")]
    crate::pipeline::initialize_pill(app);
    #[cfg(target_os = "macos")]
    if !crate::pipeline::failover::offer_restored_capture_pill(app) {
        crate::pipeline::show_pill(app, "idle");
    }
    show_main_window(app);
}

#[cfg(windows)]
pub(crate) fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = app_data_dir_override() {
        return path;
    }
    std::env::var("APPDATA")
        .map(|p| std::path::PathBuf::from(p).join("Verenu"))
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

#[cfg(target_os = "macos")]
pub(crate) fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = app_data_dir_override() {
        return path;
    }
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support/Verenu"))
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = app_data_dir_override() {
        return path;
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|root| root.join("Verenu"))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

pub(crate) fn app_db_path() -> std::path::PathBuf {
    app_data_dir().join("verenu.db")
}

pub(crate) fn fatal_startup_error(message: &str) -> ! {
    log::error!("{message}");
    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

        let mut text: Vec<u16> = format!("{message}\n\nVerenu must close.")
            .encode_utf16()
            .collect();
        text.push(0);
        let mut title: Vec<u16> = "Verenu — startup failed".encode_utf16().collect();
        title.push(0);
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    panic!("{message}");
}

#[cfg(target_os = "windows")]
pub(crate) fn cleanup_update_helper_if_requested() {
    let Some(helper) = std::env::args_os().find_map(|arg| {
        let text = arg.to_string_lossy();
        text.strip_prefix("--cleanup-update-helper=")
            .map(std::path::PathBuf::from)
    }) else {
        return;
    };

    for _ in 0..20 {
        match std::fs::remove_file(&helper) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn wait_for_relaunch_parent_exit() {
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    let Some(parent_pid) = relaunch_parent_pid() else {
        return;
    };

    unsafe {
        let handle = match OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) {
            Ok(handle) => handle,
            Err(err) => {
                if err.code() != windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) {
                    early_startup_warn(&format!(
                        "Relaunch requested but could not open parent process {parent_pid}: {err}"
                    ));
                }
                return;
            }
        };

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let wait_result = loop {
            let result = WaitForSingleObject(handle, 5_000);
            if result != WAIT_TIMEOUT || std::time::Instant::now() >= deadline {
                break result;
            }
        };
        if wait_result != WAIT_OBJECT_0 {
            early_startup_warn(&format!(
                "Relaunch waited for parent process {parent_pid} but got result {}",
                wait_result.0
            ));
        }

        let _ = CloseHandle(handle);
    }
}

#[cfg(target_os = "windows")]
fn relaunch_parent_pid() -> Option<u32> {
    let mut args = std::env::args_os().skip(1);

    while let Some(arg) = args.next() {
        let Some(text) = arg.to_str() else {
            continue;
        };

        if let Some(value) = text.strip_prefix("--relaunch-parent-pid=") {
            if let Ok(pid) = value.parse::<u32>() {
                return Some(pid);
            }
            early_startup_warn(&format!(
                "Ignoring invalid relaunch parent pid argument: {value}"
            ));
            return None;
        }

        if text == "--relaunch-parent-pid" {
            let Some(value) = args.next() else {
                early_startup_warn("Ignoring missing relaunch parent pid value");
                return None;
            };

            match value.to_string_lossy().parse::<u32>() {
                Ok(pid) => return Some(pid),
                Err(_) => {
                    early_startup_warn(&format!(
                        "Ignoring invalid relaunch parent pid argument: {}",
                        value.to_string_lossy()
                    ));
                    return None;
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn early_startup_warn(message: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;

    eprintln!("WARN: {message}");

    let mut wide: Vec<u16> = format!("WARN: {message}").encode_utf16().collect();
    wide.push(0);

    unsafe {
        OutputDebugStringW(PCWSTR(wide.as_ptr()));
    }
}
