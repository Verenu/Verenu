#[cfg(target_os = "windows")]
pub fn system_is_shutting_down() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
    unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
}

#[cfg(not(target_os = "windows"))]
pub fn system_is_shutting_down() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn fallback_platform_never_reports_shutdown() {
        assert!(!super::system_is_shutting_down());
    }
}
